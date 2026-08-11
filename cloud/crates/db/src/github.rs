use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use uuid::Uuid;
use zene_cloud_domain::{
    ApprovalRequest, AuditLog, GitOperation, GitOperationKind, GitOperationStatus,
    GithubAccount, GithubInstallation, GithubRepoSummary, OauthState, PullRequest, Repository,
};

use crate::{parse_time, Db};

impl Db {
    pub async fn save_oauth_state(
        &self,
        state: &str,
        user_id: Option<Uuid>,
        redirect_to: Option<&str>,
        ttl_secs: i64,
    ) -> Result<OauthState> {
        let now = Utc::now();
        let expires = now + Duration::seconds(ttl_secs.max(60));
        sqlx::query(
            "INSERT INTO oauth_states (state, user_id, redirect_to, created_at, expires_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(state)
        .bind(user_id.map(|v| v.to_string()))
        .bind(redirect_to)
        .bind(now.to_rfc3339())
        .bind(expires.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(OauthState {
            state: state.into(),
            user_id,
            redirect_to: redirect_to.map(|s| s.into()),
            created_at: now,
            expires_at: expires,
        })
    }

    /// Atomically fetch and delete an OAuth state if it exists and is not expired.
    pub async fn take_oauth_state(&self, state: &str) -> Result<Option<OauthState>> {
        let mut tx = self.pool.begin().await?;
        let row: Option<(String, Option<String>, Option<String>, String, String)> =
            sqlx::query_as(
                "SELECT state, user_id, redirect_to, created_at, expires_at
                 FROM oauth_states WHERE state = ?",
            )
            .bind(state)
            .fetch_optional(&mut *tx)
            .await?;
        let Some((state, user_id, redirect_to, created_at, expires_at)) = row else {
            return Ok(None);
        };
        sqlx::query("DELETE FROM oauth_states WHERE state = ?")
            .bind(&state)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        let expires = parse_time(&expires_at);
        if expires < Utc::now() {
            return Ok(None);
        }
        Ok(Some(OauthState {
            state,
            user_id: user_id.and_then(|v| Uuid::parse_str(&v).ok()),
            redirect_to,
            created_at: parse_time(&created_at),
            expires_at: expires,
        }))
    }

    pub async fn upsert_github_account(
        &self,
        user_id: Uuid,
        github_user_id: &str,
        login: &str,
        access_token_enc: &str,
        token_type: &str,
        scope: Option<&str>,
    ) -> Result<GithubAccount> {
        let now = Utc::now();
        let existing: Option<(String, String)> =
            sqlx::query_as("SELECT id, created_at FROM github_accounts WHERE user_id = ?")
                .bind(user_id.to_string())
                .fetch_optional(&self.pool)
                .await?;
        let (id, created_at) = if let Some((id, created_at)) = existing {
            sqlx::query(
                "UPDATE github_accounts
                 SET github_user_id = ?, login = ?, access_token_enc = ?, token_type = ?,
                     scope = ?, updated_at = ?
                 WHERE id = ?",
            )
            .bind(github_user_id)
            .bind(login)
            .bind(access_token_enc)
            .bind(token_type)
            .bind(scope)
            .bind(now.to_rfc3339())
            .bind(&id)
            .execute(&self.pool)
            .await?;
            (Uuid::parse_str(&id)?, parse_time(&created_at))
        } else {
            let id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO github_accounts
                 (id, user_id, github_user_id, login, access_token_enc, token_type, scope,
                  created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(id.to_string())
            .bind(user_id.to_string())
            .bind(github_user_id)
            .bind(login)
            .bind(access_token_enc)
            .bind(token_type)
            .bind(scope)
            .bind(now.to_rfc3339())
            .bind(now.to_rfc3339())
            .execute(&self.pool)
            .await?;
            (id, now)
        };
        Ok(GithubAccount {
            id,
            user_id,
            github_user_id: github_user_id.into(),
            login: login.into(),
            access_token_enc: access_token_enc.into(),
            token_type: token_type.into(),
            scope: scope.map(|s| s.into()),
            created_at,
            updated_at: now,
        })
    }

    pub async fn get_github_account(&self, user_id: Uuid) -> Result<Option<GithubAccount>> {
        let row: Option<(
            String,
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            String,
            String,
        )> = sqlx::query_as(
            "SELECT id, user_id, github_user_id, login, access_token_enc, token_type, scope,
                    created_at, updated_at
             FROM github_accounts WHERE user_id = ?",
        )
        .bind(user_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(
            |(
                id,
                user_id,
                github_user_id,
                login,
                access_token_enc,
                token_type,
                scope,
                created_at,
                updated_at,
            )| GithubAccount {
                id: Uuid::parse_str(&id).unwrap(),
                user_id: Uuid::parse_str(&user_id).unwrap(),
                github_user_id,
                login,
                access_token_enc,
                token_type,
                scope,
                created_at: parse_time(&created_at),
                updated_at: parse_time(&updated_at),
            },
        ))
    }

    pub async fn upsert_installation(
        &self,
        organization_id: Uuid,
        installation_id: &str,
        account_login: &str,
        account_type: &str,
        status: &str,
    ) -> Result<GithubInstallation> {
        let now = Utc::now();
        let existing: Option<(String, String)> = sqlx::query_as(
            "SELECT id, created_at FROM github_installations WHERE installation_id = ?",
        )
        .bind(installation_id)
        .fetch_optional(&self.pool)
        .await?;
        let (id, created_at) = if let Some((id, created_at)) = existing {
            sqlx::query(
                "UPDATE github_installations
                 SET organization_id = ?, account_login = ?, account_type = ?, status = ?,
                     updated_at = ?
                 WHERE id = ?",
            )
            .bind(organization_id.to_string())
            .bind(account_login)
            .bind(account_type)
            .bind(status)
            .bind(now.to_rfc3339())
            .bind(&id)
            .execute(&self.pool)
            .await?;
            (Uuid::parse_str(&id)?, parse_time(&created_at))
        } else {
            let id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO github_installations
                 (id, organization_id, installation_id, account_login, account_type, status,
                  created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(id.to_string())
            .bind(organization_id.to_string())
            .bind(installation_id)
            .bind(account_login)
            .bind(account_type)
            .bind(status)
            .bind(now.to_rfc3339())
            .bind(now.to_rfc3339())
            .execute(&self.pool)
            .await?;
            (id, now)
        };
        Ok(GithubInstallation {
            id,
            organization_id,
            installation_id: installation_id.into(),
            account_login: account_login.into(),
            account_type: account_type.into(),
            status: status.into(),
            created_at,
            updated_at: now,
        })
    }

    pub async fn list_installations(
        &self,
        organization_id: Uuid,
    ) -> Result<Vec<GithubInstallation>> {
        let rows: Vec<(String, String, String, String, String, String, String, String)> =
            sqlx::query_as(
                "SELECT id, organization_id, installation_id, account_login, account_type, status,
                        created_at, updated_at
                 FROM github_installations WHERE organization_id = ?
                 ORDER BY created_at DESC",
            )
            .bind(organization_id.to_string())
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| GithubInstallation {
                id: Uuid::parse_str(&r.0).unwrap(),
                organization_id: Uuid::parse_str(&r.1).unwrap(),
                installation_id: r.2,
                account_login: r.3,
                account_type: r.4,
                status: r.5,
                created_at: parse_time(&r.6),
                updated_at: parse_time(&r.7),
            })
            .collect())
    }

    pub async fn get_installation_by_provider_id(
        &self,
        installation_id: &str,
    ) -> Result<Option<GithubInstallation>> {
        let row: Option<(String, String, String, String, String, String, String, String)> =
            sqlx::query_as(
                "SELECT id, organization_id, installation_id, account_login, account_type, status,
                        created_at, updated_at
                 FROM github_installations WHERE installation_id = ?",
            )
            .bind(installation_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| GithubInstallation {
            id: Uuid::parse_str(&r.0).unwrap(),
            organization_id: Uuid::parse_str(&r.1).unwrap(),
            installation_id: r.2,
            account_login: r.3,
            account_type: r.4,
            status: r.5,
            created_at: parse_time(&r.6),
            updated_at: parse_time(&r.7),
        }))
    }

    /// Upsert repositories discovered from a GitHub installation listing.
    pub async fn sync_repos_from_github(
        &self,
        organization_id: Uuid,
        repos: &[GithubRepoSummary],
    ) -> Result<Vec<Repository>> {
        let mut out = Vec::with_capacity(repos.len());
        for repo in repos {
            out.push(self.create_repo_from_github(organization_id, repo).await?);
        }
        Ok(out)
    }

    pub async fn create_repo_from_github(
        &self,
        organization_id: Uuid,
        repo: &GithubRepoSummary,
    ) -> Result<Repository> {
        let now = Utc::now();
        let existing: Option<(String, String)> = sqlx::query_as(
            "SELECT id, created_at FROM repositories
             WHERE organization_id = ? AND provider = 'github'
               AND (
                 (provider_repo_id IS NOT NULL AND provider_repo_id = ?)
                 OR (owner = ? AND name = ?)
               )
             LIMIT 1",
        )
        .bind(organization_id.to_string())
        .bind(&repo.provider_repo_id)
        .bind(&repo.owner)
        .bind(&repo.name)
        .fetch_optional(&self.pool)
        .await?;

        let (id, created_at) = if let Some((id, created_at)) = existing {
            sqlx::query(
                "UPDATE repositories
                 SET owner = ?, name = ?, default_branch = ?, clone_url = ?,
                     installation_id = ?, provider_repo_id = ?, private = ?
                 WHERE id = ?",
            )
            .bind(&repo.owner)
            .bind(&repo.name)
            .bind(&repo.default_branch)
            .bind(&repo.clone_url)
            .bind(&repo.installation_id)
            .bind(&repo.provider_repo_id)
            .bind(if repo.private { 1 } else { 0 })
            .bind(&id)
            .execute(&self.pool)
            .await?;
            (Uuid::parse_str(&id)?, parse_time(&created_at))
        } else {
            let id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO repositories
                 (id, organization_id, provider, owner, name, default_branch, clone_url,
                  installation_id, provider_repo_id, private, created_at)
                 VALUES (?, ?, 'github', ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(id.to_string())
            .bind(organization_id.to_string())
            .bind(&repo.owner)
            .bind(&repo.name)
            .bind(&repo.default_branch)
            .bind(&repo.clone_url)
            .bind(&repo.installation_id)
            .bind(&repo.provider_repo_id)
            .bind(if repo.private { 1 } else { 0 })
            .bind(now.to_rfc3339())
            .execute(&self.pool)
            .await?;
            (id, now)
        };

        Ok(Repository {
            id,
            organization_id,
            provider: "github".into(),
            owner: repo.owner.clone(),
            name: repo.name.clone(),
            default_branch: repo.default_branch.clone(),
            clone_url: repo.clone_url.clone(),
            installation_id: Some(repo.installation_id.clone()),
            provider_repo_id: Some(repo.provider_repo_id.clone()),
            private: repo.private,
            created_at,
        })
    }

    pub async fn resolve_approval(
        &self,
        approval_id: Uuid,
        resolved_by: Uuid,
        decision: &str,
    ) -> Result<ApprovalRequest> {
        self.decide_approval(approval_id, decision, Some(&resolved_by.to_string()))
            .await
    }

    pub async fn list_pending_approvals(&self, run_id: Uuid) -> Result<Vec<ApprovalRequest>> {
        let rows: Vec<(
            String,
            String,
            String,
            Option<String>,
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
            "SELECT id, run_id, request_key, jsonrpc_id, kind, risk, payload_json, status,
                    allowed_decisions, decision, created_at, expires_at, resolved_by, resolved_at
             FROM approval_requests
             WHERE run_id = ? AND status = 'pending'
             ORDER BY created_at ASC",
        )
        .bind(run_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(crate::map_approval_full_row)
            .collect())
    }

    pub async fn create_git_operation(
        &self,
        organization_id: Uuid,
        repository_id: Uuid,
        run_id: Uuid,
        operation: GitOperationKind,
        expected_head_sha: Option<&str>,
        approval_id: Option<Uuid>,
        idempotency_key: &str,
    ) -> Result<GitOperation> {
        let existing: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM git_operations
             WHERE repository_id = ? AND idempotency_key = ?",
        )
        .bind(repository_id.to_string())
        .bind(idempotency_key)
        .fetch_optional(&self.pool)
        .await?;
        if let Some((id,)) = existing {
            return self
                .get_git_operation(Uuid::parse_str(&id)?)
                .await?
                .context("git operation missing");
        }

        let id = Uuid::new_v4();
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO git_operations
             (id, organization_id, repository_id, run_id, operation, expected_head_sha,
              approval_id, status, idempotency_key, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(organization_id.to_string())
        .bind(repository_id.to_string())
        .bind(run_id.to_string())
        .bind(operation.as_str())
        .bind(expected_head_sha)
        .bind(approval_id.map(|v| v.to_string()))
        .bind(GitOperationStatus::Running.as_str())
        .bind(idempotency_key)
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(GitOperation {
            id,
            organization_id,
            repository_id,
            run_id,
            operation,
            expected_head_sha: expected_head_sha.map(|s| s.into()),
            result_head_sha: None,
            approval_id,
            status: GitOperationStatus::Running,
            idempotency_key: idempotency_key.into(),
            provider_request_id: None,
            result: None,
            created_at: now,
            finished_at: None,
        })
    }

    pub async fn finish_git_operation(
        &self,
        operation_id: Uuid,
        status: GitOperationStatus,
        result_head_sha: Option<&str>,
        provider_request_id: Option<&str>,
        result: Option<serde_json::Value>,
    ) -> Result<GitOperation> {
        let now = Utc::now();
        sqlx::query(
            "UPDATE git_operations
             SET status = ?, result_head_sha = COALESCE(?, result_head_sha),
                 provider_request_id = COALESCE(?, provider_request_id),
                 result_json = COALESCE(?, result_json),
                 finished_at = ?
             WHERE id = ?",
        )
        .bind(status.as_str())
        .bind(result_head_sha)
        .bind(provider_request_id)
        .bind(result.as_ref().map(|v| v.to_string()))
        .bind(now.to_rfc3339())
        .bind(operation_id.to_string())
        .execute(&self.pool)
        .await?;
        self.get_git_operation(operation_id)
            .await?
            .context("git operation missing after finish")
    }

    pub async fn get_git_operation(&self, id: Uuid) -> Result<Option<GitOperation>> {
        let row: Option<(
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            String,
            String,
            Option<String>,
            Option<String>,
            String,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT id, organization_id, repository_id, run_id, operation, expected_head_sha,
                    result_head_sha, approval_id, status, idempotency_key, provider_request_id,
                    result_json, created_at, finished_at
             FROM git_operations WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(
            |(
                id,
                organization_id,
                repository_id,
                run_id,
                operation,
                expected_head_sha,
                result_head_sha,
                approval_id,
                status,
                idempotency_key,
                provider_request_id,
                result_json,
                created_at,
                finished_at,
            )| {
                GitOperation {
                    id: Uuid::parse_str(&id).unwrap(),
                    organization_id: Uuid::parse_str(&organization_id).unwrap(),
                    repository_id: Uuid::parse_str(&repository_id).unwrap(),
                    run_id: Uuid::parse_str(&run_id).unwrap(),
                    operation: GitOperationKind::parse(&operation)
                        .unwrap_or(GitOperationKind::PushBundle),
                    expected_head_sha,
                    result_head_sha,
                    approval_id: approval_id.and_then(|v| Uuid::parse_str(&v).ok()),
                    status: GitOperationStatus::parse(&status)
                        .unwrap_or(GitOperationStatus::Failed),
                    idempotency_key,
                    provider_request_id,
                    result: result_json
                        .as_deref()
                        .and_then(|s| serde_json::from_str(s).ok()),
                    created_at: parse_time(&created_at),
                    finished_at: finished_at.as_deref().map(parse_time),
                }
            },
        ))
    }

    pub async fn create_pull_request(
        &self,
        repository_id: Uuid,
        run_id: Uuid,
        title: &str,
        body: Option<&str>,
        provider_number: Option<i64>,
        url: Option<&str>,
        base_sha: Option<&str>,
        head_sha: Option<&str>,
        state: &str,
        draft: bool,
    ) -> Result<PullRequest> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO pull_requests
             (id, repository_id, run_id, provider_number, url, title, body, base_sha, head_sha,
              state, draft, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(repository_id.to_string())
        .bind(run_id.to_string())
        .bind(provider_number)
        .bind(url)
        .bind(title)
        .bind(body)
        .bind(base_sha)
        .bind(head_sha)
        .bind(state)
        .bind(if draft { 1 } else { 0 })
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(PullRequest {
            id,
            repository_id,
            run_id,
            provider_number,
            url: url.map(|s| s.into()),
            title: title.into(),
            body: body.map(|s| s.into()),
            base_sha: base_sha.map(|s| s.into()),
            head_sha: head_sha.map(|s| s.into()),
            state: state.into(),
            draft,
            created_at: now,
        })
    }

    pub async fn list_pull_requests_for_run(&self, run_id: Uuid) -> Result<Vec<PullRequest>> {
        let rows: Vec<(
            String,
            String,
            String,
            Option<i64>,
            Option<String>,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            String,
            i64,
            String,
        )> = sqlx::query_as(
            "SELECT id, repository_id, run_id, provider_number, url, title, body, base_sha,
                    head_sha, state, draft, created_at
             FROM pull_requests WHERE run_id = ? ORDER BY created_at DESC",
        )
        .bind(run_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    repository_id,
                    run_id,
                    provider_number,
                    url,
                    title,
                    body,
                    base_sha,
                    head_sha,
                    state,
                    draft,
                    created_at,
                )| {
                    PullRequest {
                        id: Uuid::parse_str(&id).unwrap(),
                        repository_id: Uuid::parse_str(&repository_id).unwrap(),
                        run_id: Uuid::parse_str(&run_id).unwrap(),
                        provider_number,
                        url,
                        title,
                        body,
                        base_sha,
                        head_sha,
                        state,
                        draft: draft != 0,
                        created_at: parse_time(&created_at),
                    }
                },
            )
            .collect())
    }

    pub async fn append_audit(
        &self,
        organization_id: Option<Uuid>,
        actor_type: &str,
        actor_id: Option<&str>,
        action: &str,
        resource_type: Option<&str>,
        resource_id: Option<&str>,
        metadata: Option<serde_json::Value>,
    ) -> Result<AuditLog> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO audit_logs
             (id, organization_id, actor_type, actor_id, action, resource_type, resource_id,
              metadata_json, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(organization_id.map(|v| v.to_string()))
        .bind(actor_type)
        .bind(actor_id)
        .bind(action)
        .bind(resource_type)
        .bind(resource_id)
        .bind(metadata.as_ref().map(|v| v.to_string()))
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(AuditLog {
            id,
            organization_id,
            actor_type: actor_type.into(),
            actor_id: actor_id.map(|s| s.into()),
            action: action.into(),
            resource_type: resource_type.map(|s| s.into()),
            resource_id: resource_id.map(|s| s.into()),
            metadata,
            created_at: now,
        })
    }

    pub async fn get_github_provider_config(
        &self,
        organization_id: Uuid,
    ) -> Result<Option<zene_cloud_domain::GithubProviderConfig>> {
        use zene_cloud_domain::GithubMode;
        let row: Option<(String, String, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, String)> =
            sqlx::query_as(
                "SELECT organization_id, mode, client_id, client_secret, app_id, app_private_key, app_slug, updated_at
                 FROM github_provider_config WHERE organization_id = ?",
            )
            .bind(organization_id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(
            |(organization_id, mode, client_id, client_secret, app_id, app_private_key, app_slug, updated_at)| {
                zene_cloud_domain::GithubProviderConfig {
                    organization_id: Uuid::parse_str(&organization_id).unwrap_or_else(|_| Uuid::nil()),
                    mode: match mode.to_ascii_lowercase().as_str() {
                        "mock" => GithubMode::Mock,
                        _ => GithubMode::Live,
                    },
                    client_id,
                    client_secret,
                    app_id,
                    app_private_key,
                    app_slug,
                    updated_at: parse_time(&updated_at),
                }
            },
        ))
    }

    pub async fn upsert_github_provider_config(
        &self,
        organization_id: Uuid,
        req: zene_cloud_domain::UpdateGithubProviderConfigRequest,
    ) -> Result<zene_cloud_domain::GithubProviderConfig> {
        use zene_cloud_domain::GithubMode;
        let now = Utc::now();
        let existing = self.get_github_provider_config(organization_id).await?;
        let mode = req
            .mode
            .or_else(|| existing.as_ref().map(|c| c.mode))
            .unwrap_or(GithubMode::Live);
        let client_id = req
            .client_id
            .filter(|s| !s.trim().is_empty())
            .or_else(|| existing.as_ref().and_then(|c| c.client_id.clone()));
        let client_secret = req
            .client_secret
            .filter(|s| !s.trim().is_empty())
            .or_else(|| existing.as_ref().and_then(|c| c.client_secret.clone()));
        let app_id = req
            .app_id
            .filter(|s| !s.trim().is_empty())
            .or_else(|| existing.as_ref().and_then(|c| c.app_id.clone()));
        let app_private_key = req
            .app_private_key
            .filter(|s| !s.trim().is_empty())
            .or_else(|| existing.as_ref().and_then(|c| c.app_private_key.clone()));
        let app_slug = req
            .app_slug
            .filter(|s| !s.trim().is_empty())
            .or_else(|| existing.as_ref().and_then(|c| c.app_slug.clone()));

        if existing.is_some() {
            sqlx::query(
                "UPDATE github_provider_config
                 SET mode = ?, client_id = ?, client_secret = ?, app_id = ?,
                     app_private_key = ?, app_slug = ?, updated_at = ?
                 WHERE organization_id = ?",
            )
            .bind(mode.as_str())
            .bind(&client_id)
            .bind(&client_secret)
            .bind(&app_id)
            .bind(&app_private_key)
            .bind(&app_slug)
            .bind(now.to_rfc3339())
            .bind(organization_id.to_string())
            .execute(&self.pool)
            .await?;
        } else {
            sqlx::query(
                "INSERT INTO github_provider_config
                 (organization_id, mode, client_id, client_secret, app_id, app_private_key, app_slug, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(organization_id.to_string())
            .bind(mode.as_str())
            .bind(&client_id)
            .bind(&client_secret)
            .bind(&app_id)
            .bind(&app_private_key)
            .bind(&app_slug)
            .bind(now.to_rfc3339())
            .bind(now.to_rfc3339())
            .execute(&self.pool)
            .await?;
        }

        Ok(zene_cloud_domain::GithubProviderConfig {
            organization_id,
            mode,
            client_id,
            client_secret,
            app_id,
            app_private_key,
            app_slug,
            updated_at: now,
        })
    }

    /// Remove all mock GitHub data for an organization (live mode).
    pub async fn purge_mock_github_data(&self, organization_id: Uuid) -> Result<()> {
        sqlx::query(
            "DELETE FROM pull_requests
             WHERE repository_id IN (
                 SELECT id FROM repositories
                 WHERE organization_id = ? AND provider = 'github'
                   AND (owner = 'mock-org' OR provider_repo_id IN ('9001', '9002'))
             )",
        )
        .bind(organization_id.to_string())
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "DELETE FROM git_operations
             WHERE repository_id IN (
                 SELECT id FROM repositories
                 WHERE organization_id = ? AND provider = 'github'
                   AND (owner = 'mock-org' OR provider_repo_id IN ('9001', '9002'))
             )",
        )
        .bind(organization_id.to_string())
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "DELETE FROM runs
             WHERE repository_id IN (
                 SELECT id FROM repositories
                 WHERE organization_id = ? AND provider = 'github'
                   AND (owner = 'mock-org' OR provider_repo_id IN ('9001', '9002'))
             )",
        )
        .bind(organization_id.to_string())
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "DELETE FROM repositories
             WHERE organization_id = ? AND provider = 'github'
               AND (owner = 'mock-org' OR provider_repo_id IN ('9001', '9002'))",
        )
        .bind(organization_id.to_string())
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "DELETE FROM github_installations
             WHERE organization_id = ? AND (installation_id = '10001' OR account_login = 'mock-org')",
        )
        .bind(organization_id.to_string())
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "DELETE FROM github_accounts
             WHERE user_id IN (
                 SELECT user_id FROM organization_members WHERE organization_id = ?
             )
               AND (login = 'mock-user' OR github_user_id = '1001'
                    OR access_token_enc LIKE 'mock_oauth%')",
        )
        .bind(organization_id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Remove all mock GitHub artifacts across the database (live mode startup / migration).
    pub async fn purge_all_mock_github_data(&self) -> Result<()> {
        sqlx::query(
            "DELETE FROM pull_requests
             WHERE repository_id IN (
                 SELECT id FROM repositories
                 WHERE provider = 'github'
                   AND (owner = 'mock-org' OR provider_repo_id IN ('9001', '9002'))
             )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "DELETE FROM git_operations
             WHERE repository_id IN (
                 SELECT id FROM repositories
                 WHERE provider = 'github'
                   AND (owner = 'mock-org' OR provider_repo_id IN ('9001', '9002'))
             )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "DELETE FROM runs
             WHERE repository_id IN (
                 SELECT id FROM repositories
                 WHERE provider = 'github'
                   AND (owner = 'mock-org' OR provider_repo_id IN ('9001', '9002'))
             )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "DELETE FROM repositories
             WHERE provider = 'github'
               AND (owner = 'mock-org' OR provider_repo_id IN ('9001', '9002'))",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "DELETE FROM github_installations
             WHERE installation_id = '10001' OR account_login = 'mock-org'",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "DELETE FROM github_accounts
             WHERE login = 'mock-user' OR github_user_id = '1001'
               OR access_token_enc LIKE 'mock_oauth%'",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("DELETE FROM run_clone_credentials WHERE mock = 1")
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

