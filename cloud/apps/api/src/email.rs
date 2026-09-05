use anyhow::{bail, Context, Result};
use serde::Serialize;

#[derive(Debug, Clone)]
pub enum EmailProvider {
    /// Cloudflare Email Sending API (`POST /accounts/{account_id}/email/sending/send`)
    Cloudflare {
        token: String,
        account_id: String,
        from: String,
    },
    /// Resend Email API (`POST https://api.resend.com/emails`)
    Resend {
        api_key: String,
        from: String,
    },
}

#[derive(Debug, Clone)]
pub struct EmailConfig {
    pub provider: Option<EmailProvider>,
}

impl EmailConfig {
    pub fn from_env() -> Self {
        // Priority 1: Cloudflare Email Sending
        // Supports CLOUDFLARE_MAIL_TOKEN or CLOUDFLARE_EMAIL_TOKEN or CLOUDFLARE_API_TOKEN
        let cf_token = std::env::var("CLOUDFLARE_MAIL_TOKEN")
            .or_else(|_| std::env::var("CLOUDFLARE_EMAIL_TOKEN"))
            .or_else(|_| std::env::var("CLOUDFLARE_API_TOKEN"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let cf_account_id = std::env::var("CLOUDFLARE_ACCOUNT_ID")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        if let (Some(token), Some(account_id)) = (cf_token, cf_account_id) {
            let from = std::env::var("CLOUDFLARE_EMAIL_FROM")
                .or_else(|_| std::env::var("EMAIL_FROM"))
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Zene <noreply@zene.run>".into());
            return Self {
                provider: Some(EmailProvider::Cloudflare {
                    token,
                    account_id,
                    from,
                }),
            };
        }

        // Priority 2: Resend (fallback)
        let resend_key = std::env::var("RESEND_API_KEY")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        if let Some(api_key) = resend_key {
            let from = std::env::var("RESEND_FROM")
                .or_else(|_| std::env::var("EMAIL_FROM"))
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Zene <noreply@zene.run>".into());
            return Self {
                provider: Some(EmailProvider::Resend { api_key, from }),
            };
        }

        Self { provider: None }
    }

    pub fn configured(&self) -> bool {
        self.provider.is_some()
    }
}

#[derive(Serialize)]
struct CloudflareSendEmailRequest<'a> {
    from: &'a str,
    to: Vec<&'a str>,
    subject: &'a str,
    text: &'a str,
    html: &'a str,
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
    let Some(provider) = &cfg.provider else {
        bail!("Email service is not configured (neither CLOUDFLARE_MAIL_TOKEN nor RESEND_API_KEY is set)");
    };

    let html = signin_html(login_url);
    let text = signin_text(login_url);
    let client = reqwest::Client::builder()
        .user_agent("zene-cloud")
        .build()
        .context("http client for email sending")?;

    match provider {
        EmailProvider::Cloudflare {
            token,
            account_id,
            from,
        } => {
            let url = format!(
                "https://api.cloudflare.com/client/v4/accounts/{account_id}/email/sending/send"
            );
            let payload = CloudflareSendEmailRequest {
                from,
                to: vec![to],
                subject: "Sign in to Zene",
                text: &text,
                html: &html,
            };

            let response = client
                .post(&url)
                .bearer_auth(token)
                .json(&payload)
                .send()
                .await
                .context("send cloudflare email request")?;

            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            if !status.is_success() {
                tracing::error!(%status, %body, "cloudflare email sending rejected request");
                bail!("failed to send sign-in email via cloudflare email service: {body}");
            }
            tracing::info!(to = %to, "sign-in email sent via Cloudflare Email Service");
            Ok(())
        }
        EmailProvider::Resend { api_key, from } => {
            let response = client
                .post("https://api.resend.com/emails")
                .bearer_auth(api_key)
                .json(&ResendEmail {
                    from,
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
                bail!("failed to send sign-in email via resend");
            }
            tracing::info!(to = %to, "sign-in email sent via Resend");
            Ok(())
        }
    }
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

pub async fn send_verification_code_email(
    cfg: &EmailConfig,
    to: &str,
    code: &str,
    purpose: &str,
) -> Result<()> {
    let Some(provider) = &cfg.provider else {
        bail!("Email service is not configured (neither CLOUDFLARE_MAIL_TOKEN nor RESEND_API_KEY is set)");
    };

    let (subject, action_desc) = if purpose == "reset_password" {
        (format!("Your Zene password reset code: {code}"), "reset your password")
    } else {
        (format!("Your Zene verification code: {code}"), "complete your registration")
    };

    let html = verification_code_html(code, action_desc);
    let text = verification_code_text(code, action_desc);
    let client = reqwest::Client::builder()
        .user_agent("zene-cloud")
        .build()
        .context("http client for email sending")?;

    match provider {
        EmailProvider::Cloudflare {
            token,
            account_id,
            from,
        } => {
            let url = format!(
                "https://api.cloudflare.com/client/v4/accounts/{account_id}/email/sending/send"
            );
            let payload = CloudflareSendEmailRequest {
                from,
                to: vec![to],
                subject: &subject,
                text: &text,
                html: &html,
            };

            let response = client
                .post(&url)
                .bearer_auth(token)
                .json(&payload)
                .send()
                .await
                .context("send cloudflare email request")?;

            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            if !status.is_success() {
                tracing::error!(%status, %body, "cloudflare email sending rejected request");
                bail!("failed to send verification code email via cloudflare email service: {body}");
            }
            tracing::info!(to = %to, "verification code email sent via Cloudflare Email Service");
            Ok(())
        }
        EmailProvider::Resend { api_key, from } => {
            let response = client
                .post("https://api.resend.com/emails")
                .bearer_auth(api_key)
                .json(&ResendEmail {
                    from,
                    to: vec![to],
                    subject: &subject,
                    html: &html,
                    text: &text,
                })
                .send()
                .await
                .context("send resend request")?;

            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            if !status.is_success() {
                tracing::error!(%status, %body, "resend rejected verification code email");
                bail!("failed to send verification code email via resend");
            }
            tracing::info!(to = %to, "verification code email sent via Resend");
            Ok(())
        }
    }
}

fn verification_code_text(code: &str, action_desc: &str) -> String {
    format!(
        "Your Zene verification code is:\n\n  {code}\n\nUse this code to {action_desc}. It expires in 10 minutes.\nIf you did not request this code, please ignore this email.\n"
    )
}

fn verification_code_html(code: &str, action_desc: &str) -> String {
    let escaped_code = escape(code);
    let escaped_desc = escape(action_desc);
    format!(
        r#"<!DOCTYPE html>
<html>
<head><meta charset="utf-8"/></head>
<body style="margin:0;padding:24px;background-color:#fafafa;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif;color:#18181b">
  <div style="max-width:480px;margin:0 auto;background:#ffffff;border:1px solid #e4e4e7;border-radius:12px;padding:32px;box-shadow:0 1px 3px rgba(0,0,0,0.05)">
    <div style="display:flex;align-items:center;margin-bottom:24px">
      <div style="background:#2744A5;color:#ffffff;font-weight:700;font-size:16px;width:32px;height:32px;line-height:32px;text-align:center;border-radius:6px;font-family:monospace;display:inline-block">Z</div>
      <span style="margin-left:10px;font-size:16px;font-weight:600;color:#09090b">Zene Cloud</span>
    </div>
    <h2 style="margin:0 0 12px;font-size:20px;font-weight:600;color:#09090b">Your verification code</h2>
    <p style="margin:0 0 20px;font-size:14px;line-height:1.6;color:#52525b">
      Please enter the following 6-digit code to {escaped_desc}:
    </p>
    <div style="background:#f4f4f5;border:1px solid #e4e4e7;border-radius:8px;padding:16px;text-align:center;margin:24px 0">
      <span style="font-family:ui-monospace,SFMono-Regular,Menlo,Monaco,Consolas,monospace;font-size:32px;font-weight:700;letter-spacing:6px;color:#09090b;display:inline-block">{escaped_code}</span>
    </div>
    <p style="margin:0 0 16px;font-size:12px;line-height:1.5;color:#71717a">
      This code will expire in 10 minutes. If you did not request this email, no further action is required.
    </p>
    <div style="margin-top:28px;padding-top:16px;border-top:1px solid #f4f4f5;font-size:11px;color:#a1a1aa;text-align:center">
      Zene Autonomous Agent Workbench
    </div>
  </div>
</body>
</html>"#
    )
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_config_cloudflare_precedence() {
        std::env::set_var("CLOUDFLARE_MAIL_TOKEN", "token_123");
        std::env::set_var("CLOUDFLARE_ACCOUNT_ID", "acc_123");
        let cfg = EmailConfig::from_env();
        assert!(cfg.configured());
        std::env::remove_var("CLOUDFLARE_MAIL_TOKEN");
        std::env::remove_var("CLOUDFLARE_ACCOUNT_ID");
    }
}
