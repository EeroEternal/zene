use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result};
use tracing::{info, warn};
use uuid::Uuid;
use zene_cloud_domain::{WorkerFence, WorkerTitleRequest};

pub(crate) fn chat_completions_url(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else {
        // Presets already include the API root (/v1, /compatible-mode/v1, /paas/v4, …).
        format!("{base}/chat/completions")
    }
}

const TITLE_SYSTEM: &str = "Write a short agent session title in the user's language. \
2–8 words, a topic label or noun phrase. Name the subject and the work. \
Do not copy the user's wording. Do not write a question. \
No quotes, markdown, or trailing punctuation. Return only the title.";

const TITLE_REWRITE_SYSTEM: &str = "The last title copied the user's request or was a question. \
Rewrite it as a short topic label (noun phrase) in the user's language, 2–8 words. \
Example: 'sglang 目前性能怎么样' → 'SGLang 性能分析'. \
No questions, quotes, or punctuation wrapping. Return only the title.";

pub(crate) fn sanitize_run_title(raw: &str) -> String {
    let cleaned = raw
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`' || c == '「' || c == '」')
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .trim_start_matches(['#', '-', '*', ' '])
        .trim();
    cleaned.chars().take(56).collect()
}

fn normalize_title_text(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub(crate) fn title_looks_like_question(title: &str) -> bool {
    let trimmed = title.trim();
    if trimmed.ends_with('?') || trimmed.ends_with('？') {
        return true;
    }
    let lower = trimmed.to_lowercase();
    lower.contains("怎么样")
        || lower.contains("如何")
        || lower.contains("怎么")
        || lower.starts_with("what ")
        || lower.starts_with("what's ")
        || lower.starts_with("how ")
        || lower.starts_with("why ")
        || lower.starts_with("can you")
        || lower.starts_with("could you")
}

pub(crate) fn title_echoes_source(title: &str, sources: &[&str]) -> bool {
    let title_n = normalize_title_text(title);
    if title_n.is_empty() {
        return true;
    }
    sources.iter().any(|source| {
        zene_cloud_domain::title_is_prompt_echo(title, source) || {
            let source_n = normalize_title_text(source);
            !source_n.is_empty() && title_n == source_n
        }
    })
}

pub(crate) fn title_needs_rewrite(title: &str, sources: &[&str]) -> bool {
    title_echoes_source(title, sources)
        || title_looks_like_question(title)
        || title.trim_start().starts_with("请用")
        || title.trim_start().starts_with("请帮")
        || title.trim_start().starts_with("请深入")
        || title.trim_start().starts_with("帮我")
}

pub(crate) fn title_from_chat_response(value: &serde_json::Value) -> String {
    let message = value.pointer("/choices/0/message");
    let content = message
        .and_then(|m| m.get("content"))
        .and_then(|content| match content {
            serde_json::Value::String(text) => Some(text.clone()),
            serde_json::Value::Array(parts) => {
                let text = parts
                    .iter()
                    .filter_map(|part| {
                        part.get("text")
                            .and_then(|t| t.as_str())
                            .or_else(|| part.as_str())
                    })
                    .collect::<Vec<_>>()
                    .join("");
                (!text.is_empty()).then_some(text)
            }
            _ => None,
        })
        .or_else(|| {
            value
                .pointer("/choices/0/text")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .unwrap_or_default();
    sanitize_run_title(&content)
}

pub(crate) struct TitleRefresh {
    pub(crate) seed: String,
    pub(crate) original_prompt: String,
    pub(crate) llm_env: Option<HashMap<String, String>>,
    pub(crate) recent: Vec<String>,
    pub(crate) last_auto: Option<String>,
}

pub(crate) fn format_title_focus(original: &str, recent: &[String]) -> String {
    let orig: String = original.chars().take(400).collect();
    if recent.is_empty() {
        return format!("Original task:\n{orig}");
    }
    let mut out = format!("Original task:\n{orig}\n\nRecent user requests:");
    for (i, turn) in recent.iter().take(5).enumerate() {
        let snippet: String = turn.chars().take(240).collect();
        out.push_str(&format!("\n{}. {snippet}", i + 1));
    }
    out
}

pub(crate) fn title_is_user_locked(current: &str, last_auto: Option<&str>, seed: &str) -> bool {
    let cur = current.trim();
    if cur.is_empty() || cur == seed {
        return false;
    }
    match last_auto {
        Some(auto) => cur != auto,
        None => true,
    }
}

fn title_chat_body(model: &str, snippet: &str, rewrite: bool) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "temperature": 0.2,
        "max_tokens": 64,
        "thinking": { "type": "disabled" },
        "messages": [
            {
                "role": "system",
                "content": if rewrite { TITLE_REWRITE_SYSTEM } else { TITLE_SYSTEM }
            },
            {
                "role": "user",
                "content": snippet
            }
        ]
    })
}

async fn request_run_title(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    model: &str,
    snippet: &str,
    rewrite: bool,
) -> Result<Option<String>> {
    let mut body = title_chat_body(model, snippet, rewrite);
    let mut resp = client
        .post(url)
        .bearer_auth(api_key)
        .json(&body)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .context("title llm request")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let err_body = resp.text().await.unwrap_or_default();
        // Providers that reject unknown `thinking` get one retry without it.
        let thinking_rejected = status.as_u16() == 400
            && (err_body.contains("thinking")
                || err_body.contains("unknown")
                || err_body.contains("Unrecognized"));
        if thinking_rejected {
            body.as_object_mut().map(|o| o.remove("thinking"));
            // Give thinking models enough room if disable isn't supported.
            body["max_tokens"] = serde_json::json!(256);
            resp = client
                .post(url)
                .bearer_auth(api_key)
                .json(&body)
                .timeout(Duration::from_secs(20))
                .send()
                .await
                .context("title llm retry")?;
        } else {
            warn!(%status, body = %err_body.chars().take(240).collect::<String>(), "title llm failed");
            return Ok(None);
        }
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let err_body = resp.text().await.unwrap_or_default();
        warn!(%status, body = %err_body.chars().take(240).collect::<String>(), "title llm failed");
        return Ok(None);
    }
    let value: serde_json::Value = resp.json().await.context("title llm json")?;
    let title = title_from_chat_response(&value);
    if title.is_empty() {
        warn!("title llm returned empty content");
        return Ok(None);
    }
    Ok(Some(title))
}

pub(crate) async fn maybe_refresh_run_title(
    client: &reqwest::Client,
    api_url: &str,
    worker_token: &str,
    run_id: Uuid,
    fence: &WorkerFence,
    focus: &str,
    current_title: Option<&str>,
    source_prompt: &str,
    llm_env: &HashMap<String, String>,
) -> Result<Option<String>> {
    let api_key = llm_env
        .get("ZENE_API_KEY")
        .cloned()
        .or_else(|| llm_env.get("OPENAI_API_KEY").cloned())
        .or_else(|| std::env::var("ZENE_API_KEY").ok())
        .or_else(|| std::env::var("OPENAI_API_KEY").ok())
        .unwrap_or_default();
    let base_url = llm_env
        .get("ZENE_BASE_URL")
        .cloned()
        .or_else(|| std::env::var("ZENE_BASE_URL").ok())
        .unwrap_or_else(|| "https://api.openai.com/v1".into());
    let model = llm_env
        .get("ZENE_MODEL")
        .cloned()
        .or_else(|| std::env::var("ZENE_MODEL").ok())
        .unwrap_or_else(|| "gpt-4o-mini".into());
    if api_key.trim().is_empty() {
        return Ok(None);
    }
    let url = chat_completions_url(&base_url);
    let snippet: String = focus.chars().take(1200).collect();
    let sources = [source_prompt, current_title.unwrap_or("")];
    let mut title = match request_run_title(client, &url, &api_key, &model, &snippet, false).await?
    {
        Some(title) => title,
        None => zene_cloud_domain::summarize_prompt_title(source_prompt),
    };
    if title_needs_rewrite(&title, &sources) {
        if let Some(rewritten) =
            request_run_title(client, &url, &api_key, &model, &snippet, true).await?
        {
            if !title_needs_rewrite(&rewritten, &sources) {
                title = rewritten;
            } else {
                title = zene_cloud_domain::summarize_prompt_title(source_prompt);
            }
        } else {
            title = zene_cloud_domain::summarize_prompt_title(source_prompt);
        }
    }
    if title_needs_rewrite(&title, &sources) {
        warn!(run_id = %run_id, echo = %title, "title still echoed the prompt");
        return Ok(None);
    }
    if current_title.is_some_and(|cur| cur.trim() == title) {
        return Ok(Some(title));
    }
    let req = WorkerTitleRequest {
        title: title.clone(),
        fence: Some(fence.clone()),
    };
    client
        .post(format!("{api_url}/internal/v1/runs/{run_id}/title"))
        .bearer_auth(worker_token)
        .json(&req)
        .send()
        .await?
        .error_for_status()
        .context("post title")?;
    info!(run_id = %run_id, %title, "refreshed run title");
    Ok(Some(title))
}
