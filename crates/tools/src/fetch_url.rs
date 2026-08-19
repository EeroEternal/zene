use anyhow::{Context, Result};
use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;
use serde_json::json;
use zene_llm::ToolDefinition;

use crate::registry::{Tool, ToolContext, ToolResult};

const FETCH_TIMEOUT_SECS: u64 = 30;
const MAX_BODY_BYTES: usize = 100 * 1024;

#[derive(Debug, Deserialize)]
struct FetchUrlArgs {
    url: String,
}

pub struct FetchUrlTool;

#[async_trait]
impl Tool for FetchUrlTool {
    fn name(&self) -> &str {
        "FetchUrl"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "FetchUrl".to_string(),
            description: "Fetch a URL over HTTP GET and return plain text content (HTML is stripped). Body is capped at 100KB.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to fetch."
                    }
                },
                "required": ["url"]
            }),
        }
    }

    async fn execute(&self, arguments: &str, ctx: &ToolContext) -> Result<ToolResult> {
        let args: FetchUrlArgs = serde_json::from_str(arguments).context("parse FetchUrl args")?;
        if args.url.trim().is_empty() {
            return Ok(ToolResult {
                content: "FetchUrl requires a non-empty `url`.".to_string(),
                is_error: true,
            });
        }

        if let Some(cancel) = &ctx.cancel {
            if cancel.is_cancelled() {
                return Ok(ToolResult {
                    content: "Fetch cancelled.".to_string(),
                    is_error: true,
                });
            }
        }

        if let Err(err) = ctx.sandbox.authorize_egress(&args.url).await {
            return Ok(ToolResult {
                content: format!("FetchUrl blocked by sandbox: {err}"),
                is_error: true,
            });
        }

        match fetch_url_text(&args.url).await {
            Ok(text) => {
                if text.trim().is_empty() {
                    Ok(ToolResult {
                        content: "The response body is empty.".to_string(),
                        is_error: false,
                    })
                } else {
                    Ok(ToolResult {
                        content: text,
                        is_error: false,
                    })
                }
            }
            Err(err) => Ok(ToolResult {
                content: format!("Failed to fetch URL: {err}"),
                is_error: true,
            }),
        }
    }
}

async fn fetch_url_text(url: &str) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .context("build HTTP client")?;

    let response = client
        .get(url)
        .header(
            reqwest::header::USER_AGENT,
            "Zene/0.1 (+https://github.com/zene)",
        )
        .send()
        .await
        .with_context(|| format!("network error fetching {url}"))?;

    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("HTTP status {}", status.as_u16());
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let bytes = response.bytes().await.context("read response body")?;
    let truncated = if bytes.len() > MAX_BODY_BYTES {
        &bytes[..MAX_BODY_BYTES]
    } else {
        &bytes
    };

    let body = String::from_utf8_lossy(truncated).into_owned();

    let text = if content_type.contains("html") || looks_like_html(&body) {
        html_to_text(&body)
    } else {
        body
    };

    Ok(text.trim().to_string())
}

fn looks_like_html(body: &str) -> bool {
    let trimmed = body.trim_start();
    trimmed.starts_with("<!DOCTYPE")
        || trimmed.starts_with("<html")
        || trimmed.starts_with("<HTML")
        || trimmed.contains("<body")
        || trimmed.contains("<BODY")
}

pub fn html_to_text(html: &str) -> String {
    let script_re = Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap();
    let style_re = Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap();
    let tag_re = Regex::new(r"(?s)<[^>]+>").unwrap();
    let ws_re = Regex::new(r"[ \t\f\v\u{00a0}]+").unwrap();
    let blank_re = Regex::new(r"\n{3,}").unwrap();

    let mut text = script_re.replace_all(html, "\n").into_owned();
    text = style_re.replace_all(&text, "\n").into_owned();
    text = text
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("</p>", "\n")
        .replace("</div>", "\n")
        .replace("</li>", "\n");
    text = tag_re.replace_all(&text, " ").into_owned();
    text = decode_basic_entities(&text);
    text = ws_re.replace_all(&text, " ").into_owned();
    text = blank_re.replace_all(&text, "\n\n").into_owned();
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn decode_basic_entities(input: &str) -> String {
    input
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_to_text_strips_tags() {
        let html = "<html><body><h1>Title</h1><p>Hello <b>world</b></p></body></html>";
        let text = html_to_text(html);
        assert!(text.contains("Title"));
        assert!(text.contains("Hello world"));
        assert!(!text.contains("<"));
    }

    #[test]
    fn html_to_text_removes_script() {
        let html = "<div>Visible<script>alert(1)</script></div>";
        let text = html_to_text(html);
        assert!(text.contains("Visible"));
        assert!(!text.contains("alert"));
    }
}
