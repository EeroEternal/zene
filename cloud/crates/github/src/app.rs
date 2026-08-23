use anyhow::{bail, Context, Result};
use chrono::{Duration, Utc};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};

use crate::types::GithubConfig;

#[derive(Debug, Clone)]
pub struct GithubAppAuth {
    pub app_id: String,
    /// PEM-encoded RSA private key.
    pub private_key_pem: Option<String>,
    pub app_slug: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallationToken {
    pub token: String,
    pub expires_at: String,
    pub installation_id: String,
}

#[derive(Debug, Serialize)]
struct AppJwtClaims {
    iat: i64,
    exp: i64,
    iss: String,
}

impl GithubAppAuth {
    pub fn from_config(cfg: &GithubConfig) -> Self {
        Self {
            app_id: cfg.app_id.clone().unwrap_or_else(|| "0".into()),
            private_key_pem: cfg.app_private_key_pem.clone(),
            app_slug: cfg.app_slug.clone(),
        }
    }

    pub fn is_configured(&self) -> bool {
        self.private_key_pem.is_some()
    }

    /// Create a GitHub App JWT (RS256).
    pub fn create_app_jwt(&self) -> Result<String> {
        let pem = self
            .private_key_pem
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("GitHub App private key PEM is missing"))?;
        let now = Utc::now();
        let claims = AppJwtClaims {
            // GitHub recommends iat slightly in the past to account for clock skew.
            iat: (now - Duration::seconds(60)).timestamp(),
            exp: (now + Duration::minutes(9)).timestamp(),
            iss: self.app_id.clone(),
        };
        let key = EncodingKey::from_rsa_pem(pem.as_bytes())
            .context("parse GitHub App private key PEM")?;
        encode(&Header::new(Algorithm::RS256), &claims, &key).context("encode GitHub App JWT")
    }

    /// Exchange App JWT for an installation access token.
    ///
    /// When `repository_ids` is set, GitHub limits the token to those
    /// repositories and the given `permissions`. Omitting both keeps the
    /// installation-wide token (server-side listing only).
    pub async fn installation_token(
        &self,
        http: &reqwest::Client,
        api_base: &str,
        installation_id: &str,
    ) -> Result<InstallationToken> {
        self.installation_token_scoped(http, api_base, installation_id, None, None)
            .await
    }

    pub fn scoped_token_body(
        repository_ids: &[u64],
        permissions: &serde_json::Value,
    ) -> serde_json::Value {
        serde_json::json!({
            "repository_ids": repository_ids,
            "permissions": permissions,
        })
    }

    pub async fn installation_token_scoped(
        &self,
        http: &reqwest::Client,
        api_base: &str,
        installation_id: &str,
        repository_ids: Option<&[u64]>,
        permissions: Option<&serde_json::Value>,
    ) -> Result<InstallationToken> {
        if installation_id.trim().is_empty() {
            bail!("installation_id is empty");
        }

        let jwt = self.create_app_jwt()?;
        let url = format!("{api_base}/app/installations/{installation_id}/access_tokens");

        #[derive(Deserialize)]
        struct Resp {
            token: String,
            expires_at: String,
        }

        let mut req = http
            .post(&url)
            .header("Accept", "application/vnd.github+json")
            .header("Authorization", format!("Bearer {jwt}"))
            .header("User-Agent", "zene-cloud")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let (Some(ids), Some(perms)) = (repository_ids, permissions) {
            if ids.is_empty() {
                bail!("scoped installation token requires repository_ids");
            }
            req = req.json(&Self::scoped_token_body(ids, perms));
        }
        let resp = req
            .send()
            .await
            .context("create installation token request")?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("create installation token failed ({status}): {text}");
        }
        let body: Resp = serde_json::from_str(&text).context("parse installation token")?;
        Ok(InstallationToken {
            token: body.token,
            expires_at: body.expires_at,
            installation_id: installation_id.into(),
        })
    }

    pub fn install_url(&self) -> Option<String> {
        self.app_slug
            .as_ref()
            .map(|slug| format!("https://github.com/apps/{slug}/installations/new"))
    }

    pub fn install_url_with_state(&self, state: &str) -> Option<String> {
        self.app_slug.as_ref().map(|slug| {
            format!(
                "https://github.com/apps/{slug}/installations/new?state={}",
                urlencoding(state)
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::GithubAppAuth;

    #[test]
    fn scoped_token_body_includes_repository_ids_and_permissions() {
        let body = GithubAppAuth::scoped_token_body(
            &[1296269],
            &serde_json::json!({ "contents": "read", "metadata": "read" }),
        );
        assert_eq!(body["repository_ids"][0], 1296269);
        assert_eq!(body["permissions"]["contents"], "read");
    }
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
