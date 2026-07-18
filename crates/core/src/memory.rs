//! Session memory flush + post-compact injection (grok-aligned subset).
//!
//! Before compaction, optionally ask the model to extract durable lessons into
//! `{workdir}/.zene/memory/daily/YYYY-MM-DD.md`. After compaction (and at
//! session start), recent memory is re-injected as a stable `<memory-context>`
//! block so work survives history loss without reshuffling the system prefix
//! every turn.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::{info, warn};
use zene_llm::{ChatClient, ChatRequest, Message, ToolDefinition};

pub const MEMORY_CONTEXT_OPEN: &str = "<memory-context>";
pub const MEMORY_CONTEXT_CLOSE: &str = "</memory-context>";

const FLUSH_SYSTEM_PROMPT: &str = "\
You are a memory assistant. Extract ALL useful information from this conversation \
that would help you be more effective in future sessions with this user. \
Write a concise markdown summary with ## headers covering:

- **Decisions & rationale** — what was chosen and why
- **Technical context** — architecture, APIs, patterns, tools, file paths discussed
- **Problems & solutions** — bugs found, how they were fixed, workarounds

Omit any section where there is nothing substantive to report. \
Do NOT include ephemeral progress or routine Q&A.

Respond with NO_REPLY if nothing genuinely useful was learned.";

const MAX_FLUSH_CHARS: usize = 4_000;
const MAX_INJECT_CHARS: usize = 3_000;
const FLUSH_INPUT_MSG_CAP: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlushResult {
    NothingToStore,
    Accepted,
    Rejected,
}

pub fn memory_enabled() -> bool {
    match std::env::var("ZENE_MEMORY") {
        Ok(v) => {
            let v = v.trim().to_lowercase();
            !(v == "0" || v == "false" || v == "off" || v == "no")
        }
        Err(_) => true,
    }
}

pub fn memory_root(workdir: &Path) -> PathBuf {
    workdir.join(".zene").join("memory")
}

pub fn daily_log_path(workdir: &Path) -> PathBuf {
    let day = chrono::Utc::now().format("%Y-%m-%d");
    memory_root(workdir).join("daily").join(format!("{day}.md"))
}

pub fn conversation_has_memory_context(messages: &[Message]) -> bool {
    messages.first().is_some_and(|m| {
        m.role == zene_llm::Role::System
            && m.content
                .as_ref()
                .is_some_and(|c| c.contains(MEMORY_CONTEXT_OPEN))
    })
}

fn is_no_reply(text: &str) -> bool {
    let t = text.trim().to_uppercase();
    t == "NO_REPLY" || t == "NO REPLY" || t.starts_with("NO_REPLY\n")
}

fn has_markdown_headers(text: &str) -> bool {
    text.lines().any(|l| l.trim_start().starts_with("## "))
}

pub fn process_flush_response(response: &str) -> Result<Option<String>, FlushResult> {
    let trimmed = response.trim();
    if trimmed.is_empty() || is_no_reply(trimmed) {
        return Err(FlushResult::NothingToStore);
    }
    let content: String = if trimmed.chars().count() > MAX_FLUSH_CHARS {
        trimmed.chars().take(MAX_FLUSH_CHARS).collect()
    } else {
        trimmed.to_string()
    };
    if !has_markdown_headers(&content) {
        return Err(FlushResult::Rejected);
    }
    Ok(Some(content))
}

pub fn append_daily_log(workdir: &Path, content: &str) -> Result<PathBuf> {
    let path = daily_log_path(workdir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("create memory daily dir")?;
    }
    let mut block = String::new();
    if path.exists() {
        block.push_str("\n\n---\n\n");
    }
    block.push_str(&format!(
        "## Flush {}\n\n{content}\n",
        chrono::Utc::now().format("%H:%M:%SZ")
    ));
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .context("open daily memory log")?;
    file.write_all(block.as_bytes())
        .context("write daily memory log")?;
    Ok(path)
}

/// Load recent memory text for injection (MEMORY.md + recent daily logs).
pub fn load_recent_memory(workdir: &Path) -> Option<String> {
    let root = memory_root(workdir);
    let mut chunks = Vec::new();

    let memory_md = root.join("MEMORY.md");
    if let Ok(text) = fs::read_to_string(&memory_md) {
        if !text.trim().is_empty() {
            chunks.push(text);
        }
    }

    let daily_dir = root.join("daily");
    if let Ok(mut entries) = fs::read_dir(&daily_dir) {
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
            .collect();
        files.sort();
        for path in files.iter().rev().take(3) {
            if let Ok(text) = fs::read_to_string(path) {
                if !text.trim().is_empty() {
                    chunks.push(text);
                }
            }
        }
    }

    if chunks.is_empty() {
        return None;
    }
    let mut combined = chunks.join("\n\n");
    if combined.chars().count() > MAX_INJECT_CHARS {
        // Keep the newest tail.
        let owned: String = combined
            .chars()
            .rev()
            .take(MAX_INJECT_CHARS)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        combined = format!("…\n{owned}");
    }
    Some(combined)
}

pub fn format_memory_context_block(body: &str) -> String {
    format!(
        "{MEMORY_CONTEXT_OPEN}\n## Relevant Memory from Past Sessions\n\n{body}\n{MEMORY_CONTEXT_CLOSE}"
    )
}

/// Append memory context to the system message once (KV-stable for the session).
pub fn ensure_memory_in_system(messages: &mut [Message], workdir: &Path) {
    if !memory_enabled() || conversation_has_memory_context(messages) {
        return;
    }
    let Some(body) = load_recent_memory(workdir) else {
        return;
    };
    let block = format_memory_context_block(&body);
    if let Some(system) = messages.first_mut() {
        if system.role == zene_llm::Role::System {
            let existing = system.content.clone().unwrap_or_default();
            system.content = Some(format!("{existing}\n\n{block}"));
        }
    }
}

/// Reminder fragment for post-compaction reinjection.
pub fn memory_reminder(workdir: &Path) -> Option<String> {
    if !memory_enabled() {
        return None;
    }
    let body = load_recent_memory(workdir)?;
    Some(format_memory_context_block(&body))
}

fn format_flush_input(messages: &[Message]) -> String {
    let start = messages.len().saturating_sub(FLUSH_INPUT_MSG_CAP);
    let mut out = String::new();
    for message in &messages[start..] {
        let role = match message.role {
            zene_llm::Role::System => continue,
            zene_llm::Role::User => "user",
            zene_llm::Role::Assistant => "assistant",
            zene_llm::Role::Tool => "tool",
        };
        out.push_str(&format!("[{role}] "));
        if let Some(content) = &message.content {
            let clipped: String = content.chars().take(800).collect();
            out.push_str(&clipped);
            out.push('\n');
        }
    }
    out
}

/// Run a flush LLM call and append to the daily log when accepted.
pub async fn run_memory_flush(
    client: &ChatClient,
    model: &str,
    messages: &[Message],
    workdir: &Path,
) -> Result<FlushResult> {
    if !memory_enabled() {
        return Ok(FlushResult::NothingToStore);
    }
    let conversation = format_flush_input(messages);
    if conversation.trim().is_empty() {
        return Ok(FlushResult::NothingToStore);
    }

    let request = ChatRequest {
        model: model.to_string(),
        messages: vec![
            Message::system(FLUSH_SYSTEM_PROMPT),
            Message::user(format!(
                "Conversation to extract memory from:\n\n{conversation}"
            )),
        ],
        tools: Vec::<ToolDefinition>::new(),
        stream: false,
    };

    let response = client.chat(request).await.context("memory flush chat")?;
    let text = response
        .message
        .content
        .unwrap_or_default();

    match process_flush_response(&text) {
        Ok(Some(content)) => {
            let path = append_daily_log(workdir, &content)?;
            info!(path = %path.display(), chars = content.len(), "memory flush wrote daily log");
            Ok(FlushResult::Accepted)
        }
        Err(FlushResult::NothingToStore) => {
            info!("memory flush: nothing to store");
            Ok(FlushResult::NothingToStore)
        }
        Err(FlushResult::Rejected) => {
            warn!("memory flush: rejected (missing ## headers)");
            Ok(FlushResult::Rejected)
        }
        Err(FlushResult::Accepted) | Ok(None) => Ok(FlushResult::NothingToStore),
    }
}

/// Soft threshold: flush when usage is within `lead` pp of auto-compact.
pub fn should_flush(
    usage_percent: u8,
    compact_threshold_percent: u8,
    already_flushed_this_cycle: bool,
) -> bool {
    if !memory_enabled() || already_flushed_this_cycle {
        return false;
    }
    let lead = std::env::var("ZENE_MEMORY_FLUSH_LEAD_PERCENT")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(5u8);
    let start = compact_threshold_percent.saturating_sub(lead);
    usage_percent >= start
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn rejects_no_reply_and_unstructured() {
        assert!(matches!(
            process_flush_response("NO_REPLY"),
            Err(FlushResult::NothingToStore)
        ));
        assert!(matches!(
            process_flush_response("just some prose without headers"),
            Err(FlushResult::Rejected)
        ));
        assert!(matches!(
            process_flush_response("## Decisions\n\nUse Axum.\n"),
            Ok(Some(_))
        ));
    }

    #[test]
    fn append_and_load_daily() {
        let dir = tempdir().unwrap();
        append_daily_log(dir.path(), "## Decisions\n\nShip it.\n").unwrap();
        let loaded = load_recent_memory(dir.path()).unwrap();
        assert!(loaded.contains("Ship it"));
        let block = format_memory_context_block(&loaded);
        assert!(block.contains(MEMORY_CONTEXT_OPEN));
    }

    #[test]
    fn should_flush_respects_cycle() {
        assert!(should_flush(82, 85, false));
        assert!(!should_flush(82, 85, true));
        assert!(!should_flush(10, 85, false));
    }
}
