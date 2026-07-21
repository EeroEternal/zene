use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use zene_cloud_domain::GithubMode;

#[derive(Debug, Clone)]
pub struct GithubConfig {
    pub mode: GithubMode,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub app_id: Option<String>,
    pub app_private_key_pem: Option<String>,
    pub app_slug: Option<String>,
    pub api_base: String,
    pub oauth_authorize_url: String,
    pub oauth_token_url: String,
}

impl GithubConfig {
    pub fn from_env() -> Result<Self> {
        let mode = GithubMode::from_env();
        let client_id = std::env::var("GITHUB_CLIENT_ID").ok().filter(|s| !s.is_empty());
        let client_secret = std::env::var("GITHUB_CLIENT_SECRET")
            .ok()
            .filter(|s| !s.is_empty());
        let app_id = std::env::var("GITHUB_APP_ID").ok().filter(|s| !s.is_empty());
        let app_slug = std::env::var("GITHUB_APP_SLUG").ok().filter(|s| !s.is_empty());
        let app_private_key_pem = match std::env::var("GITHUB_APP_PRIVATE_KEY_PATH") {
            Ok(path) if !path.is_empty() => {
                let pem = std::fs::read_to_string(&path)
                    .map_err(|e| anyhow::anyhow!("read GITHUB_APP_PRIVATE_KEY_PATH ({path}): {e}"))?;
                Some(pem)
            }
            _ => std::env::var("GITHUB_APP_PRIVATE_KEY")
                .ok()
                .filter(|s| !s.is_empty()),
        };

        if mode == GithubMode::Live {
            if client_id.is_none() || client_secret.is_none() {
                bail!(
                    "live GitHub mode requires GITHUB_CLIENT_ID and GITHUB_CLIENT_SECRET"
                );
            }
        }

        Ok(Self {
            mode,
            client_id,
            client_secret,
            app_id,
            app_private_key_pem,
            app_slug,
            api_base: std::env::var("GITHUB_API_BASE")
                .unwrap_or_else(|_| "https://api.github.com".into()),
            oauth_authorize_url: std::env::var("GITHUB_OAUTH_AUTHORIZE_URL")
                .unwrap_or_else(|_| "https://github.com/login/oauth/authorize".into()),
            oauth_token_url: std::env::var("GITHUB_OAUTH_TOKEN_URL")
                .unwrap_or_else(|_| "https://github.com/login/oauth/access_token".into()),
        })
    }

    pub fn mock() -> Self {
        Self {
            mode: GithubMode::Mock,
            client_id: Some("mock-client-id".into()),
            client_secret: Some("mock-client-secret".into()),
            app_id: Some("123456".into()),
            app_private_key_pem: None,
            app_slug: Some("zene-cloud-mock".into()),
            api_base: "https://api.github.com".into(),
            oauth_authorize_url: "https://github.com/login/oauth/authorize".into(),
            oauth_token_url: "https://github.com/login/oauth/access_token".into(),
        }
    }

    pub fn is_mock(&self) -> bool {
        self.mode == GithubMode::Mock
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListedRepo {
    pub id: i64,
    pub full_name: String,
    pub name: String,
    pub owner_login: String,
    pub default_branch: String,
    pub clone_url: String,
    pub private: bool,
}

#[derive(Debug, Clone)]
pub struct CreatePullRequestParams {
    pub owner: String,
    pub repo: String,
    pub title: String,
    pub body: Option<String>,
    pub head: String,
    pub base: String,
    pub draft: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum GithubApiError {
    #[error("github api error: {0}")]
    Api(String),
    #[error("github config error: {0}")]
    Config(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
