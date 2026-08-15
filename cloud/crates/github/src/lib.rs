//! GitHub OAuth + GitHub App helpers for Zene Cloud.
//!
//! Live OAuth + GitHub App; credentials from env or Settings UI (DB).

mod app;
mod client;
mod oauth;
mod types;

pub use app::{GithubAppAuth, InstallationToken};
pub use client::{AppInstallation, GithubClient};
pub use oauth::{new_oauth_state, OauthConfig, OauthTokens};
pub use types::{CreatePullRequestParams, GithubApiError, GithubConfig, ListedRepo};

use anyhow::Result;
use zene_cloud_domain::GithubMode;

/// Build a client from environment (`GITHUB_*` related vars).
pub fn from_env() -> Result<GithubClient> {
    let config = GithubConfig::from_env()?;
    Ok(GithubClient::new(config))
}

pub fn mode_from_env() -> GithubMode {
    GithubMode::from_env()
}
