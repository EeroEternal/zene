use anyhow::{bail, Context, Result};
use serde::Deserialize;
use zene_cloud_domain::{
    GithubAccountType, GithubRepoSummary, GithubUser, PullRequest, PullRequestState,
};

use crate::app::{GithubAppAuth, InstallationToken};
use crate::oauth::{self, OauthConfig, OauthTokens};
use crate::types::{CreatePullRequestParams, GithubConfig, ListedRepo};

#[derive(Debug, Clone)]
pub struct AppInstallation {
    pub id: String,
    pub account_login: String,
    pub account_type: GithubAccountType,
}

#[derive(Clone)]
pub struct GithubClient {
    config: GithubConfig,
    http: reqwest::Client,
    app: GithubAppAuth,
}

impl GithubClient {
    pub fn new(config: GithubConfig) -> Self {
        let app = GithubAppAuth::from_config(&config);
        let http = reqwest::Client::builder()
            .user_agent("zene-cloud")
            .build()
            .expect("reqwest client");
        Self { config, http, app }
    }

    pub fn mock() -> Self {
        Self::new(GithubConfig::mock())
    }

    pub fn mode(&self) -> zene_cloud_domain::GithubMode {
        self.config.mode
    }

    pub fn is_mock(&self) -> bool {
        self.config.is_mock()
    }

    pub fn config(&self) -> &GithubConfig {
        &self.config
    }

    pub fn app(&self) -> &GithubAppAuth {
        &self.app
    }

    pub fn oauth_config(&self, redirect_uri: Option<String>) -> Result<OauthConfig> {
        OauthConfig::from_github_config(&self.config, redirect_uri)
    }

    /// Start OAuth: returns `(authorize_url, state)`.
    pub fn begin_oauth(&self, redirect_uri: Option<String>) -> Result<(String, String)> {
        let cfg = self.oauth_config(redirect_uri)?;
        let state = oauth::new_oauth_state();
        let url = oauth::build_authorize_url(&cfg, &state);
        Ok((url, state))
    }

    pub async fn exchange_oauth_code(
        &self,
        code: &str,
        redirect_uri: Option<String>,
    ) -> Result<OauthTokens> {
        let cfg = self.oauth_config(redirect_uri)?;
        oauth::exchange_code(&cfg, &self.http, code, self.is_mock()).await
    }

    pub async fn get_user(&self, access_token: &str) -> Result<GithubUser> {
        if self.is_mock() {
            return Ok(GithubUser {
                id: "1001".into(),
                login: "mock-user".into(),
                name: Some("Mock User".into()),
                avatar_url: Some("https://avatars.githubusercontent.com/u/0".into()),
            });
        }

        #[derive(Deserialize)]
        struct GhUser {
            id: i64,
            login: String,
            name: Option<String>,
            avatar_url: Option<String>,
        }

        let resp = self
            .http
            .get(format!("{}/user", self.config.api_base))
            .header("Accept", "application/vnd.github+json")
            .header("Authorization", format!("Bearer {access_token}"))
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .context("get user")?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("get user failed ({status}): {text}");
        }
        let u: GhUser = serde_json::from_str(&text).context("parse user")?;
        Ok(GithubUser {
            id: u.id.to_string(),
            login: u.login,
            name: u.name,
            avatar_url: u.avatar_url,
        })
    }

    pub async fn create_app_jwt(&self) -> Result<String> {
        self.app.create_app_jwt()
    }

    pub fn install_url(&self) -> Option<String> {
        self.app.install_url()
    }

    pub fn install_url_with_state(&self, state: &str) -> Option<String> {
        self.app.install_url_with_state(state)
    }

    pub async fn get_installation(&self, installation_id: &str) -> Result<AppInstallation> {
        if self.is_mock() {
            return Ok(AppInstallation {
                id: installation_id.into(),
                account_login: "mock-org".into(),
                account_type: GithubAccountType::Organization,
            });
        }

        let jwt = self.create_app_jwt().await?;
        let url = format!(
            "{}/app/installations/{installation_id}",
            self.config.api_base
        );
        #[derive(Deserialize)]
        struct Resp {
            id: i64,
            account: RawAccount,
        }
        #[derive(Deserialize)]
        struct RawAccount {
            login: String,
            #[serde(rename = "type")]
            account_type: String,
        }

        let resp = self
            .http
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("Authorization", format!("Bearer {jwt}"))
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .context("get installation")?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("get installation failed ({status}): {text}");
        }
        let body: Resp = serde_json::from_str(&text).context("parse installation")?;
        Ok(AppInstallation {
            id: body.id.to_string(),
            account_login: body.account.login,
            account_type: github_account_type(&body.account.account_type),
        })
    }

    pub async fn list_app_installations(&self) -> Result<Vec<AppInstallation>> {
        if self.is_mock() {
            return Ok(vec![AppInstallation {
                id: "10001".into(),
                account_login: "mock-org".into(),
                account_type: GithubAccountType::Organization,
            }]);
        }

        let jwt = self.create_app_jwt().await?;
        let mut page = 1u32;
        let mut out = Vec::new();
        loop {
            let url = format!(
                "{}/app/installations?per_page=100&page={page}",
                self.config.api_base
            );
            #[derive(Deserialize)]
            struct Page {
                id: i64,
                account: RawAccount,
            }
            #[derive(Deserialize)]
            struct RawAccount {
                login: String,
                #[serde(rename = "type")]
                account_type: String,
            }

            let resp = self
                .http
                .get(&url)
                .header("Accept", "application/vnd.github+json")
                .header("Authorization", format!("Bearer {jwt}"))
                .header("X-GitHub-Api-Version", "2022-11-28")
                .send()
                .await
                .context("list app installations")?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                bail!("list app installations failed ({status}): {text}");
            }
            let page_body: Vec<Page> =
                serde_json::from_str(&text).context("parse app installations")?;
            let count = page_body.len();
            for inst in page_body {
                out.push(AppInstallation {
                    id: inst.id.to_string(),
                    account_login: inst.account.login,
                    account_type: github_account_type(&inst.account.account_type),
                });
            }
            if count < 100 {
                break;
            }
            page += 1;
            if page > 50 {
                break;
            }
        }
        Ok(out)
    }

    pub async fn installation_token(&self, installation_id: &str) -> Result<InstallationToken> {
        self.app
            .installation_token(&self.http, &self.config.api_base, installation_id)
            .await
    }

    pub async fn list_installation_repos(
        &self,
        installation_id: &str,
    ) -> Result<Vec<GithubRepoSummary>> {
        if self.is_mock() {
            return Ok(vec![
                GithubRepoSummary {
                    provider_repo_id: "9001".into(),
                    owner: "mock-org".into(),
                    name: "demo".into(),
                    default_branch: "main".into(),
                    clone_url: "https://github.com/mock-org/demo.git".into(),
                    private: false,
                    installation_id: installation_id.into(),
                },
                GithubRepoSummary {
                    provider_repo_id: "9002".into(),
                    owner: "mock-org".into(),
                    name: "private-app".into(),
                    default_branch: "main".into(),
                    clone_url: "https://github.com/mock-org/private-app.git".into(),
                    private: true,
                    installation_id: installation_id.into(),
                },
            ]);
        }

        let token = self.installation_token(installation_id).await?;
        let mut page = 1u32;
        let mut out = Vec::new();
        loop {
            let url = format!(
                "{}/installation/repositories?per_page=100&page={page}",
                self.config.api_base
            );
            #[derive(Deserialize)]
            struct Page {
                repositories: Vec<RawRepo>,
                total_count: Option<i64>,
            }
            #[derive(Deserialize)]
            struct RawRepo {
                id: i64,
                name: String,
                full_name: String,
                private: bool,
                clone_url: String,
                default_branch: Option<String>,
                owner: RawOwner,
            }
            #[derive(Deserialize)]
            struct RawOwner {
                login: String,
            }

            let resp = self
                .http
                .get(&url)
                .header("Accept", "application/vnd.github+json")
                .header("Authorization", format!("Bearer {}", token.token))
                .header("X-GitHub-Api-Version", "2022-11-28")
                .send()
                .await
                .context("list installation repos")?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                bail!("list installation repos failed ({status}): {text}");
            }
            let page_body: Page = serde_json::from_str(&text).context("parse repos page")?;
            let count = page_body.repositories.len();
            for r in page_body.repositories {
                let _ = r.full_name;
                out.push(GithubRepoSummary {
                    provider_repo_id: r.id.to_string(),
                    owner: r.owner.login,
                    name: r.name,
                    default_branch: r.default_branch.unwrap_or_else(|| "main".into()),
                    clone_url: r.clone_url,
                    private: r.private,
                    installation_id: installation_id.into(),
                });
            }
            if count < 100 {
                break;
            }
            if let Some(total) = page_body.total_count {
                if out.len() as i64 >= total {
                    break;
                }
            }
            page += 1;
            if page > 50 {
                break;
            }
        }
        Ok(out)
    }

    /// Convenience returning the crate-local ListedRepo shape.
    pub async fn list_installation_repos_raw(
        &self,
        installation_id: &str,
    ) -> Result<Vec<ListedRepo>> {
        let repos = self.list_installation_repos(installation_id).await?;
        Ok(repos
            .into_iter()
            .map(|r| ListedRepo {
                id: r.provider_repo_id.parse().unwrap_or(0),
                full_name: format!("{}/{}", r.owner, r.name),
                name: r.name,
                owner_login: r.owner,
                default_branch: r.default_branch,
                clone_url: r.clone_url,
                private: r.private,
            })
            .collect())
    }

    pub async fn list_repo_branches(
        &self,
        installation_id: &str,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<String>> {
        if self.is_mock() {
            return Ok(vec![
                "main".into(),
                "dev".into(),
                "deploy".into(),
                "feature-deploy-codebase-optimizations".into(),
            ]);
        }

        let token = self.installation_token(installation_id).await?;
        let mut page = 1u32;
        let mut out = Vec::new();
        loop {
            let url = format!(
                "{}/repos/{owner}/{repo}/branches?per_page=100&page={page}",
                self.config.api_base
            );
            #[derive(Deserialize)]
            struct Branch {
                name: String,
            }

            let resp = self
                .http
                .get(&url)
                .header("Accept", "application/vnd.github+json")
                .header("Authorization", format!("Bearer {}", token.token))
                .header("X-GitHub-Api-Version", "2022-11-28")
                .send()
                .await
                .with_context(|| format!("list branches for {owner}/{repo}"))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                bail!("list branches failed ({status}): {text}");
            }
            let page_body: Vec<Branch> =
                serde_json::from_str(&text).context("parse branches page")?;
            let count = page_body.len();
            for branch in page_body {
                out.push(branch.name);
            }
            if count < 100 {
                break;
            }
            page += 1;
            if page > 50 {
                break;
            }
        }
        Ok(out)
    }

    pub async fn create_pull_request(
        &self,
        installation_id: &str,
        params: CreatePullRequestParams,
    ) -> Result<PullRequest> {
        if self.is_mock() {
            let number = 42i64;
            return Ok(PullRequest {
                id: uuid::Uuid::new_v4(),
                repository_id: uuid::Uuid::nil(),
                run_id: uuid::Uuid::nil(),
                provider_number: Some(number),
                url: Some(format!(
                    "https://github.com/{}/{}/pull/{number}",
                    params.owner, params.repo
                )),
                title: params.title,
                body: params.body,
                base_sha: None,
                head_sha: None,
                state: if params.draft {
                    PullRequestState::Draft
                } else {
                    PullRequestState::Open
                },
                draft: params.draft,
                created_at: chrono::Utc::now(),
            });
        }

        let token = self.installation_token(installation_id).await?;
        let url = format!(
            "{}/repos/{}/{}/pulls",
            self.config.api_base, params.owner, params.repo
        );

        #[derive(serde::Serialize)]
        struct Body<'a> {
            title: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            body: Option<&'a str>,
            head: &'a str,
            base: &'a str,
            draft: bool,
        }
        #[derive(Deserialize)]
        struct Resp {
            number: i64,
            html_url: String,
            state: String,
            draft: Option<bool>,
            title: String,
            body: Option<String>,
        }

        let resp = self
            .http
            .post(&url)
            .header("Accept", "application/vnd.github+json")
            .header("Authorization", format!("Bearer {}", token.token))
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&Body {
                title: &params.title,
                body: params.body.as_deref(),
                head: &params.head,
                base: &params.base,
                draft: params.draft,
            })
            .send()
            .await
            .context("create pull request")?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            if status.as_u16() == 422 && text.contains("No commits between") {
                bail!(
                    "cannot create pull request: branch has no commits ahead of base (push your latest changes first)"
                );
            }
            bail!("create pull request failed ({status}): {text}");
        }
        let pr: Resp = serde_json::from_str(&text).context("parse pull request")?;
        Ok(PullRequest {
            id: uuid::Uuid::new_v4(),
            repository_id: uuid::Uuid::nil(),
            run_id: uuid::Uuid::nil(),
            provider_number: Some(pr.number),
            url: Some(pr.html_url),
            title: pr.title,
            body: pr.body,
            base_sha: None,
            head_sha: None,
            state: pull_request_state(&pr.state, pr.draft.unwrap_or(params.draft)),
            draft: pr.draft.unwrap_or(params.draft),
            created_at: chrono::Utc::now(),
        })
    }

    /// Mark a draft pull request ready for review (`draft: false`).
    pub async fn mark_pull_request_ready(
        &self,
        installation_id: &str,
        owner: &str,
        repo: &str,
        number: i64,
    ) -> Result<PullRequest> {
        if self.is_mock() {
            return Ok(PullRequest {
                id: uuid::Uuid::new_v4(),
                repository_id: uuid::Uuid::nil(),
                run_id: uuid::Uuid::nil(),
                provider_number: Some(number),
                url: Some(format!(
                    "https://github.com/{owner}/{repo}/pull/{number}"
                )),
                title: "Mock PR".into(),
                body: None,
                base_sha: None,
                head_sha: None,
                state: PullRequestState::Open,
                draft: false,
                created_at: chrono::Utc::now(),
            });
        }

        let token = self.installation_token(installation_id).await?;
        let url = format!(
            "{}/repos/{owner}/{repo}/pulls/{number}",
            self.config.api_base
        );

        #[derive(serde::Serialize)]
        struct Body {
            draft: bool,
        }
        #[derive(Deserialize)]
        struct Resp {
            number: i64,
            html_url: String,
            state: String,
            draft: Option<bool>,
            title: String,
            body: Option<String>,
        }

        let resp = self
            .http
            .patch(&url)
            .header("Accept", "application/vnd.github+json")
            .header("Authorization", format!("Bearer {}", token.token))
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&Body { draft: false })
            .send()
            .await
            .context("mark pull request ready")?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("mark pull request ready failed ({status}): {text}");
        }
        let pr: Resp = serde_json::from_str(&text).context("parse pull request")?;
        Ok(PullRequest {
            id: uuid::Uuid::new_v4(),
            repository_id: uuid::Uuid::nil(),
            run_id: uuid::Uuid::nil(),
            provider_number: Some(pr.number),
            url: Some(pr.html_url),
            title: pr.title,
            body: pr.body,
            base_sha: None,
            head_sha: None,
            state: pull_request_state(&pr.state, pr.draft.unwrap_or(false)),
            draft: pr.draft.unwrap_or(false),
            created_at: chrono::Utc::now(),
        })
    }

    /// Merge an open pull request.
    pub async fn merge_pull_request(
        &self,
        installation_id: &str,
        owner: &str,
        repo: &str,
        number: i64,
    ) -> Result<PullRequest> {
        if self.is_mock() {
            return Ok(PullRequest {
                id: uuid::Uuid::new_v4(),
                repository_id: uuid::Uuid::nil(),
                run_id: uuid::Uuid::nil(),
                provider_number: Some(number),
                url: Some(format!(
                    "https://github.com/{owner}/{repo}/pull/{number}"
                )),
                title: "Mock PR".into(),
                body: None,
                base_sha: None,
                head_sha: None,
                state: PullRequestState::Merged,
                draft: false,
                created_at: chrono::Utc::now(),
            });
        }

        let token = self.installation_token(installation_id).await?;
        let url = format!(
            "{}/repos/{owner}/{repo}/pulls/{number}/merge",
            self.config.api_base
        );

        #[derive(Deserialize)]
        struct Resp {
            #[allow(dead_code)]
            merged: bool,
            #[allow(dead_code)]
            message: Option<String>,
        }

        let resp = self
            .http
            .put(&url)
            .header("Accept", "application/vnd.github+json")
            .header("Authorization", format!("Bearer {}", token.token))
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&serde_json::json!({}))
            .send()
            .await
            .context("merge pull request")?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("merge pull request failed ({status}): {text}");
        }
        let _: Resp = serde_json::from_str(&text).context("parse merge response")?;

        let pr_url = format!(
            "{}/repos/{owner}/{repo}/pulls/{number}",
            self.config.api_base
        );
        let get = self
            .http
            .get(&pr_url)
            .header("Accept", "application/vnd.github+json")
            .header("Authorization", format!("Bearer {}", token.token))
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .context("fetch merged pull request")?;
        let status = get.status();
        let text = get.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("fetch merged pull request failed ({status}): {text}");
        }
        #[derive(Deserialize)]
        struct PrResp {
            number: i64,
            html_url: String,
            state: String,
            draft: Option<bool>,
            title: String,
            body: Option<String>,
            merged: Option<bool>,
        }
        let pr: PrResp = serde_json::from_str(&text).context("parse pull request")?;
        let state = if pr.merged.unwrap_or(false) || pr.state == "closed" {
            PullRequestState::Merged
        } else {
            pull_request_state(&pr.state, pr.draft.unwrap_or(false))
        };
        Ok(PullRequest {
            id: uuid::Uuid::new_v4(),
            repository_id: uuid::Uuid::nil(),
            run_id: uuid::Uuid::nil(),
            provider_number: Some(pr.number),
            url: Some(pr.html_url),
            title: pr.title,
            body: pr.body,
            base_sha: None,
            head_sha: None,
            state,
            draft: pr.draft.unwrap_or(false),
            created_at: chrono::Utc::now(),
        })
    }
}

fn github_account_type(value: &str) -> GithubAccountType {
    GithubAccountType::parse(value).unwrap_or(GithubAccountType::Organization)
}

fn pull_request_state(value: &str, draft: bool) -> PullRequestState {
    PullRequestState::parse(value).unwrap_or(if draft {
        PullRequestState::Draft
    } else {
        PullRequestState::Open
    })
}
