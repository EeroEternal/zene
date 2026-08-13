mod github;
mod llm;

use std::path::Path;

use anyhow::{bail, Context, Result};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use chrono::{Duration, Utc};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;
use uuid::Uuid;
use zene_cloud_domain::{
    ApprovalDecision, ApprovalKind, ApprovalRequest, ApprovalRisk, ApprovalStatus, AuthResponse,
    CloneAuthResponse, CreateApprovalRequest, CreateRepositoryRequest, CreateRunRequest, LoginRequest, Organization,
    PlatformEvent, QueueActive, QueueHold, QueueStats, RegisterRequest, Repository, Run, RunEvent, RunEventKind, RunMessage,
    RunStatus, User, WorkerCommand, WorkerFence,
};

#[derive(Clone)]
pub struct Db {
    pool: SqlitePool,
}

impl Db {
    pub async fn connect(database_url: &str) -> Result<Self> {
        let options = SqliteConnectOptions::from_str(database_url)
            .context("parse sqlite url")?
            .create_if_missing(true);
        if let Some(parent) = options.get_filename().parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let pool = SqlitePoolOptions::new()
            .max_connections(10)
            .connect_with(options)
            .await
            .context("connect sqlite")?;
        sqlx::query("PRAGMA foreign_keys = ON;")
            .execute(&pool)
            .await?;
        Ok(Self { pool })
    }

    /// Apply SQL migrations in filename order, tracking applied versions in
    /// `schema_migrations`. For migrations that add columns, an already-present
    /// column is tolerated so a process interrupted after DDL can safely retry.
    pub async fn migrate(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version TEXT PRIMARY KEY NOT NULL,
                applied_at TEXT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;

        let migrations: &[(&str, &str)] = &[
            ("001_init", include_str!("../../../migrations/001_init.sql")),
            (
                "002_github_git",
                include_str!("../../../migrations/002_github_git.sql"),
            ),
            (
                "003_worker",
                include_str!("../../../migrations/003_worker.sql"),
            ),
            (
                "004_github_settings",
                include_str!("../../../migrations/004_github_settings.sql"),
            ),
            (
                "005_purge_mock_github",
                include_str!("../../../migrations/005_purge_mock_github.sql"),
            ),
            (
                "006_user_llm_settings",
                include_str!("../../../migrations/006_user_llm_settings.sql"),
            ),
            (
                "007_run_archive",
                include_str!("../../../migrations/007_run_archive.sql"),
            ),
            (
                "008_run_max_turns",
                include_str!("../../../migrations/008_run_max_turns.sql"),
            ),
            (
                "009_acp_session",
                include_str!("../../../migrations/009_acp_session.sql"),
            ),
            (
                "010_event_cursor",
                include_str!("../../../migrations/010_event_cursor.sql"),
            ),
            (
                "011_worker_command_leases",
                include_str!("../../../migrations/011_worker_command_leases.sql"),
            ),
        ];

        for (version, sql) in migrations {
            let applied: Option<(String,)> =
                sqlx::query_as("SELECT version FROM schema_migrations WHERE version = ?")
                    .bind(version)
                    .fetch_optional(&self.pool)
                    .await?;
            if applied.is_some() {
                continue;
            }

            let ignore_alter_dupes = version.starts_with("002")
                || version.starts_with("003")
                || version.starts_with("007")
                || version.starts_with("010")
                || version.starts_with("011");
            for statement in split_sql_statements(sql) {
                match sqlx::query(&statement).execute(&self.pool).await {
                    Ok(_) => {}
                    Err(err)
                        if ignore_alter_dupes && is_ignorable_alter_error(&err, &statement) =>
                    {
                        tracing::debug!(
                            version,
                            statement = %truncate_sql(&statement),
                            "ignoring alter failure (column likely exists)"
                        );
                    }
                    Err(err) => {
                        return Err(err).with_context(|| {
                            format!(
                                "migration {version} failed on: {}",
                                truncate_sql(&statement)
                            )
                        });
                    }
                }
            }

            sqlx::query(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?, ?)",
            )
            .bind(version)
            .bind(Utc::now().to_rfc3339())
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    pub async fn ensure_dev_worker_token(&self, raw_token: &str) -> Result<()> {
        let hash = hash_token(raw_token);
        let exists: Option<(String,)> =
            sqlx::query_as("SELECT id FROM worker_tokens WHERE token_hash = ?")
                .bind(&hash)
                .fetch_optional(&self.pool)
                .await?;
        if exists.is_some() {
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO worker_tokens (id, token_hash, name, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(hash)
        .bind("dev-worker")
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn register(&self, req: RegisterRequest) -> Result<AuthResponse> {
        let email = req.email.trim().to_lowercase();
        if email.is_empty() || req.password.len() < 8 {
            bail!("invalid email or password too short");
        }
        let user_id = Uuid::new_v4();
        let org_id = Uuid::new_v4();
        let now = Utc::now();
        let password_hash = hash_password(&req.password)?;
        let slug = slugify(&req.display_name);

        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO users (id, email, display_name, password_hash, created_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(user_id.to_string())
        .bind(&email)
        .bind(&req.display_name)
        .bind(password_hash)
        .bind(now.to_rfc3339())
        .execute(&mut *tx)
        .await
        .context("email may already exist")?;

        sqlx::query(
            "INSERT INTO organizations (id, slug, name, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(org_id.to_string())
        .bind(&slug)
        .bind(format!("{}'s Workspace", req.display_name))
        .bind(now.to_rfc3339())
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO organization_members (organization_id, user_id, role, joined_at)
             VALUES (?, ?, ?, ?)",
        )
        .bind(org_id.to_string())
        .bind(user_id.to_string())
        .bind("owner")
        .bind(now.to_rfc3339())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        let org_name = format!("{}'s Workspace", req.display_name);
        let token = self.create_session(user_id).await?;
        Ok(AuthResponse {
            token,
            user: User {
                id: user_id,
                email,
                display_name: req.display_name,
                created_at: now,
            },
            organization: Organization {
                id: org_id,
                slug,
                name: org_name,
                created_at: now,
            },
        })
    }

    pub async fn login(&self, req: LoginRequest) -> Result<AuthResponse> {
        let email = req.email.trim().to_lowercase();
        let row: Option<(String, String, String, String)> = sqlx::query_as(
            "SELECT id, email, display_name, password_hash FROM users WHERE email = ?",
        )
        .bind(&email)
        .fetch_optional(&self.pool)
        .await?;
        let Some((id, email, display_name, password_hash)) = row else {
            bail!("invalid credentials");
        };
        verify_password(&req.password, &password_hash)?;
        let user_id = Uuid::parse_str(&id)?;
        let org = self.primary_org(user_id).await?;
        let token = self.create_session(user_id).await?;
        Ok(AuthResponse {
            token,
            user: User {
                id: user_id,
                email,
                display_name,
                created_at: Utc::now(),
            },
            organization: org,
        })
    }

    pub async fn user_from_token(&self, token: &str) -> Result<Option<User>> {
        let hash = hash_token(token);
        let row: Option<(String, String, String, String)> = sqlx::query_as(
            "SELECT u.id, u.email, u.display_name, u.created_at
             FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = ? AND s.expires_at > ?",
        )
        .bind(hash)
        .bind(Utc::now().to_rfc3339())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(id, email, display_name, created_at)| User {
            id: Uuid::parse_str(&id).unwrap(),
            email,
            display_name,
            created_at: parse_time(&created_at),
        }))
    }

    pub async fn verify_worker_token(&self, token: &str) -> Result<bool> {
        let hash = hash_token(token);
        let row: Option<(String,)> =
            sqlx::query_as("SELECT id FROM worker_tokens WHERE token_hash = ?")
                .bind(hash)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.is_some())
    }

    pub async fn primary_org(&self, user_id: Uuid) -> Result<Organization> {
        let row: (String, String, String, String) = sqlx::query_as(
            "SELECT o.id, o.slug, o.name, o.created_at
             FROM organizations o
             JOIN organization_members m ON m.organization_id = o.id
             WHERE m.user_id = ?
             ORDER BY m.joined_at ASC LIMIT 1",
        )
        .bind(user_id.to_string())
        .fetch_one(&self.pool)
        .await?;
        Ok(Organization {
            id: Uuid::parse_str(&row.0)?,
            slug: row.1,
            name: row.2,
            created_at: parse_time(&row.3),
        })
    }

    pub async fn create_repository(
        &self,
        org_id: Uuid,
        req: CreateRepositoryRequest,
    ) -> Result<Repository> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let clone_url = req.clone_url.unwrap_or_else(|| {
            format!("https://github.com/{}/{}.git", req.owner, req.name)
        });
        sqlx::query(
            "INSERT INTO repositories
             (id, organization_id, provider, owner, name, default_branch, clone_url, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(org_id.to_string())
        .bind("github")
        .bind(&req.owner)
        .bind(&req.name)
        .bind(&req.default_branch)
        .bind(&clone_url)
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(Repository {
            id,
            organization_id: org_id,
            provider: "github".into(),
            owner: req.owner,
            name: req.name,
            default_branch: req.default_branch,
            clone_url,
            installation_id: None,
            provider_repo_id: None,
            private: false,
            created_at: now,
        })
    }

    pub async fn list_repositories(&self, org_id: Uuid) -> Result<Vec<Repository>> {
        let rows = sqlx::query_as::<_, RepoRow>(
            "SELECT id, organization_id, provider, owner, name, default_branch, clone_url,
                    installation_id, provider_repo_id, COALESCE(private, 0) as private, created_at
             FROM repositories WHERE organization_id = ? ORDER BY created_at DESC",
        )
        .bind(org_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(RepoRow::into_repo).collect())
    }

    pub async fn get_repository(&self, id: Uuid) -> Result<Option<Repository>> {
        let row = sqlx::query_as::<_, RepoRow>(
            "SELECT id, organization_id, provider, owner, name, default_branch, clone_url,
                    installation_id, provider_repo_id, COALESCE(private, 0) as private, created_at
             FROM repositories WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(RepoRow::into_repo))
    }

    pub async fn create_run(
        &self,
        org_id: Uuid,
        user_id: Uuid,
        req: CreateRunRequest,
    ) -> Result<Run> {
        let repo = self
            .get_repository(req.repository_id)
            .await?
            .context("repository not found")?;
        if repo.organization_id != org_id {
            bail!("repository not in organization");
        }
        let id = Uuid::new_v4();
        let now = Utc::now();
        let title = title_from_prompt(&req.prompt);
        let head_branch = format!(
            "zene/{}/{}",
            slugify(&title).chars().take(24).collect::<String>(),
            &id.to_string()[..8]
        );
        let base_ref = req
            .base_ref
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| repo.default_branch.clone());
        let run = Run {
            id,
            organization_id: org_id,
            repository_id: repo.id,
            requested_by: user_id,
            status: RunStatus::Queued,
            status_version: 1,
            title,
            prompt: req.prompt.clone(),
            base_ref,
            base_sha: None,
            head_branch,
            head_sha: None,
            model: req.model,
            permission_mode: req.permission_mode,
            max_turns: req.max_turns,
            created_at: now,
            started_at: None,
            finished_at: None,
            archived_at: None,
        };
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO runs
             (id, organization_id, repository_id, requested_by, status, status_version, title,
              prompt, base_ref, base_sha, head_branch, head_sha, model, permission_mode, max_turns,
              created_at, started_at, finished_at, archived_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(run.id.to_string())
        .bind(run.organization_id.to_string())
        .bind(run.repository_id.to_string())
        .bind(run.requested_by.to_string())
        .bind(run.status.as_str())
        .bind(run.status_version)
        .bind(&run.title)
        .bind(&run.prompt)
        .bind(&run.base_ref)
        .bind(&run.base_sha)
        .bind(&run.head_branch)
        .bind(&run.head_sha)
        .bind(&run.model)
        .bind(&run.permission_mode)
        .bind(run.max_turns as i64)
        .bind(run.created_at.to_rfc3339())
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .execute(&mut *tx)
        .await?;

        let msg_id = Uuid::new_v4();
        // Initial prompt is delivered via ClaimedRun.prompt; mark it seen for the inbox.
        sqlx::query(
            "INSERT INTO run_messages
             (id, run_id, author_id, role, content, created_at, delivered_to_worker)
             VALUES (?, ?, ?, ?, ?, ?, 1)",
        )
        .bind(msg_id.to_string())
        .bind(run.id.to_string())
        .bind(user_id.to_string())
        .bind("user")
        .bind(&req.prompt)
        .bind(now.to_rfc3339())
        .execute(&mut *tx)
        .await?;

        self.append_event_tx(
            &mut tx,
            run.id,
            0,
            Some("platform.run.created"),
            None,
            RunEventKind::Platform,
            platform_payload(PlatformEvent::RunCreated {
                title: run.title.clone(),
                prompt: run.prompt.clone(),
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(run)
    }

    pub async fn list_runs(&self, org_id: Uuid) -> Result<Vec<Run>> {
        let sql = format!(
            "SELECT {RUN_COLUMNS} FROM runs
             WHERE organization_id = ? AND archived_at IS NULL
             ORDER BY created_at DESC LIMIT 100"
        );
        let rows = sqlx::query_as::<_, RunRow>(&sql)
            .bind(org_id.to_string())
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(RunRow::into_run).collect())
    }

    pub async fn get_run(&self, run_id: Uuid) -> Result<Option<Run>> {
        let sql = format!("SELECT {RUN_COLUMNS} FROM runs WHERE id = ?");
        let row = sqlx::query_as::<_, RunRow>(&sql)
            .bind(run_id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(RunRow::into_run))
    }

    pub async fn update_run_meta(
        &self,
        run_id: Uuid,
        title: Option<&str>,
        archived: Option<bool>,
    ) -> Result<Run> {
        self.update_run_meta_inner(run_id, title, archived, None, false)
            .await
    }

    /// Update worker-owned metadata only when the supplied attempt is still
    /// active. The transaction validates the fence before changing the title,
    /// so a reclaimed worker cannot overwrite a replacement worker's title.
    pub async fn update_run_title_fenced(
        &self,
        run_id: Uuid,
        fence: &WorkerFence,
        title: &str,
    ) -> Result<Run> {
        self.update_run_meta_inner(run_id, Some(title), None, Some(fence), false)
            .await
    }

    /// Compatibility path for old workers that do not send a fence. It is
    /// accepted only while no attempt is active, preventing a stale worker
    /// from mutating a run after it has been reclaimed.
    pub async fn update_run_title_legacy(&self, run_id: Uuid, title: &str) -> Result<Run> {
        self.update_run_meta_inner(run_id, Some(title), None, None, true)
            .await
    }

    async fn update_run_meta_inner(
        &self,
        run_id: Uuid,
        title: Option<&str>,
        archived: Option<bool>,
        fence: Option<&WorkerFence>,
        legacy_worker: bool,
    ) -> Result<Run> {
        let mut tx = self.pool.begin().await?;
        if let Some(fence) = fence {
            self.validate_worker_fence(&mut tx, run_id, fence).await?;
        } else if legacy_worker {
            let active: Option<(String,)> = sqlx::query_as(
                "SELECT id FROM run_attempts WHERE run_id = ? AND finished_at IS NULL LIMIT 1",
            )
            .bind(run_id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
            if active.is_some() {
                bail!("stale_attempt: worker fence required for active run");
            }
        }
        let sql = format!("SELECT {RUN_COLUMNS} FROM runs WHERE id = ?");
        sqlx::query_as::<_, RunRow>(&sql)
            .bind(run_id.to_string())
            .fetch_optional(&mut *tx)
            .await?
            .map(RunRow::into_run)
            .context("run not found")?;

        if let Some(title) = title {
            let title = title.trim();
            if !title.is_empty() {
                sqlx::query("UPDATE runs SET title = ? WHERE id = ?")
                    .bind(title.chars().take(80).collect::<String>())
                    .bind(run_id.to_string())
                    .execute(&mut *tx)
                    .await?;
            }
        }
        if let Some(archived) = archived {
            if archived {
                sqlx::query("UPDATE runs SET archived_at = ? WHERE id = ?")
                    .bind(Utc::now().to_rfc3339())
                    .bind(run_id.to_string())
                    .execute(&mut *tx)
                    .await?;
            } else {
                sqlx::query("UPDATE runs SET archived_at = NULL WHERE id = ?")
                    .bind(run_id.to_string())
                    .execute(&mut *tx)
                    .await?;
            }
        }

        let updated = sqlx::query_as::<_, RunRow>(&sql)
            .bind(run_id.to_string())
            .fetch_one(&mut *tx)
            .await?
            .into_run();
        if title.is_some() {
            let title_event_id = fence
                .map(|fence| format!("platform.run.title.worker.{}", fence.generation));
            self.append_event_tx(
                &mut tx,
                run_id,
                fence.map_or(0, |fence| fence.generation),
                title_event_id.as_deref().or(Some("platform.run.title")), 
                None,
                RunEventKind::Platform,
                platform_payload(PlatformEvent::RunTitle {
                    title: updated.title.clone(),
                }),
            )
            .await?;
        }
        if archived == Some(true) {
            self.append_event_tx(
                &mut tx,
                run_id,
                0,
                Some("platform.run.archived"),
                None,
                RunEventKind::Platform,
                platform_payload(PlatformEvent::RunArchived),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(updated)
    }

    pub async fn delete_run(&self, run_id: Uuid) -> Result<()> {
        let run_id_s = run_id.to_string();
        let mut tx = self.pool.begin().await?;
        for sql in [
            "DELETE FROM run_events WHERE run_id = ?",
            "DELETE FROM run_messages WHERE run_id = ?",
            "DELETE FROM run_attempts WHERE run_id = ?",
            "DELETE FROM approval_requests WHERE run_id = ?",
            "DELETE FROM run_clone_credentials WHERE run_id = ?",
            "DELETE FROM pull_requests WHERE run_id = ?",
            "DELETE FROM git_operations WHERE run_id = ?",
            "DELETE FROM runs WHERE id = ?",
        ] {
            sqlx::query(sql)
                .bind(&run_id_s)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn reclaim_stale_runs(&self) -> Result<u64> {
        self.reclaim_stale_runs_at(Utc::now()).await
    }

    pub async fn reclaim_stale_runs_at(&self, now: chrono::DateTime<Utc>) -> Result<u64> {
        let now = now.to_rfc3339();
        let mut tx = self.pool.begin().await?;
        // Explicit hold policy: a stale waiting_for_user session can be
        // safely replaced and its unacked inbox commands retried. Approval
        // holds are excluded because their lifecycle has separate semantics.
        let stale = sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT a.run_id FROM run_attempts a
             JOIN runs r ON r.id = a.run_id
             WHERE a.finished_at IS NULL
               AND a.lease_expires_at IS NOT NULL
               AND a.lease_expires_at < ?
               AND r.status IN ('provisioning','starting','cloning','running','waiting_for_user')",
        )
        .bind(&now)
        .fetch_all(&mut *tx)
        .await?;
        if stale.is_empty() {
            tx.commit().await?;
            return Ok(0);
        }
        for run_id in &stale {
            sqlx::query(
                "UPDATE run_attempts
                 SET status = 'expired', finished_at = ?, failure_code = 'lease_expired'
                 WHERE run_id = ? AND finished_at IS NULL",
            )
            .bind(&now)
            .bind(run_id)
            .execute(&mut *tx)
            .await?;
            // A replacement worker should not wait for the old command lease
            // after the attempt itself has been declared stale.
            sqlx::query(
                "UPDATE run_messages
                 SET delivery_claimed_at = NULL, delivery_claim_attempt_id = NULL
                 WHERE run_id = ? AND COALESCE(delivered_to_worker, 0) = 0",
            )
            .bind(run_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE runs
                 SET status = 'queued', status_version = status_version + 1,
                     finished_at = NULL, last_error = 'worker lease expired; re-queued'
                 WHERE id = ?
                   AND status IN ('provisioning','starting','cloning','running','waiting_for_user')",
            )
            .bind(run_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(stale.len() as u64)
    }

    pub async fn claim_next_run(
        &self,
        worker_id: &str,
        workspace_root: &Path,
    ) -> Result<Option<(Run, Uuid, i64, Option<String>, String)>> {
        // Recover runs left mid-flight when a worker dies (common during long git clones).
        let _ = self.reclaim_stale_runs().await;

        let mut tx = self.pool.begin().await?;
        let sql = format!(
            "SELECT {RUN_COLUMNS} FROM runs
             WHERE status = 'queued' AND archived_at IS NULL
             ORDER BY created_at ASC LIMIT 1"
        );
        let row = sqlx::query_as::<_, RunRow>(&sql)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let run = row.into_run();
        let attempt_id = Uuid::new_v4();
        let attempt = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(MAX(attempt), 0) + 1 FROM run_attempts WHERE run_id = ?",
        )
        .bind(run.id.to_string())
        .fetch_one(&mut *tx)
        .await?;
        let resume_session_id = sqlx::query_scalar::<_, Option<String>>(
            "SELECT acp_session_id FROM run_attempts
             WHERE run_id = ? AND acp_session_id IS NOT NULL
             ORDER BY attempt DESC LIMIT 1",
        )
        .bind(run.id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        .flatten();
        let generation = attempt;
        let now = Utc::now();
        let lease = now + Duration::seconds(60);
        sqlx::query(
            "UPDATE runs SET status = ?, status_version = status_version + 1, started_at = COALESCE(started_at, ?)
             WHERE id = ? AND status = 'queued'",
        )
        .bind(RunStatus::Provisioning.as_str())
        .bind(now.to_rfc3339())
        .bind(run.id.to_string())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO run_attempts
             (id, run_id, attempt, generation, worker_id, status, lease_expires_at, started_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(attempt_id.to_string())
        .bind(run.id.to_string())
        .bind(attempt)
        .bind(generation)
        .bind(worker_id)
        .bind("running")
        .bind(lease.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        let workspace_dir = workspace_root
            .join(run.id.to_string())
            .to_string_lossy()
            .to_string();
        let mut run = self.get_run(run.id).await?.context("run missing after claim")?;
        run.status = RunStatus::Provisioning;
        Ok(Some((run, attempt_id, generation, resume_session_id, workspace_dir)))
    }

    pub async fn set_runtime_session_id_fenced(
        &self,
        run_id: Uuid,
        fence: &WorkerFence,
        session_id: &str,
    ) -> Result<()> {
        self.validate_worker_fence_simple(run_id, fence).await?;
        let result = sqlx::query(
            "UPDATE run_attempts SET acp_session_id = ?
             WHERE id = ? AND run_id = ? AND generation = ? AND worker_id = ? AND finished_at IS NULL",
        )
        .bind(session_id)
        .bind(fence.attempt_id.to_string())
        .bind(run_id.to_string())
        .bind(fence.generation)
        .bind(&fence.worker_id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            bail!("stale_attempt: worker fence rejected runtime session update")
        }
        Ok(())
    }

    pub async fn update_run_status_fenced(
        &self,
        run_id: Uuid,
        fence: &WorkerFence,
        status: RunStatus,
        head_sha: Option<String>,
        failure_code: Option<String>,
    ) -> Result<Run> {
        let mut tx = self.pool.begin().await?;
        self.validate_worker_fence(&mut tx, run_id, fence).await?;
        let now = Utc::now();
        let finished = if status.is_terminal() || matches!(status, RunStatus::Completed) {
            Some(now.to_rfc3339())
        } else {
            None
        };
        sqlx::query(
            "UPDATE runs
             SET status = ?, status_version = status_version + 1, head_sha = COALESCE(?, head_sha),
                 finished_at = COALESCE(?, finished_at)
             WHERE id = ?",
        )
        .bind(status.as_str())
        .bind(head_sha.clone())
        .bind(finished.clone())
        .bind(run_id.to_string())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE run_attempts SET status = ?, failure_code = COALESCE(?, failure_code), finished_at = COALESCE(?, finished_at)
             WHERE id = ? AND run_id = ? AND finished_at IS NULL",
        )
        .bind(status.as_str())
        .bind(failure_code)
        .bind(finished)
        .bind(fence.attempt_id.to_string())
        .bind(run_id.to_string())
        .execute(&mut *tx)
        .await?;
        self.append_event_tx(
            &mut tx,
            run_id,
            fence.generation,
            Some(&format!("platform.status.{}", status.as_str())),
            None,
            RunEventKind::Platform,
            run_status_payload(status, head_sha),
        )
        .await?;
        tx.commit().await?;
        self.get_run(run_id)
            .await?
            .context("run missing after status update")
    }

    pub async fn update_run_status(
        &self,
        run_id: Uuid,
        status: RunStatus,
        head_sha: Option<String>,
        failure_code: Option<String>,
    ) -> Result<Run> {
        let mut tx = self.pool.begin().await?;
        let now = Utc::now();
        let finished = if status.is_terminal() || matches!(status, RunStatus::Completed) {
            Some(now.to_rfc3339())
        } else {
            None
        };
        sqlx::query(
            "UPDATE runs
             SET status = ?, status_version = status_version + 1, head_sha = COALESCE(?, head_sha),
                 finished_at = COALESCE(?, finished_at)
             WHERE id = ?",
        )
        .bind(status.as_str())
        .bind(head_sha.clone())
        .bind(finished)
        .bind(run_id.to_string())
        .execute(&mut *tx)
        .await?;
        if let Some(code) = failure_code {
            sqlx::query(
                "UPDATE run_attempts SET failure_code = ?, finished_at = ?
                 WHERE run_id = ? AND finished_at IS NULL",
            )
            .bind(code)
            .bind(now.to_rfc3339())
            .bind(run_id.to_string())
            .execute(&mut *tx)
            .await?;
        }
        self.append_event_tx(
            &mut tx,
            run_id,
            0,
            Some(&format!("platform.status.{}", status.as_str())),
            None,
            RunEventKind::Platform,
            run_status_payload(status, head_sha),
        )
        .await?;
        let updated = sqlx::query_as::<_, RunRow>(&format!("SELECT {RUN_COLUMNS} FROM runs WHERE id = ?"))
            .bind(run_id.to_string())
            .fetch_optional(&mut *tx)
            .await?
            .map(RunRow::into_run)
            .context("run missing after status update")?;
        tx.commit().await?;
        Ok(updated)
    }

    pub async fn heartbeat_fenced(&self, run_id: Uuid, fence: &WorkerFence) -> Result<()> {
        let lease = Utc::now() + Duration::seconds(60);
        let result = sqlx::query(
            "UPDATE run_attempts SET lease_expires_at = ?
             WHERE id = ? AND run_id = ? AND generation = ? AND worker_id = ? AND finished_at IS NULL",
        )
        .bind(lease.to_rfc3339())
        .bind(fence.attempt_id.to_string())
        .bind(run_id.to_string())
        .bind(fence.generation)
        .bind(&fence.worker_id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            bail!("stale_attempt: worker fence rejected heartbeat")
        }
        Ok(())
    }

    pub async fn heartbeat(&self, run_id: Uuid, worker_id: &str) -> Result<()> {
        let lease = Utc::now() + Duration::seconds(60);
        sqlx::query(
            "UPDATE run_attempts SET lease_expires_at = ?
             WHERE run_id = ? AND worker_id = ? AND finished_at IS NULL",
        )
        .bind(lease.to_rfc3339())
        .bind(run_id.to_string())
        .bind(worker_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Aggregate queue depth for the worker supervisor (single-node pool).
    pub async fn queue_stats(&self) -> Result<QueueStats> {
        let queued = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM runs
             WHERE status = 'queued' AND archived_at IS NULL",
        )
        .fetch_one(&self.pool)
        .await? as u64;

        let active_rows = sqlx::query_as::<_, (String, String, String)>(
            "SELECT a.worker_id, r.id, r.status
             FROM runs r
             JOIN run_attempts a ON a.run_id = r.id AND a.finished_at IS NULL
             WHERE r.archived_at IS NULL
               AND r.status IN ('provisioning','starting','cloning','running')
             ORDER BY a.started_at ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        let hold_rows = sqlx::query_as::<_, (String, String, String)>(
            "SELECT a.worker_id, r.id, COALESCE(a.started_at, r.started_at, r.created_at)
             FROM runs r
             JOIN run_attempts a ON a.run_id = r.id AND a.finished_at IS NULL
             WHERE r.archived_at IS NULL
               AND r.status IN ('waiting_for_user','waiting_for_approval')
             ORDER BY a.started_at ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        let actives = active_rows
            .into_iter()
            .filter_map(|(worker_id, run_id, status)| {
                let run_id = Uuid::parse_str(&run_id).ok()?;
                Some(QueueActive {
                    worker_id,
                    run_id,
                    status,
                })
            })
            .collect::<Vec<_>>();

        let holds = hold_rows
            .into_iter()
            .filter_map(|(worker_id, run_id, since)| {
                let run_id = Uuid::parse_str(&run_id).ok()?;
                let since = chrono::DateTime::parse_from_rfc3339(&since)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
                    .or_else(|| {
                        // SQLite may store without offset; treat as UTC.
                        chrono::NaiveDateTime::parse_from_str(&since, "%Y-%m-%d %H:%M:%S%.f")
                            .or_else(|_| {
                                chrono::NaiveDateTime::parse_from_str(&since, "%Y-%m-%dT%H:%M:%S%.f")
                            })
                            .ok()
                            .map(|ndt| ndt.and_utc())
                    })
                    .unwrap_or_else(Utc::now);
                Some(QueueHold {
                    worker_id,
                    run_id,
                    since,
                })
            })
            .collect::<Vec<_>>();

        Ok(QueueStats {
            queued,
            active: actives.len() as u64,
            holding: holds.len() as u64,
            holds,
            actives,
        })
    }

    pub async fn append_event_fenced(
        &self,
        run_id: Uuid,
        fence: &WorkerFence,
        source_event_id: Option<&str>,
        event_type: RunEventKind,
        payload: serde_json::Value,
    ) -> Result<RunEvent> {
        let mut tx = self.pool.begin().await?;
        self.validate_worker_fence(&mut tx, run_id, fence).await?;
        let event = self
            .append_event_tx(&mut tx, run_id, fence.generation, source_event_id, None, event_type, payload)
            .await?;
        tx.commit().await?;
        Ok(event)
    }

    pub async fn append_event(
        &self,
        run_id: Uuid,
        attempt_generation: i64,
        source_event_id: Option<&str>,
        event_type: RunEventKind,
        payload: serde_json::Value,
    ) -> Result<RunEvent> {
        let mut tx = self.pool.begin().await?;
        let event = self
            .append_event_tx(&mut tx, run_id, attempt_generation, source_event_id, None, event_type, payload)
            .await?;
        tx.commit().await?;
        Ok(event)
    }

    pub async fn append_event_fenced_with_cursor(
        &self,
        run_id: Uuid,
        fence: &WorkerFence,
        source_event_id: Option<&str>,
        cursor: Option<u64>,
        event_type: RunEventKind,
        payload: serde_json::Value,
    ) -> Result<RunEvent> {
        let mut tx = self.pool.begin().await?;
        self.validate_worker_fence(&mut tx, run_id, fence).await?;
        let event = self
            .append_event_tx(&mut tx, run_id, fence.generation, source_event_id, cursor, event_type, payload)
            .await?;
        tx.commit().await?;
        Ok(event)
    }

    async fn append_event_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        run_id: Uuid,
        attempt_generation: i64,
        source_event_id: Option<&str>,
        cursor: Option<u64>,
        event_type: RunEventKind,
        payload: serde_json::Value,
    ) -> Result<RunEvent> {
        let next: (i64,) =
            sqlx::query_as("SELECT COALESCE(MAX(seq), 0) + 1 FROM run_events WHERE run_id = ?")
                .bind(run_id.to_string())
                .fetch_one(&mut **tx)
                .await?;
        let now = Utc::now();
        if let Some(source_id) = source_event_id {
            let existing: Option<(i64, Option<i64>, String, String, String)> = sqlx::query_as(
                "SELECT seq, cursor, event_type, payload_json, created_at FROM run_events
                 WHERE run_id = ? AND source_event_id = ?",
            )
            .bind(run_id.to_string())
            .bind(source_id)
            .fetch_optional(&mut **tx)
            .await?;
            if let Some((seq, stored_cursor, event_type, payload_json, created_at)) = existing {
                return Ok(RunEvent {
                    run_id,
                    seq,
                    cursor: stored_cursor.and_then(|value| u64::try_from(value).ok()),
                    event_type,
                    payload: serde_json::from_str(&payload_json).unwrap_or(payload),
                    created_at: parse_time(&created_at),
                });
            }
        }

        sqlx::query(
            "INSERT INTO run_events
             (run_id, seq, attempt_generation, source_event_id, cursor, event_type, payload_json, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(run_id.to_string())
        .bind(next.0)
        .bind(attempt_generation)
        .bind(source_event_id)
        .bind(cursor.map(|value| value as i64))
        .bind(event_type.as_event_type())
        .bind(payload.to_string())
        .bind(now.to_rfc3339())
        .execute(&mut **tx)
        .await?;
        Ok(RunEvent {
            run_id,
            seq: next.0,
            cursor,
            event_type: event_type.as_event_type().to_string(),
            payload,
            created_at: now,
        })
    }

    async fn validate_worker_fence_simple(
        &self,
        run_id: Uuid,
        fence: &WorkerFence,
    ) -> Result<()> {
        let row: Option<(i64, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT generation, worker_id, finished_at FROM run_attempts
             WHERE id = ? AND run_id = ?",
        )
        .bind(fence.attempt_id.to_string())
        .bind(run_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        match row {
            Some((generation, Some(worker_id), None))
                if generation == fence.generation && worker_id == fence.worker_id => Ok(()),
            _ => bail!("stale_attempt: worker fence rejected runtime session update"),
        }
    }

    async fn validate_worker_fence(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        run_id: Uuid,
        fence: &WorkerFence,
    ) -> Result<()> {
        let current: Option<(i64, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT generation, worker_id, finished_at FROM run_attempts
             WHERE id = ? AND run_id = ?",
        )
        .bind(fence.attempt_id.to_string())
        .bind(run_id.to_string())
        .fetch_optional(&mut **tx)
        .await?;
        let valid = current
            .as_ref()
            .is_some_and(|(generation, worker_id, finished_at)| {
                *generation == fence.generation
                    && worker_id.as_deref() == Some(fence.worker_id.as_str())
                    && finished_at.is_none()
            });
        if !valid {
            bail!("stale_attempt: worker fence rejected operation")
        }
        Ok(())
    }

    pub async fn events_after(&self, run_id: Uuid, after_seq: i64) -> Result<Vec<RunEvent>> {
        let rows: Vec<(i64, Option<i64>, String, String, String)> = sqlx::query_as(
            "SELECT seq, cursor, event_type, payload_json, created_at
             FROM run_events WHERE run_id = ? AND seq > ? ORDER BY seq ASC LIMIT 500",
        )
        .bind(run_id.to_string())
        .bind(after_seq)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(seq, cursor, event_type, payload_json, created_at)| RunEvent {
                run_id,
                seq,
                cursor: cursor.and_then(|value| u64::try_from(value).ok()),
                event_type,
                payload: serde_json::from_str(&payload_json).unwrap_or(serde_json::json!({})),
                created_at: parse_time(&created_at),
            })
            .collect())
    }

    /// Read the complete canonical event stream after a provider cursor.
    ///
    /// Provider cursors are only present on some runtime events, so replay must
    /// retain every event and use `seq` for ordering. The cursor is translated
    /// to the greatest canonical sequence observed at or before that provider
    /// position; callers therefore cannot lose platform events without a cursor.
    pub async fn events_after_cursor(&self, run_id: Uuid, after_cursor: u64) -> Result<Vec<RunEvent>> {
        let after_seq: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq), 0) FROM run_events
             WHERE run_id = ? AND cursor IS NOT NULL AND cursor <= ?",
        )
        .bind(run_id.to_string())
        .bind(after_cursor as i64)
        .fetch_one(&self.pool)
        .await?;
        self.events_after(run_id, after_seq).await
    }

    pub async fn add_message(
        &self,
        run_id: Uuid,
        author_id: Option<Uuid>,
        role: &str,
        content: &str,
        client_message_id: Option<&str>,
    ) -> Result<RunMessage> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO run_messages
             (id, run_id, author_id, role, content, client_message_id, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(run_id.to_string())
        .bind(author_id.map(|v| v.to_string()))
        .bind(role)
        .bind(content)
        .bind(client_message_id)
        .bind(now.to_rfc3339())
        .execute(&mut *tx)
        .await?;
        self.append_event_tx(
            &mut tx,
            run_id,
            0,
            client_message_id,
            None,
            RunEventKind::Platform,
            platform_payload(PlatformEvent::MessageCreated {
                role: role.to_string(),
                text: content.to_string(),
            }),
        )
        .await?;
        let current_status: Option<String> = sqlx::query_scalar(
            "SELECT status FROM runs WHERE id = ?",
        )
        .bind(run_id.to_string())
        .fetch_optional(&mut *tx)
        .await?;
        if current_status.as_deref() == Some(RunStatus::Completed.as_str()) {
            // Re-queue with the follow-up as the next claim prompt.
            sqlx::query("UPDATE runs SET prompt = ? WHERE id = ?")
                .bind(content)
                .bind(run_id.to_string())
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE run_messages SET delivered_to_worker = 1 WHERE id = ?")
                .bind(id.to_string())
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "UPDATE runs
                 SET status = ?, status_version = status_version + 1
                 WHERE id = ?",
            )
            .bind(RunStatus::Queued.as_str())
            .bind(run_id.to_string())
            .execute(&mut *tx)
            .await?;
            self.append_event_tx(
                &mut tx,
                run_id,
                0,
                Some("platform.status.queued"),
                None,
                RunEventKind::Platform,
                run_status_payload(RunStatus::Queued, None),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(RunMessage {
            id,
            run_id,
            author_id,
            role: role.into(),
            content: content.into(),
            created_at: now,
        })
    }

    pub async fn list_messages(&self, run_id: Uuid) -> Result<Vec<RunMessage>> {
        let rows: Vec<(String, String, Option<String>, String, String, String)> = sqlx::query_as(
            "SELECT id, run_id, author_id, role, content, created_at
             FROM run_messages WHERE run_id = ? ORDER BY created_at ASC",
        )
        .bind(run_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, run_id, author_id, role, content, created_at)| RunMessage {
                id: Uuid::parse_str(&id).unwrap(),
                run_id: Uuid::parse_str(&run_id).unwrap(),
                author_id: author_id.and_then(|v| Uuid::parse_str(&v).ok()),
                role,
                content,
                created_at: parse_time(&created_at),
            })
            .collect())
    }

    pub async fn get_clone_auth(&self, run_id: Uuid) -> Result<CloneAuthResponse> {
        let run = self
            .get_run(run_id)
            .await?
            .context("run not found")?;
        let repo = self
            .get_repository(run.repository_id)
            .await?
            .context("repository not found")?;

        // Prefer cached credentials when present.
        let cached: Option<(String, Option<String>, Option<String>, i64)> = sqlx::query_as(
            "SELECT clone_url, token_enc, username, mock FROM run_clone_credentials WHERE run_id = ?",
        )
        .bind(run_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        if let Some((clone_url, token, username, mock)) = cached {
            return Ok(CloneAuthResponse {
                run_id,
                repository_id: repo.id,
                clone_url,
                username,
                token,
                base_ref: run.base_ref,
                head_branch: run.head_branch,
                mock: mock != 0,
            });
        }

        // Phase 0: no GitHub App tokens yet — treat as mock workspace unless a real
        // clone URL looks locally usable (file:// or existing path).
        let mock = !repo.clone_url.starts_with("file://")
            && !Path::new(&repo.clone_url).exists();
        let now = Utc::now();
        sqlx::query(
            "INSERT OR REPLACE INTO run_clone_credentials
             (run_id, clone_url, token_enc, username, mock, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(run_id.to_string())
        .bind(&repo.clone_url)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(if mock { 1 } else { 0 })
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(CloneAuthResponse {
            run_id,
            repository_id: repo.id,
            clone_url: repo.clone_url,
            username: None,
            token: None,
            base_ref: run.base_ref,
            head_branch: run.head_branch,
            mock,
        })
    }

    pub async fn poll_worker_commands_fenced(
        &self,
        run_id: Uuid,
        fence: &WorkerFence,
    ) -> Result<Vec<WorkerCommand>> {
        self.poll_worker_commands_fenced_at(run_id, fence, Utc::now()).await
    }

    pub async fn poll_worker_commands_fenced_at(
        &self,
        run_id: Uuid,
        fence: &WorkerFence,
        now: chrono::DateTime<Utc>,
    ) -> Result<Vec<WorkerCommand>> {
        // A command is visible until the worker acknowledges successful runtime
        // delivery. Claims expire so a crashed worker cannot lose a follow-up.
        const CLAIM_LEASE: i64 = 60;
        let stale_before = (now - Duration::seconds(CLAIM_LEASE)).to_rfc3339();
        let run = self
            .get_run(run_id)
            .await?
            .context("run not found")?;
        let mut tx = self.pool.begin().await?;
        self.validate_worker_fence(&mut tx, run_id, fence).await?;
        let mut commands = Vec::new();

        if matches!(
            run.status,
            RunStatus::Cancelled | RunStatus::Stopping | RunStatus::TimedOut
        ) {
            commands.push(WorkerCommand::cancel(format!("cancel-{}", run.status.as_str())));
        }

        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT id, content FROM run_messages
             WHERE run_id = ? AND role = 'user' AND COALESCE(delivered_to_worker, 0) = 0
               AND (delivery_claimed_at IS NULL OR delivery_claimed_at < ?)
             ORDER BY created_at ASC LIMIT 20",
        )
        .bind(run_id.to_string())
        .bind(&stale_before)
        .fetch_all(&mut *tx)
        .await?;

        for (id, content) in rows {
            let claimed = sqlx::query(
                "UPDATE run_messages
                 SET delivery_attempt = COALESCE(delivery_attempt, 0) + 1,
                     delivery_claimed_at = ?, delivery_claim_attempt_id = ?
                 WHERE id = ? AND run_id = ? AND role = 'user'
                   AND COALESCE(delivered_to_worker, 0) = 0
                   AND (delivery_claimed_at IS NULL OR delivery_claimed_at < ?)",
            )
            .bind(now.to_rfc3339())
            .bind(fence.attempt_id.to_string())
            .bind(&id)
            .bind(run_id.to_string())
            .bind(&stale_before)
            .execute(&mut *tx)
            .await?;
            if claimed.rows_affected() != 1 {
                continue;
            }
            let message_id = Uuid::parse_str(&id)?;
            commands.push(WorkerCommand::prompt(format!("msg-{id}"), content, message_id));
        }
        tx.commit().await?;
        Ok(commands)
    }

    pub async fn ack_worker_command_fenced(
        &self,
        run_id: Uuid,
        fence: &WorkerFence,
        message_id: Uuid,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        self.validate_worker_fence(&mut tx, run_id, fence).await?;
        let result = sqlx::query(
            "UPDATE run_messages
             SET delivered_to_worker = 1,
                 delivery_claimed_at = NULL,
                 delivery_claim_attempt_id = NULL
             WHERE id = ? AND run_id = ? AND role = 'user'
               AND COALESCE(delivered_to_worker, 0) = 0
               AND delivery_claim_attempt_id = ?",
        )
        .bind(message_id.to_string())
        .bind(run_id.to_string())
        .bind(fence.attempt_id.to_string())
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() == 0 {
            let already_acked: Option<(i64,)> = sqlx::query_as(
                "SELECT delivered_to_worker FROM run_messages WHERE id = ? AND run_id = ?",
            )
            .bind(message_id.to_string())
            .bind(run_id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
            if !matches!(already_acked, Some((1,))) {
                bail!("command_not_claimed: worker command acknowledgement rejected")
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// Legacy, unfenced command read. Keep it read-only so old callers cannot
    /// permanently consume a command before runtime delivery succeeds. New
    /// workers must use `poll_worker_commands_fenced` followed by ack.
    #[allow(dead_code)]
    pub async fn poll_worker_commands(&self, run_id: Uuid) -> Result<Vec<WorkerCommand>> {
        let run = self
            .get_run(run_id)
            .await?
            .context("run not found")?;
        let mut commands = Vec::new();

        if matches!(
            run.status,
            RunStatus::Cancelled | RunStatus::Stopping | RunStatus::TimedOut
        ) {
            commands.push(WorkerCommand::cancel(format!("cancel-{}", run.status.as_str())));
        }

        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT id, content FROM run_messages
             WHERE run_id = ? AND role = 'user' AND COALESCE(delivered_to_worker, 0) = 0
             ORDER BY created_at ASC LIMIT 20",
        )
        .bind(run_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        for (id, content) in rows {
            let message_id = Uuid::parse_str(&id)?;
            commands.push(WorkerCommand::prompt(format!("msg-{id}"), content, message_id));
        }

        Ok(commands)
    }

    pub async fn create_approval(
        &self,
        run_id: Uuid,
        req: CreateApprovalRequest,
    ) -> Result<ApprovalRequest> {
        let run = self
            .get_run(run_id)
            .await?
            .context("run not found")?;

        let id = Uuid::new_v4();
        let now = Utc::now();
        let auto = matches!(
            run.permission_mode.as_str(),
            "yolo" | "auto" | "default"
        );
        let status = if auto {
            ApprovalStatus::Resolved
        } else {
            ApprovalStatus::Pending
        };
        let decision = if auto {
            Some(ApprovalDecision::AllowOnce)
        } else {
            None
        };
        let resolved_at = if auto {
            Some(now.to_rfc3339())
        } else {
            None
        };
        let allowed = serde_json::to_string(&req.allowed_decisions)?;
        let payload = req.payload.to_string();

        let inserted = sqlx::query(
            "INSERT INTO approval_requests
             (id, run_id, request_key, kind, risk, payload_json, status,
              allowed_decisions, created_at, expires_at, resolved_by, resolved_at, decision)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(run_id, request_key) DO NOTHING",
        )
        .bind(id.to_string())
        .bind(run_id.to_string())
        .bind(&req.request_key)
        .bind(req.kind.as_str())
        .bind(req.risk.as_str())
        .bind(&payload)
        .bind(status.as_str())
        .bind(&allowed)
        .bind(now.to_rfc3339())
        .bind(req.expires_at.map(|t| t.to_rfc3339()))
        .bind(if auto { Some("system") } else { None })
        .bind(resolved_at)
        .bind(decision.map(|d| d.as_str()))
        .execute(&self.pool)
        .await?;

        // The unique constraint is the authority under concurrent creates. Only
        // the caller that inserted the row may emit status/event side effects.
        if inserted.rows_affected() == 0 {
            return self
                .get_approval_by_key(run_id, &req.request_key)
                .await?
                .context("approval missing after conflicting create");
        }

        if !auto {
            self.update_run_status(run_id, RunStatus::WaitingForApproval, None, None)
                .await?;
        }

        self.append_event(
            run_id,
            0,
            Some(&format!("approval.{}", req.request_key)),
            RunEventKind::Platform,
            platform_payload(PlatformEvent::ApprovalCreated {
                approval_id: id,
                status,
                decision,
                kind: req.kind,
            }),
        )
        .await?;

        self.get_approval(id)
            .await?
            .context("approval missing after create")
    }

    pub async fn get_approval(&self, approval_id: Uuid) -> Result<Option<ApprovalRequest>> {
        let row: Option<(
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT id, run_id, request_key, kind, risk, payload_json, status,
                    allowed_decisions, decision, created_at, expires_at, resolved_by, resolved_at
             FROM approval_requests WHERE id = ?",
        )
        .bind(approval_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(map_approval_full_row))
    }

    pub async fn get_approval_by_key(
        &self,
        run_id: Uuid,
        request_key: &str,
    ) -> Result<Option<ApprovalRequest>> {
        let row: Option<(
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT id, run_id, request_key, kind, risk, payload_json, status,
                    allowed_decisions, decision, created_at, expires_at, resolved_by, resolved_at
             FROM approval_requests WHERE run_id = ? AND request_key = ?",
        )
        .bind(run_id.to_string())
        .bind(request_key)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(map_approval_full_row))
    }

    pub async fn decide_approval(
        &self,
        approval_id: Uuid,
        decision: ApprovalDecision,
        resolved_by: Option<&str>,
    ) -> Result<ApprovalRequest> {
        let existing = self
            .get_approval(approval_id)
            .await?
            .context("approval not found")?;
        if existing.status != ApprovalStatus::Pending {
            return Ok(existing);
        }
        let now = Utc::now();
        let status = decision.status();
        let updated = sqlx::query(
            "UPDATE approval_requests
             SET status = ?, decision = ?, resolved_by = ?, resolved_at = ?
             WHERE id = ? AND status = 'pending'",
        )
        .bind(status.as_str())
        .bind(decision.as_str())
        .bind(resolved_by)
        .bind(now.to_rfc3339())
        .bind(approval_id.to_string())
        .execute(&self.pool)
        .await?;

        // A concurrent decision may have won between the read and UPDATE. The
        // affected row count, not the earlier snapshot, decides who owns the
        // transition and its side effects.
        if updated.rows_affected() == 0 {
            return self
                .get_approval(approval_id)
                .await?
                .context("approval missing after conflicting decision");
        }

        let run = self
            .get_run(existing.run_id)
            .await?
            .context("run not found")?;
        if matches!(run.status, RunStatus::WaitingForApproval) {
            self.update_run_status(existing.run_id, RunStatus::Running, None, None)
                .await?;
        }

        self.append_event(
            existing.run_id,
            0,
            Some(&format!("approval.decided.{}", existing.request_key)),
            RunEventKind::Platform,
            platform_payload(PlatformEvent::ApprovalDecided {
                approval_id,
                decision,
            }),
        )
        .await?;

        self.get_approval(approval_id)
            .await?
            .context("approval missing after decide")
    }

    pub async fn record_git_operation(
        &self,
        run_id: Uuid,
        operation: &str,
        status: &str,
        idempotency_key: &str,
        result: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let run = self
            .get_run(run_id)
            .await?
            .context("run not found")?;
        let id = Uuid::new_v4();
        let now = Utc::now();
        sqlx::query(
            "INSERT OR IGNORE INTO git_operations
             (id, organization_id, repository_id, run_id, operation, expected_head_sha,
              result_head_sha, approval_id, status, idempotency_key, provider_request_id,
              result_json, created_at, finished_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(run.organization_id.to_string())
        .bind(run.repository_id.to_string())
        .bind(run_id.to_string())
        .bind(operation)
        .bind(Option::<String>::None)
        .bind(run.head_sha.clone())
        .bind(Option::<String>::None)
        .bind(status)
        .bind(idempotency_key)
        .bind(Option::<String>::None)
        .bind(result.to_string())
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(serde_json::json!({
            "ok": true,
            "operation": operation,
            "status": status,
            "idempotencyKey": idempotency_key,
            "result": result,
        }))
    }

    async fn create_session(&self, user_id: Uuid) -> Result<String> {
        let token = format!("zc_{}", Uuid::new_v4().simple());
        let hash = hash_token(&token);
        let now = Utc::now();
        let expires = now + Duration::days(14);
        sqlx::query(
            "INSERT INTO sessions (id, user_id, token_hash, expires_at, created_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(user_id.to_string())
        .bind(hash)
        .bind(expires.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(token)
    }
}

#[derive(sqlx::FromRow)]
struct RepoRow {
    id: String,
    organization_id: String,
    provider: String,
    owner: String,
    name: String,
    default_branch: String,
    clone_url: String,
    installation_id: Option<String>,
    provider_repo_id: Option<String>,
    private: i64,
    created_at: String,
}

impl RepoRow {
    fn into_repo(self) -> Repository {
        Repository {
            id: Uuid::parse_str(&self.id).unwrap(),
            organization_id: Uuid::parse_str(&self.organization_id).unwrap(),
            provider: self.provider,
            owner: self.owner,
            name: self.name,
            default_branch: self.default_branch,
            clone_url: self.clone_url,
            installation_id: self.installation_id,
            provider_repo_id: self.provider_repo_id,
            private: self.private != 0,
            created_at: parse_time(&self.created_at),
        }
    }
}

#[derive(sqlx::FromRow)]
struct RunRow {
    id: String,
    organization_id: String,
    repository_id: String,
    requested_by: String,
    status: String,
    status_version: i64,
    title: String,
    prompt: String,
    base_ref: String,
    base_sha: Option<String>,
    head_branch: String,
    head_sha: Option<String>,
    model: String,
    permission_mode: String,
    max_turns: i64,
    created_at: String,
    started_at: Option<String>,
    finished_at: Option<String>,
    archived_at: Option<String>,
}

impl RunRow {
    fn into_run(self) -> Run {
        Run {
            id: Uuid::parse_str(&self.id).unwrap(),
            organization_id: Uuid::parse_str(&self.organization_id).unwrap(),
            repository_id: Uuid::parse_str(&self.repository_id).unwrap(),
            requested_by: Uuid::parse_str(&self.requested_by).unwrap(),
            status: RunStatus::parse(&self.status).unwrap_or(RunStatus::Failed),
            status_version: self.status_version,
            title: self.title,
            prompt: self.prompt,
            base_ref: self.base_ref,
            base_sha: self.base_sha,
            head_branch: self.head_branch,
            head_sha: self.head_sha,
            model: self.model,
            permission_mode: self.permission_mode,
            max_turns: self.max_turns.max(0) as u32,
            created_at: parse_time(&self.created_at),
            started_at: self.started_at.as_deref().map(parse_time),
            finished_at: self.finished_at.as_deref().map(parse_time),
            archived_at: self.archived_at.as_deref().map(parse_time),
        }
    }
}

fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("hash password: {e}"))?
        .to_string();
    Ok(hash)
}

fn verify_password(password: &str, hash: &str) -> Result<()> {
    let parsed = PasswordHash::new(hash).map_err(|e| anyhow::anyhow!("bad hash: {e}"))?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| anyhow::anyhow!("invalid credentials"))?;
    Ok(())
}

fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub(crate) fn parse_time(value: &str) -> chrono::DateTime<Utc> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|v| v.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn title_from_prompt(prompt: &str) -> String {
    let cleaned = prompt
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("Untitled agent")
        .trim_start_matches(['#', '-', '*', ' '])
        .trim();
    let mut title: String = cleaned.chars().take(56).collect();
    if cleaned.chars().count() > 56 {
        title.push('…');
    }
    if title.is_empty() {
        "Untitled agent".into()
    } else {
        title
    }
}

fn slugify(input: &str) -> String {
    let slug = input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        format!("org-{}", &Uuid::new_v4().to_string()[..8])
    } else {
        slug
    }
}

fn split_sql_statements(sql: &str) -> Vec<String> {
    sql.split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

fn is_ignorable_alter_error(err: &sqlx::Error, statement: &str) -> bool {
    // Migration statements may retain leading SQL comments from the file.
    // Look for the actual ALTER verb rather than requiring it at byte zero.
    let upper = statement.to_ascii_uppercase();
    if !upper.contains("ALTER TABLE") {
        return false;
    }
    let msg = err.to_string().to_ascii_lowercase();
    msg.contains("duplicate column")
        || msg.contains("already exists")
        || msg.contains("duplicate column name")
}

fn truncate_sql(statement: &str) -> String {
    let one_line = statement.replace('\n', " ");
    if one_line.len() > 120 {
        format!("{}…", &one_line[..120])
    } else {
        one_line
    }
}

#[allow(dead_code)]
pub(crate) fn map_approval_full_row(
    row: (
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    ),
) -> ApprovalRequest {
    let allowed: Vec<ApprovalDecision> = serde_json::from_str::<Vec<String>>(&row.7)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| ApprovalDecision::parse(&value))
        .collect();
    ApprovalRequest {
        id: Uuid::parse_str(&row.0).unwrap(),
        run_id: Uuid::parse_str(&row.1).unwrap(),
        request_key: row.2,
        kind: ApprovalKind::parse(&row.3).unwrap_or(ApprovalKind::Permission),
        risk: ApprovalRisk::parse(&row.4).unwrap_or(ApprovalRisk::Medium),
        payload: serde_json::from_str(&row.5).unwrap_or(serde_json::json!({})),
        status: ApprovalStatus::parse(&row.6).unwrap_or(ApprovalStatus::Pending),
        allowed_decisions: allowed,
        decision: row.8.as_deref().and_then(ApprovalDecision::parse),
        created_at: parse_time(&row.9),
        expires_at: row.10.as_deref().map(parse_time),
        resolved_by: row.11,
        resolved_at: row.12.as_deref().map(parse_time),
    }
}

const RUN_COLUMNS: &str = "id, organization_id, repository_id, requested_by, status, status_version,
    title, prompt, base_ref, base_sha, head_branch, head_sha, model, permission_mode, max_turns,
    created_at, started_at, finished_at, archived_at";

fn platform_payload(event: PlatformEvent) -> serde_json::Value {
    serde_json::to_value(event).unwrap_or_else(|_| serde_json::json!({}))
}

fn run_status_payload(status: RunStatus, head_sha: Option<String>) -> serde_json::Value {
    platform_payload(PlatformEvent::RunStatusChanged { status, head_sha })
}
