use anyhow::{bail, Context, Result};
use serde::Serialize;

#[derive(Debug, Clone)]
pub struct EmailConfig {
    pub api_key: Option<String>,
    pub from: String,
}

impl EmailConfig {
    pub fn from_env() -> Self {
        Self {
            api_key: std::env::var("RESEND_API_KEY")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            from: std::env::var("RESEND_FROM")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Zene <noreply@zene.run>".into()),
        }
    }

    pub fn configured(&self) -> bool {
        self.api_key.is_some()
    }
}

#[derive(Serialize)]
struct ResendEmail<'a> {
    from: &'a str,
    to: Vec<&'a str>,
    subject: &'a str,
    html: &'a str,
    text: &'a str,
}

pub async fn send_signin_email(cfg: &EmailConfig, to: &str, login_url: &str) -> Result<()> {
    let Some(api_key) = cfg.api_key.as_deref() else {
        bail!("RESEND_API_KEY is not set");
    };
    let html = signin_html(login_url);
    let text = signin_text(login_url);
    let client = reqwest::Client::builder()
        .user_agent("zene-cloud")
        .build()
        .context("resend http client")?;
    let response = client
        .post("https://api.resend.com/emails")
        .bearer_auth(api_key)
        .json(&ResendEmail {
            from: &cfg.from,
            to: vec![to],
            subject: "Sign in to Zene",
            html: &html,
            text: &text,
        })
        .send()
        .await
        .context("send resend request")?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        tracing::error!(%status, %body, "resend rejected sign-in email");
        bail!("failed to send sign-in email");
    }
    Ok(())
}

fn signin_text(login_url: &str) -> String {
    format!(
        "Sign in to Zene Cloud Console.\n\n{login_url}\n\nThis link expires in 15 minutes. Ignore this email if you did not request it.\n"
    )
}

fn signin_html(login_url: &str) -> String {
    let href = escape(login_url);
    format!(
        r#"<div style="font-family:Inter,ui-sans-serif,sans-serif;color:#2E3436;font-size:14px;line-height:1.5">
<p>Sign in to Zene Cloud Console.</p>
<p><a href="{href}" style="color:#3584E4">Open sign-in link</a></p>
<p style="color:#687174;font-size:12px">This link expires in 15 minutes. Ignore this email if you did not request it.</p>
</div>"#
    )
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
