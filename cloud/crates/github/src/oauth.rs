use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::GithubConfig;

#[derive(Debug, Clone)]
pub struct OauthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub authorize_url: String,
    pub token_url: String,
    pub redirect_uri: Option<String>,
    pub scopes: Vec<String>,
}

impl OauthConfig {
    pub fn from_github_config(cfg: &GithubConfig, redirect_uri: Option<String>) -> Result<Self> {
        let client_id = cfg
            .client_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("GitHub OAuth client_id is not configured"))?;
        let client_secret = cfg
            .client_secret
            .clone()
            .ok_or_else(|| anyhow::anyhow!("GitHub OAuth client_secret is not configured"))?;
        Ok(Self {
            client_id,
            client_secret,
            authorize_url: cfg.oauth_authorize_url.clone(),
            token_url: cfg.oauth_token_url.clone(),
            redirect_uri,
            scopes: vec!["read:user".into(), "user:email".into()],
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OauthTokens {
    pub access_token: String,
    pub token_type: String,
    pub scope: Option<String>,
}

/// Build the GitHub OAuth authorize URL and a CSRF `state` value.
pub fn build_authorize_url(cfg: &OauthConfig, state: &str) -> String {
    let mut url = format!(
        "{}?client_id={}&state={}&response_type=code",
        cfg.authorize_url,
        urlencoding(&cfg.client_id),
        urlencoding(state)
    );
    if !cfg.scopes.is_empty() {
        url.push_str(&format!("&scope={}", urlencoding(&cfg.scopes.join(" "))));
    }
    if let Some(redirect) = &cfg.redirect_uri {
        url.push_str(&format!("&redirect_uri={}", urlencoding(redirect)));
    }
    url
}

pub fn new_oauth_state() -> String {
    format!("gh_{}", Uuid::new_v4().simple())
}

/// Exchange an authorization code for tokens.
pub async fn exchange_code(
    cfg: &OauthConfig,
    http: &reqwest::Client,
    code: &str,
) -> Result<OauthTokens> {
    if code.trim().is_empty() {
        bail!("oauth code is empty");
    }

    #[derive(Serialize)]
    struct Body<'a> {
        client_id: &'a str,
        client_secret: &'a str,
        code: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        redirect_uri: Option<&'a str>,
    }

    #[derive(Deserialize)]
    struct TokenResponse {
        access_token: String,
        token_type: Option<String>,
        scope: Option<String>,
        error: Option<String>,
        error_description: Option<String>,
    }

    let resp = http
        .post(&cfg.token_url)
        .header("Accept", "application/json")
        .json(&Body {
            client_id: &cfg.client_id,
            client_secret: &cfg.client_secret,
            code,
            redirect_uri: cfg.redirect_uri.as_deref(),
        })
        .send()
        .await
        .context("oauth token exchange request")?;

    let status = resp.status();
    let body: TokenResponse = resp.json().await.context("oauth token exchange json")?;
    if !status.is_success() || body.error.is_some() {
        bail!(
            "oauth token exchange failed: {} {}",
            body.error.unwrap_or_else(|| status.to_string()),
            body.error_description.unwrap_or_default()
        );
    }
    Ok(OauthTokens {
        access_token: body.access_token,
        token_type: body.token_type.unwrap_or_else(|| "bearer".into()),
        scope: body.scope,
    })
}

fn urlencoding(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_url_includes_state() {
        let cfg = OauthConfig {
            client_id: "cid".into(),
            client_secret: "sec".into(),
            authorize_url: "https://github.com/login/oauth/authorize".into(),
            token_url: "https://github.com/login/oauth/access_token".into(),
            redirect_uri: Some("http://localhost/cb".into()),
            scopes: vec!["read:user".into()],
        };
        let url = build_authorize_url(&cfg, "state123");
        assert!(url.contains("client_id=cid"));
        assert!(url.contains("state=state123"));
        assert!(url.contains("redirect_uri="));
    }
}
