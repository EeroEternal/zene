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

const TITLE_SYSTEM: &str = "Write a short title for this task in the user's language. \
2–8 words, a topic label or noun phrase. Name the subject and the work. \
Do not copy the user's wording. Do not write a question. \
Do not include prefixes like 'Title:' or 'Agent Session:'. \
No quotes, markdown, or trailing punctuation. Return only the title text.";

const TITLE_REWRITE_SYSTEM: &str = "The last title copied the user's request, was a question, or included a prefix. \
Rewrite it as a short topic label (noun phrase) in the user's language, 2–8 words. \
Example: 'sglang 目前性能怎么样' → 'SGLang 性能分析'. \
Do not include prefixes like 'Title:' or 'Agent Session:'. \
No questions, quotes, or punctuation wrapping. Return only the title text.";

pub(crate) fn strip_title_prefix(raw: &str) -> &str {
    let lower = raw.to_lowercase();
    const PREFIX_WORDS: &[&str] = &[
        "agent session",
        "session title",
        "session",
        "task title",
        "run title",
        "title",
        "topic",
        "会话标题",
        "会话",
        "任务标题",
        "任务",
        "标题",
        "主题",
    ];
    for word in PREFIX_WORDS {
        if lower.starts_with(word) {
            let rest = &raw[word.len()..];
            let rest_trimmed = rest.trim_start();
            if let Some(after_delim) = rest_trimmed.strip_prefix(|c| c == ':' || c == '：' || c == '-' || c == '—' || c == '–') {
                return after_delim.trim();
            }
        }
    }
    raw
}

pub(crate) fn sanitize_run_title(raw: &str) -> String {
    let mut cleaned = raw
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`' || c == '「' || c == '」' || c == '“' || c == '”' || c == '‘' || c == '’')
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .trim_start_matches(['#', '-', '*', ' '])
        .trim();
    cleaned = strip_title_prefix(cleaned);
    cleaned = cleaned
        .trim_matches(|c| c == '"' || c == '\'' || c == '`' || c == '「' || c == '」' || c == '“' || c == '”' || c == '‘' || c == '’')
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

#[cfg(test)]
mod tests {
    use super::{
        chat_completions_url, format_title_focus, sanitize_run_title, title_echoes_source,
        title_from_chat_response, title_is_user_locked, title_looks_like_question,
        title_needs_rewrite,
    };
    use serde_json::json;

    #[test]
    fn completions_url_appends_path() {
        assert_eq!(
            chat_completions_url("https://api.deepseek.com/v1"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_url("https://open.bigmodel.cn/api/paas/v4/"),
            "https://open.bigmodel.cn/api/paas/v4/chat/completions"
        );
        assert_eq!(
            chat_completions_url("https://api.openai.com/v1/chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn sanitize_strips_wrapping() {
        assert_eq!(sanitize_run_title("  \"项目总结\"  "), "项目总结");
        assert_eq!(sanitize_run_title("# Fix login bug\nmore"), "Fix login bug");
        assert_eq!(
            sanitize_run_title("Agent Session: Task Check"),
            "Task Check"
        );
        assert_eq!(
            sanitize_run_title("agent session - Task Check"),
            "Task Check"
        );
        assert_eq!(sanitize_run_title("Title: 代码优化"), "代码优化");
        assert_eq!(sanitize_run_title("会话标题：任务检查"), "任务检查");
    }

    #[test]
    fn title_rejects_prompt_echo_and_questions() {
        assert!(title_echoes_source(
            "sglang 目前性能怎么样",
            &["sglang 目前性能怎么样"]
        ));
        assert!(title_echoes_source(
            "SGLang 目前性能怎么样？",
            &["sglang 目前性能怎么样"]
        ));
        assert!(!title_echoes_source(
            "SGLang 性能分析",
            &["sglang 目前性能怎么样"]
        ));
        let long = "请用 Rust 逐步分析多线程 Tokio 异步队列可能出现的死锁根因，并给出推导证明与完整的重构代码";
        assert!(title_echoes_source(
            "请用 Rust 逐步分析多线程 Tokio 异步队列可…",
            &[long]
        ));
        assert!(title_needs_rewrite(
            "请用 Rust 逐步分析多线程 Tokio 异步队列",
            &[long]
        ));
        assert!(title_looks_like_question("sglang 目前性能怎么样"));
        assert!(title_needs_rewrite(
            "sglang 目前性能怎么样",
            &["sglang 目前性能怎么样"]
        ));
        assert!(!title_needs_rewrite(
            "SGLang 性能分析",
            &["sglang 目前性能怎么样"]
        ));
    }

    #[test]
    fn title_from_chat_reads_string_or_parts() {
        assert_eq!(
            title_from_chat_response(&json!({
                "choices": [{ "message": { "content": "SGLang 性能分析" } }]
            })),
            "SGLang 性能分析"
        );
        assert_eq!(
            title_from_chat_response(&json!({
                "choices": [{ "message": { "content": [{ "type": "text", "text": "SGLang 性能分析" }] } }]
            })),
            "SGLang 性能分析"
        );
    }

    #[test]
    fn title_focus_includes_recent_follow_ups() {
        let focus = format_title_focus("检查一下项目", &["服务器不动，继续用".into()]);
        assert!(focus.contains("Original task:\n检查一下项目"));
        assert!(focus.contains("1. 服务器不动，继续用"));
    }

    #[test]
    fn user_rename_locks_auto_title() {
        assert!(!title_is_user_locked("检查一下项目", None, "检查一下项目"));
        assert!(title_is_user_locked("我改的标题", None, "检查一下项目"));
        assert!(!title_is_user_locked(
            "SSH 优化服务器",
            Some("SSH 优化服务器"),
            "检查一下项目"
        ));
        assert!(title_is_user_locked(
            "手动标题",
            Some("SSH 优化服务器"),
            "检查一下项目"
        ));
    }
}
