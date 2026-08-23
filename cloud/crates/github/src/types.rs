use anyhow::Result;
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
        let client_id = std::env::var("GITHUB_CLIENT_ID")
            .ok()
            .filter(|s| !s.is_empty());
        let client_secret = std::env::var("GITHUB_CLIENT_SECRET")
            .ok()
            .filter(|s| !s.is_empty());
        let app_id = std::env::var("GITHUB_APP_ID")
            .ok()
            .filter(|s| !s.is_empty());
        let app_slug = std::env::var("GITHUB_APP_SLUG")
            .ok()
            .filter(|s| !s.is_empty());
        let app_private_key_pem = match std::env::var("GITHUB_APP_PRIVATE_KEY_PATH") {
            Ok(path) if !path.is_empty() => {
                let pem = std::fs::read_to_string(&path).map_err(|e| {
                    anyhow::anyhow!("read GITHUB_APP_PRIVATE_KEY_PATH ({path}): {e}")
                })?;
                Some(pem)
            }
            _ => std::env::var("GITHUB_APP_PRIVATE_KEY")
                .ok()
                .filter(|s| !s.is_empty()),
        };

        if mode == GithubMode::Live {
            // Credentials may come from Settings UI (DB) at runtime; env is optional override.
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

    pub fn live_default() -> Self {
        Self {
            mode: GithubMode::Live,
            client_id: None,
            client_secret: None,
            app_id: None,
            app_private_key_pem: None,
            app_slug: None,
            api_base: "https://api.github.com".into(),
            oauth_authorize_url: "https://github.com/login/oauth/authorize".into(),
            oauth_token_url: "https://github.com/login/oauth/access_token".into(),
        }
    }

    pub fn merge_env_and_db(stored: Option<zene_cloud_domain::GithubProviderConfig>) -> Self {
        let mut cfg = Self::live_default();
        if let Some(stored) = stored {
            cfg.mode = stored.mode;
            if stored.client_id.is_some() {
                cfg.client_id = stored.client_id;
            }
            if stored.client_secret.is_some() {
                cfg.client_secret = stored.client_secret;
            }
            if stored.app_id.is_some() {
                cfg.app_id = stored.app_id;
            }
            if stored.app_private_key.is_some() {
                cfg.app_private_key_pem = stored.app_private_key;
            }
            if stored.app_slug.is_some() {
                cfg.app_slug = stored.app_slug;
            }
        }

        if let Ok(v) = std::env::var("GITHUB_CLIENT_ID") {
            if !v.is_empty() {
                cfg.client_id = Some(v);
            }
        }
        if let Ok(v) = std::env::var("GITHUB_CLIENT_SECRET") {
            if !v.is_empty() {
                cfg.client_secret = Some(v);
            }
        }
        if let Ok(v) = std::env::var("GITHUB_APP_ID") {
            if !v.is_empty() {
                cfg.app_id = Some(v);
            }
        }
        if let Ok(v) = std::env::var("GITHUB_APP_SLUG") {
            if !v.is_empty() {
                cfg.app_slug = Some(v);
            }
        }
        if cfg.app_private_key_pem.is_none() {
            if let Ok(path) = std::env::var("GITHUB_APP_PRIVATE_KEY_PATH") {
                if !path.is_empty() {
                    if let Ok(pem) = std::fs::read_to_string(&path) {
                        cfg.app_private_key_pem = Some(pem);
                    }
                }
            } else if let Ok(pem) = std::env::var("GITHUB_APP_PRIVATE_KEY") {
                if !pem.is_empty() {
                    cfg.app_private_key_pem = Some(pem);
                }
            }
        }

        cfg
    }

    pub fn is_configured(&self) -> bool {
        self.is_app_configured()
    }

    pub fn is_app_configured(&self) -> bool {
        self.app_id.as_ref().is_some_and(|s| !s.is_empty())
            && self
                .app_private_key_pem
                .as_ref()
                .is_some_and(|s| !s.is_empty())
            && self.app_slug.as_ref().is_some_and(|s| !s.is_empty())
    }

    pub fn is_oauth_configured(&self) -> bool {
        self.client_id.as_ref().is_some_and(|s| !s.is_empty())
            && self.client_secret.as_ref().is_some_and(|s| !s.is_empty())
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
