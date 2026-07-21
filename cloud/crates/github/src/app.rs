use anyhow::{bail, Context, Result};
use chrono::{Duration, Utc};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};

use crate::types::GithubConfig;

#[derive(Debug, Clone)]
pub struct GithubAppAuth {
    pub app_id: String,
    /// PEM-encoded RSA private key. When `None`, JWT issuance returns mock tokens.
    pub private_key_pem: Option<String>,
    pub app_slug: Option<String>,
    mock: bool,
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
            app_id: cfg
                .app_id
                .clone()
                .unwrap_or_else(|| "0".into()),
            private_key_pem: cfg.app_private_key_pem.clone(),
            app_slug: cfg.app_slug.clone(),
            mock: cfg.is_mock() || cfg.app_private_key_pem.is_none(),
        }
    }

    pub fn is_mock(&self) -> bool {
        self.mock
    }

    /// Create a GitHub App JWT (RS256). Falls back to a mock token when no key is available.
    pub fn create_app_jwt(&self) -> Result<String> {
        if self.mock || self.private_key_pem.is_none() {
            return Ok(format!("mock.app.jwt.{}", self.app_id));
        }
        let pem = self.private_key_pem.as_ref().unwrap();
        let now = Utc::now();
        let claims = AppJwtClaims {
            // GitHub recommends iat slightly in the past to account for clock skew.
            iat: (now - Duration::seconds(60)).timestamp(),
            exp: (now + Duration::minutes(9)).timestamp(),
            iss: self.app_id.clone(),
        };
        let key = EncodingKey::from_rsa_pem(pem.as_bytes())
            .context("parse GitHub App private key PEM")?;
        encode(&Header::new(Algorithm::RS256), &claims, &key)
            .context("encode GitHub App JWT")
    }

    /// Exchange App JWT for an installation access token.
    pub async fn installation_token(
        &self,
        http: &reqwest::Client,
        api_base: &str,
        installation_id: &str,
    ) -> Result<InstallationToken> {
        if installation_id.trim().is_empty() {
            bail!("installation_id is empty");
        }
        if self.mock {
            let expires = (Utc::now() + Duration::hours(1)).to_rfc3339();
            return Ok(InstallationToken {
                token: format!("ghs_mock_{installation_id}"),
                expires_at: expires,
                installation_id: installation_id.into(),
            });
        }

        let jwt = self.create_app_jwt()?;
        let url = format!("{api_base}/app/installations/{installation_id}/access_tokens");

        #[derive(Deserialize)]
        struct Resp {
            token: String,
            expires_at: String,
        }

        let resp = http
            .post(&url)
            .header("Accept", "application/vnd.github+json")
            .header("Authorization", format!("Bearer {jwt}"))
            .header("User-Agent", "zene-cloud")
            .header("X-GitHub-Api-Version", "2022-11-28")
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
}
