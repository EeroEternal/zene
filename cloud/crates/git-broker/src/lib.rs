//! Git Broker: short-lived clone credentials, bundle accept/push, draft PRs.
//!
//! Mock mode (`ZENE_CLOUD_GITHUB_MODE=mock`) never talks to GitHub and
//! records operations in SQLite with fake SHAs/URLs. Live mode uses installation
//! tokens and a temporary git workdir for push.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{bail, Context, Result};
use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};
use tokio::process::Command;
use uuid::Uuid;
use zene_cloud_db::Db;
use zene_cloud_domain::{
    AcceptBundleResult, CloneTokenResponse, CreatePullRequestBody, GitOperationKind,
    GitOperationStatus, GithubMode, PullRequest, PullRequestState, Run,
};
use zene_cloud_github::{CreatePullRequestParams, GithubClient};

#[derive(Clone)]
pub struct GitBroker {
    db: Db,
    github: GithubClient,
    mode: GithubMode,
}

impl GitBroker {
    pub fn new(db: Db, github: GithubClient) -> Self {
        let mode = github.mode();
        Self { db, github, mode }
    }

    pub fn from_env(db: Db) -> Result<Self> {
        let github = zene_cloud_github::from_env()?;
        Ok(Self::new(db, github))
    }

    pub fn mock(db: Db) -> Self {
        Self::new(db, GithubClient::mock())
    }

    pub fn mode(&self) -> GithubMode {
        self.mode
    }

    pub fn github(&self) -> &GithubClient {
        &self.github
    }

    /// Issue a short-lived read clone token for a run's repository.
    pub async fn issue_read_clone_token(&self, run: &Run) -> Result<CloneTokenResponse> {
        let repo = self
            .db
            .get_repository(run.repository_id)
            .await?
            .context("repository not found")?;

        let expires_at = Utc::now() + Duration::minutes(30);
        let (token, mode) = if self.mode == GithubMode::Mock || self.github.is_mock() {
            (
                format!("mock_clone_{}", &run.id.to_string()[..8]),
                GithubMode::Mock,
            )
        } else {
            let installation_id = repo
                .installation_id
                .as_deref()
                .context("repository has no GitHub installation_id")?;
            let tok = self.github.installation_token(installation_id).await?;
            (tok.token, GithubMode::Live)
        };

        self.db
            .append_audit(
                Some(run.organization_id),
                "system",
                Some("git-broker"),
                "git.clone_token.issued",
                Some("run"),
                Some(&run.id.to_string()),
                Some(serde_json::json!({
                    "repositoryId": repo.id,
                    "mode": mode,
                    "expiresAt": expires_at,
                })),
            )
            .await?;

        Ok(CloneTokenResponse {
            token,
            clone_url: repo.clone_url,
            expires_at,
            mode,
        })
    }

    /// Accept a git bundle and push the run's head branch.
    ///
    /// `bundle` may be raw bytes or a filesystem path (UTF-8 path string starting
    /// with `/` or containing `.bundle`).
    pub async fn accept_bundle_and_push(
        &self,
        run: &Run,
        bundle: &[u8],
        expected_head: Option<&str>,
        idempotency_key: &str,
    ) -> Result<AcceptBundleResult> {
        let repo = self
            .db
            .get_repository(run.repository_id)
            .await?
            .context("repository not found")?;

        let op = self
            .db
            .create_git_operation(
                run.organization_id,
                run.repository_id,
                run.id,
                GitOperationKind::PushBundle,
                expected_head,
                None,
                idempotency_key,
            )
            .await?;

        // Idempotent replay.
        if op.status == GitOperationStatus::Succeeded {
            if let Some(sha) = op.result_head_sha {
                let push_url = op
                    .result
                    .as_ref()
                    .and_then(|v| v.get("pushUrl"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                return Ok(AcceptBundleResult {
                    head_sha: sha,
                    push_url,
                    operation_id: op.id,
                });
            }
        }

        let result = if self.mode == GithubMode::Mock || self.github.is_mock() {
            self.mock_accept_bundle(run, &repo.clone_url, bundle, expected_head)
                .await
        } else {
            self.live_accept_bundle(run, &repo, bundle, expected_head)
                .await
        };

        match result {
            Ok((head_sha, push_url)) => {
                self.db
                    .finish_git_operation(
                        op.id,
                        GitOperationStatus::Succeeded,
                        Some(&head_sha),
                        None,
                        Some(serde_json::json!({
                            "pushUrl": push_url,
                            "expectedHead": expected_head,
                        })),
                    )
                    .await?;
                self.db
                    .update_run_status(run.id, run.status, Some(head_sha.clone()), None)
                    .await
                    .ok();
                self.db
                    .append_audit(
                        Some(run.organization_id),
                        "system",
                        Some("git-broker"),
                        "git.bundle.pushed",
                        Some("run"),
                        Some(&run.id.to_string()),
                        Some(serde_json::json!({
                            "headSha": head_sha,
                            "pushUrl": push_url,
                            "operationId": op.id,
                        })),
                    )
                    .await?;
                Ok(AcceptBundleResult {
                    head_sha,
                    push_url,
                    operation_id: op.id,
                })
            }
            Err(err) => {
                self.db
                    .finish_git_operation(
                        op.id,
                        GitOperationStatus::Failed,
                        None,
                        None,
                        Some(serde_json::json!({ "error": err.to_string() })),
                    )
                    .await
                    .ok();
                Err(err)
            }
        }
    }

    /// Convenience: load bundle bytes from a path then push.
    pub async fn accept_bundle_path_and_push(
        &self,
        run: &Run,
        bundle_path: &Path,
        expected_head: Option<&str>,
        idempotency_key: &str,
    ) -> Result<AcceptBundleResult> {
        let bytes = tokio::fs::read(bundle_path)
            .await
            .with_context(|| format!("read bundle {}", bundle_path.display()))?;
        self.accept_bundle_and_push(run, &bytes, expected_head, idempotency_key)
            .await
    }

    pub async fn create_draft_pr(
        &self,
        run: &Run,
        body: CreatePullRequestBody,
    ) -> Result<PullRequest> {
        let repo = self
            .db
            .get_repository(run.repository_id)
            .await?
            .context("repository not found")?;

        let base = body
            .base_ref
            .unwrap_or_else(|| run.base_ref.clone());
        let head = body
            .head_ref
            .unwrap_or_else(|| run.head_branch.clone());
        let draft = body.draft;

        let installation_id = repo
            .installation_id
            .clone()
            .unwrap_or_else(|| "mock-install".into());

        let remote = self
            .github
            .create_pull_request(
                &installation_id,
                CreatePullRequestParams {
                    owner: repo.owner.clone(),
                    repo: repo.name.clone(),
                    title: body.title.clone(),
                    body: body.body.clone(),
                    head: head.clone(),
                    base: base.clone(),
                    draft,
                },
            )
            .await?;

        let pr = self
            .db
            .create_pull_request(
                repo.id,
                run.id,
                &body.title,
                body.body.as_deref(),
                remote.provider_number,
                remote.url.as_deref(),
                run.base_sha.as_deref(),
                run.head_sha.as_deref(),
                if draft {
                    PullRequestState::Draft
                } else {
                    PullRequestState::Open
                },
                draft,
            )
            .await?;

        let key = format!("create-pr-{}", pr.id);
        let op = self
            .db
            .create_git_operation(
                run.organization_id,
                repo.id,
                run.id,
                GitOperationKind::CreatePr,
                run.head_sha.as_deref(),
                None,
                &key,
            )
            .await?;
        self.db
            .finish_git_operation(
                op.id,
                GitOperationStatus::Succeeded,
                run.head_sha.as_deref(),
                remote.provider_number.map(|n| n.to_string()).as_deref(),
                Some(serde_json::json!({
                    "pullRequestId": pr.id,
                    "url": pr.url,
                    "number": pr.provider_number,
                })),
            )
            .await?;

        self.db
            .append_audit(
                Some(run.organization_id),
                "system",
                Some("git-broker"),
                "git.pr.created",
                Some("pull_request"),
                Some(&pr.id.to_string()),
                Some(serde_json::json!({
                    "runId": run.id,
                    "url": pr.url,
                    "draft": draft,
                })),
            )
            .await?;

        Ok(pr)
    }

    pub async fn mark_pr_ready(&self, run: &Run, pr: &PullRequest) -> Result<PullRequest> {
        let repo = self
            .db
            .get_repository(run.repository_id)
            .await?
            .context("repository not found")?;
        let number = pr
            .provider_number
            .context("pull request has no provider number")?;

        let remote = if self.mode == GithubMode::Mock || self.github.is_mock() {
            self.github
                .mark_pull_request_ready("mock-install", &repo.owner, &repo.name, number)
                .await?
        } else {
            let installation_id = repo
                .installation_id
                .as_deref()
                .context("repository has no GitHub installation_id")?;
            self.github
                .mark_pull_request_ready(installation_id, &repo.owner, &repo.name, number)
                .await?
        };

        self.db
            .update_pull_request_state(pr.id, PullRequestState::Open, false)
            .await?
            .context("update pull request state")?;

        self.db
            .append_audit(
                Some(run.organization_id),
                "system",
                Some("git-broker"),
                "git.pr.ready",
                Some("pull_request"),
                Some(&pr.id.to_string()),
                Some(serde_json::json!({
                    "runId": run.id,
                    "url": remote.url,
                    "number": remote.provider_number,
                })),
            )
            .await?;

        Ok(self
            .db
            .get_pull_request(pr.id)
            .await?
            .context("pull request not found")?)
    }

    pub async fn merge_pr(&self, run: &Run, pr: &PullRequest) -> Result<PullRequest> {
        let repo = self
            .db
            .get_repository(run.repository_id)
            .await?
            .context("repository not found")?;
        let number = pr
            .provider_number
            .context("pull request has no provider number")?;

        let remote = if self.mode == GithubMode::Mock || self.github.is_mock() {
            self.github
                .merge_pull_request("mock-install", &repo.owner, &repo.name, number)
                .await?
        } else {
            let installation_id = repo
                .installation_id
                .as_deref()
                .context("repository has no GitHub installation_id")?;
            self.github
                .merge_pull_request(installation_id, &repo.owner, &repo.name, number)
                .await?
        };

        self.db
            .update_pull_request_state(pr.id, PullRequestState::Merged, false)
            .await?
            .context("update pull request state")?;

        self.db
            .append_audit(
                Some(run.organization_id),
                "system",
                Some("git-broker"),
                "git.pr.merged",
                Some("pull_request"),
                Some(&pr.id.to_string()),
                Some(serde_json::json!({
                    "runId": run.id,
                    "url": remote.url,
                    "number": remote.provider_number,
                })),
            )
            .await?;

        Ok(self
            .db
            .get_pull_request(pr.id)
            .await?
            .context("pull request not found")?)
    }

    async fn mock_accept_bundle(
        &self,
        run: &Run,
        clone_url: &str,
        bundle: &[u8],
        expected_head: Option<&str>,
    ) -> Result<(String, String)> {
        let mut hasher = Sha256::new();
        hasher.update(bundle);
        hasher.update(run.id.as_bytes());
        if let Some(h) = expected_head {
            hasher.update(h.as_bytes());
        }
        let digest = hasher.finalize();
        let head_sha = format!("{:x}", digest)
            .chars()
            .take(40)
            .collect::<String>();
        let push_url = format!(
            "{}/compare/{}...{}",
            clone_url.trim_end_matches(".git"),
            expected_head.unwrap_or("main"),
            &head_sha[..8]
        );
        Ok((head_sha, push_url))
    }

    async fn live_accept_bundle(
        &self,
        run: &Run,
        repo: &zene_cloud_domain::Repository,
        bundle: &[u8],
        expected_head: Option<&str>,
    ) -> Result<(String, String)> {
        let installation_id = repo
            .installation_id
            .as_deref()
            .context("repository has no GitHub installation_id")?;
        let token = self.github.installation_token(installation_id).await?;

        let tmp = tempfile::tempdir().context("create tempdir for bundle push")?;
        let work = tmp.path().join("repo");
        tokio::fs::create_dir_all(&work).await?;

        let bundle_path = tmp.path().join("incoming.bundle");
        tokio::fs::write(&bundle_path, bundle).await?;

        // Authenticated clone URL: https://x-access-token:TOKEN@github.com/owner/repo.git
        let auth_url = inject_token_into_https_url(&repo.clone_url, &token.token)?;

        run_git(&work, &["init"]).await?;
        run_git(&work, &["remote", "add", "origin", &auth_url]).await?;
        run_git(
            &work,
            &[
                "fetch",
                "origin",
                &format!("+refs/heads/{}:refs/remotes/origin/{}", run.base_ref, run.base_ref),
            ],
        )
        .await
        .ok(); // base may already exist; ignore soft failures before bundle

        let bundle_str = bundle_path
            .to_str()
            .context("bundle path is not utf-8")?;
        if run_git(
            &work,
            &["fetch", bundle_str, run.head_branch.as_str()],
        )
        .await
        .is_err()
        {
            run_git(&work, &["bundle", "unbundle", bundle_str])
                .await
                .context("git bundle unbundle")?;
        }

        if let Some(expected) = expected_head {
            let current = match git_stdout(&work, &["rev-parse", "FETCH_HEAD"]).await {
                Ok(v) => v,
                Err(_) => git_stdout(&work, &["rev-parse", "HEAD"])
                    .await
                    .unwrap_or_default(),
            };
            let current = current.trim();
            if !current.is_empty() && current != expected {
                tracing::warn!(
                    expected,
                    current,
                    "bundle head does not match expected_head"
                );
            }
        }

        // Ensure we have a local branch to push.
        if run_git(
            &work,
            &["checkout", "-B", &run.head_branch, "FETCH_HEAD"],
        )
        .await
        .is_err()
        {
            run_git(&work, &["checkout", "-B", &run.head_branch])
                .await
                .context("checkout head branch")?;
        }

        run_git(
            &work,
            &[
                "push",
                "-u",
                "origin",
                &format!("HEAD:refs/heads/{}", run.head_branch),
            ],
        )
        .await
        .context("git push")?;

        let head_sha = git_stdout(&work, &["rev-parse", "HEAD"])
            .await
            .context("rev-parse after push")?
            .trim()
            .to_string();
        let push_url = format!(
            "https://github.com/{}/{}/tree/{}",
            repo.owner, repo.name, run.head_branch
        );
        Ok((head_sha, push_url))
    }
}

fn inject_token_into_https_url(clone_url: &str, token: &str) -> Result<String> {
    let url = clone_url.trim();
    if let Some(rest) = url.strip_prefix("https://") {
        Ok(format!("https://x-access-token:{token}@{rest}"))
    } else if let Some(rest) = url.strip_prefix("http://") {
        Ok(format!("http://x-access-token:{token}@{rest}"))
    } else {
        bail!("unsupported clone url scheme for token injection: {clone_url}");
    }
}

async fn run_git(cwd: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .with_context(|| format!("spawn git {}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {} failed with {}: {stderr}", args.join(" "), output.status);
    }
    Ok(())
}

async fn git_stdout(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .await
        .with_context(|| format!("spawn git {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Helper used by tests / callers that want a stable fake workspace path.
pub fn mock_workspace_hint(run_id: Uuid) -> PathBuf {
    PathBuf::from(format!("/tmp/zene-cloud-mock/{run_id}"))
}
