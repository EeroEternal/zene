use std::fs;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use tracing::info;
use zene_config::CompactionConfig;
use zene_llm::{ChatClient, ChatRequest, ChatResponse, Message, MessageKind, Role, ToolDefinition};
use zene_session::{session_record_dir, SessionRecord};
use zene_tools::{BackgroundTask, BackgroundTaskStatus};

use crate::input_ladder::{prepare_summary_input, InputLadderStage};
use crate::prefire::PrefireCache;
use crate::tokens::{self, TokenEstimator};
use crate::two_pass::{
    note_for_pass2, pass2_user_prompt, split_messages_for_two_pass, TWO_PASS_DEFAULT_SPLIT_FRACTION,
};

const SUMMARY_SYSTEM_PROMPT: &str = "You summarize coding agent conversations. Preserve key user requests, files discussed or modified, tool outcomes, errors and fixes, and current task state. Prefer tight prose over verbatim dumps. Aim for a few hundred to a few thousand words.";

/// Max chars kept in a tool result before truncate-only compaction replaces the body.
const TRUNCATE_TOOL_RESULT_MAX_CHARS: usize = 800;
/// Max chars kept in assistant text before truncate-only compaction replaces the body.
const TRUNCATE_ASSISTANT_TEXT_MAX_CHARS: usize = 1_200;
/// Cleaned summaries shorter than this are treated as degenerate and retried
/// (aligned with grok-build `MIN_SUMMARY_SEED_CHARS`).
pub const MIN_SUMMARY_SEED_CHARS: usize = 500;

pub fn should_compact(estimated_tokens: u32, config: &CompactionConfig) -> bool {
    let threshold =
        (config.context_window_tokens as f32 * config.trigger_ratio).floor() as u32;
    estimated_tokens >= threshold
}

pub fn keep_recent_token_budget(config: &CompactionConfig) -> u32 {
    (config.context_window_tokens as f32 * config.keep_recent_ratio).floor() as u32
}

fn system_prefix_start(messages: &[Message]) -> usize {
    messages
        .first()
        .filter(|m| m.role == Role::System)
        .map(|_| 1usize)
        .unwrap_or(0)
}

/// Index where the recent tail begins. Returns `None` if there is nothing worth compacting.
pub fn tail_start_index(
    messages: &[Message],
    keep_recent_tokens: u32,
    min_keep_messages: usize,
    estimator: &TokenEstimator,
) -> Option<usize> {
    if messages.is_empty() {
        return None;
    }

    let prefix_start = system_prefix_start(messages);

    if prefix_start >= messages.len() {
        return None;
    }

    let non_system_count = messages.len() - prefix_start;
    if non_system_count <= min_keep_messages {
        return None;
    }

    let mut tokens = 0u32;
    let mut tail_start = messages.len();

    for i in (prefix_start..messages.len()).rev() {
        tokens += estimator.estimate_message_tokens(&messages[i]);
        tail_start = i;
        if tokens >= keep_recent_tokens {
            break;
        }
    }

    let max_tail_start = messages.len().saturating_sub(min_keep_messages);
    if tail_start > max_tail_start {
        tail_start = max_tail_start;
    }

    // Never split assistant tool_calls from their tool results (grok tool-pair snap).
    while tail_start > prefix_start && messages[tail_start].role == Role::Tool {
        tail_start -= 1;
    }

    if tail_start <= prefix_start {
        None
    } else {
        Some(tail_start)
    }
}

pub fn is_context_overflow_error(err: &anyhow::Error) -> bool {
    zene_llm::is_context_overflow(&err.to_string())
}

/// Whether a compaction summary is too short / empty to be useful.
pub fn is_degenerate_summary(summary: &str) -> bool {
    let cleaned = summary.trim();
    cleaned.is_empty()
        || cleaned == "(empty summary)"
        || cleaned.chars().count() < MIN_SUMMARY_SEED_CHARS
}

/// Find the last real user turn (non-empty content) for full-replace assembly.
pub fn last_user_query_index(messages: &[Message]) -> Option<usize> {
    messages.iter().rposition(|m| {
        m.role == Role::User
            && m.content
                .as_ref()
                .is_some_and(|c| !c.trim().is_empty())
    })
}

/// Rebuild history after LLM summarize (grok-style full-replace):
/// `system + last_user_query + recent_after_query + summary + optional reminder`.
pub fn assemble_full_replace_history(
    system: Option<Message>,
    last_user_query: Option<Message>,
    recent_after_query: Vec<Message>,
    summary: String,
    system_reminder: Option<String>,
) -> Vec<Message> {
    let mut out = Vec::new();
    if let Some(system) = system {
        out.push(system);
    }
    if let Some(query) = last_user_query {
        out.push(query);
    }
    out.extend(recent_after_query);
    out.push(Message::compaction_summary(format!(
        "[Previous conversation summary]\n{summary}"
    )));
    if let Some(reminder) = system_reminder {
        if !reminder.trim().is_empty() {
            out.push(Message::user(format!(
                "<system-reminder>\n{reminder}\n</system-reminder>"
            )));
        }
    }
    out
}

/// Post-compaction `<system-reminder>`: todos, background tasks, memory.
pub fn build_compaction_reminder(
    session: &SessionRecord,
    background_tasks: &[BackgroundTask],
    memory_block: Option<&str>,
) -> Option<String> {
    let mut sections = Vec::new();

    let actionable: Vec<_> = session
        .todos
        .iter()
        .filter(|item| {
            !matches!(item.status, zene_session::TodoStatus::Completed)
        })
        .collect();
    if !actionable.is_empty() {
        let mut lines = vec!["Active todos:".to_string()];
        for item in actionable {
            let status = match item.status {
                zene_session::TodoStatus::Pending => "pending",
                zene_session::TodoStatus::InProgress => "in_progress",
                zene_session::TodoStatus::Completed => "completed",
            };
            lines.push(format!("- [{status}] {}", item.content));
        }
        sections.push(lines.join("\n"));
    }

    let running: Vec<_> = background_tasks
        .iter()
        .filter(|t| t.status == BackgroundTaskStatus::Running)
        .collect();
    if !running.is_empty() {
        let mut lines = vec!["Background tasks still running:".to_string()];
        for task in running {
            let kind = match task.kind {
                zene_tools::BackgroundTaskKind::Bash => "bash",
                zene_tools::BackgroundTaskKind::Subagent => "task",
            };
            lines.push(format!("- {} ({kind}): {}", task.id, task.label));
        }
        sections.push(lines.join("\n"));
    }

    if let Some(memory) = memory_block {
        if !memory.trim().is_empty() {
            sections.push(memory.trim().to_string());
        }
    }

    if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n\n"))
    }
}

fn format_messages_for_summary(messages: &[Message]) -> String {
    let mut out = String::new();
    for message in messages {
        let role = match message.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };
        out.push_str(&format!("[{role}] "));
        if let Some(content) = &message.content {
            out.push_str(content);
            out.push('\n');
        }
        if let Some(tool_calls) = &message.tool_calls {
            for call in tool_calls {
                out.push_str(&format!("tool_call: {}({})\n", call.name, call.arguments));
            }
        }
        if message.role == Role::Tool {
            if let Some(name) = &message.name {
                out.push_str(&format!("tool: {name}\n"));
            }
        }
        out.push('\n');
    }
    out
}

/// Summarize with input ladder + degenerate-summary retries.
#[allow(dead_code)]
pub async fn summarize_messages(
    client: &ChatClient,
    model: &str,
    messages: &[Message],
) -> Result<String> {
    summarize_messages_with_ladder(client, model, messages, None, &TokenEstimator::default()).await
}

pub async fn summarize_messages_with_ladder(
    client: &ChatClient,
    model: &str,
    messages: &[Message],
    context_window: Option<u32>,
    estimator: &TokenEstimator,
) -> Result<String> {
    summarize_prepared_input(
        client,
        model,
        messages,
        context_window,
        estimator,
        "Summarize this conversation for a coding agent to continue work",
    )
    .await
}

async fn summarize_prepared_input(
    client: &ChatClient,
    model: &str,
    messages: &[Message],
    context_window: Option<u32>,
    estimator: &TokenEstimator,
    user_lead: &str,
) -> Result<String> {
    let mut stage = InputLadderStage::Verbatim;
    let budget = context_window.map(|w| (w as f32 * 0.7).floor() as u32);

    loop {
        let input = prepare_summary_input(messages, stage, budget, estimator);
        let conversation = format_messages_for_summary(&input);
        let request = ChatRequest {
            model: model.to_string(),
            messages: vec![
                Message::system(SUMMARY_SYSTEM_PROMPT),
                Message::user(format!("{user_lead}:\n\n{conversation}")),
            ],
            tools: Vec::<ToolDefinition>::new(),
            stream: false,
        };

        match client.chat(request).await {
            Ok(response) => {
                let summary = response
                    .message
                    .content
                    .unwrap_or_else(|| "(empty summary)".to_string());
                if is_degenerate_summary(&summary) {
                    info!(
                        stage = stage.as_str(),
                        summary_chars = summary.chars().count(),
                        "compaction summary degenerate; retrying"
                    );
                    if let Some(next) = stage.next() {
                        stage = next;
                        continue;
                    }
                    bail!(
                        "compaction summary too short ({} chars; need ≥ {}); refusing to discard conversation state",
                        summary.chars().count(),
                        MIN_SUMMARY_SEED_CHARS
                    );
                }
                return Ok(summary);
            }
            Err(err) if is_context_overflow_error(&err) => {
                info!(
                    stage = stage.as_str(),
                    "compaction summary overflow; stepping input ladder"
                );
                match stage.next() {
                    Some(next) => stage = next,
                    None => return Err(err).context("compaction summary overflow at lossy stage"),
                }
            }
            Err(err) => return Err(err).context("compaction summary"),
        }
    }
}

/// Pass2 of two-pass compaction: merge NOTE₁ with recent tail messages.
pub async fn summarize_pass2(
    client: &ChatClient,
    model: &str,
    system: Option<&Message>,
    note1: &str,
    tail: &[Message],
    context_window: Option<u32>,
    estimator: &TokenEstimator,
    hint: Option<&str>,
) -> Result<String> {
    let mut msgs = Vec::new();
    if let Some(system) = system {
        msgs.push(system.clone());
    }
    msgs.push(Message::user(format!(
        "Your conversation was summarized due to context constraints. \
         Here is the summary of the conversation so far:\n\n\
         <summary_content>\n{}\n</summary_content>",
        note_for_pass2(note1)
    )));
    msgs.extend(tail.iter().cloned());
    msgs.push(Message::user(pass2_user_prompt(note1, hint)));
    summarize_prepared_input(
        client,
        model,
        &msgs,
        context_window,
        estimator,
        "Produce the final compaction summary",
    )
    .await
}

/// Summarize a compactable prefix, using prefire NOTE₁ when valid, else sync two-pass
/// for large prefixes, else single-pass ladder.
pub async fn summarize_prefix(
    client: &ChatClient,
    model: &str,
    prefix: &[Message],
    context_window: Option<u32>,
    estimator: &TokenEstimator,
    prefire: Option<&PrefireCache>,
    hint: Option<&str>,
) -> Result<String> {
    let system = prefix.first().filter(|m| m.role == Role::System);
    let body_start = if system.is_some() { 1 } else { 0 };
    let body = &prefix[body_start..];

    if let Some(cache) = prefire {
        if cache.split_idx > 0 && cache.split_idx < body.len() {
            let pass1_prefix = &body[..cache.split_idx];
            let tail = &body[cache.split_idx..];
            info!(
                split_idx = cache.split_idx,
                note1_chars = cache.note1.len(),
                "compaction using prefire NOTE₁ (two-pass pass2)"
            );
            return summarize_pass2(
                client,
                model,
                system,
                &cache.note1,
                tail,
                context_window,
                estimator,
                hint,
            )
            .await;
        }
    }

    let body_tokens = estimator.estimate_messages_tokens(body);
    let two_pass_min = context_window
        .map(|w| (w as f32 * 0.35).floor() as u32)
        .unwrap_or(8_000);
    if body.len() >= 8 && body_tokens >= two_pass_min {
        let split = split_messages_for_two_pass(body, estimator, TWO_PASS_DEFAULT_SPLIT_FRACTION);
        if split.split_idx > 0 && split.split_idx < body.len() {
            info!(
                split_idx = split.split_idx,
                body_tokens, "compaction sync two-pass (no prefire cache)"
            );
            let note1 = summarize_messages_with_ladder(
                client,
                model,
                &body[..split.split_idx],
                context_window,
                estimator,
            )
            .await?;
            return summarize_pass2(
                client,
                model,
                system,
                &note1,
                &body[split.split_idx..],
                context_window,
                estimator,
                hint,
            )
            .await;
        }
    }

    summarize_messages_with_ladder(client, model, prefix, context_window, estimator).await
}

/// Persist compacted prefix for recovery (grok CompactionMode::Segments lite).
pub fn persist_compaction_segment(
    session_id: &str,
    prefix: &[Message],
    summary: &str,
) -> Result<PathBuf> {
    let dir = session_record_dir(session_id).join("compaction_segments");
    fs::create_dir_all(&dir).context("create compaction_segments dir")?;
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let path = dir.join(format!("{ts}.md"));
    let mut body = String::new();
    body.push_str("# Compaction segment\n\n");
    body.push_str("## Final summary\n\n");
    body.push_str(summary);
    body.push_str("\n\n## Compacted prefix\n\n");
    body.push_str(&format_messages_for_summary(prefix));
    fs::write(&path, body).context("write compaction segment")?;
    Ok(path)
}

pub struct CompactionPlan {
    pub tail_start: usize,
    pub compacted_count: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct CompactionStats {
    pub tokens_before: u32,
    pub tokens_after: u32,
}

pub struct CompactionResult {
    pub reason: String,
    pub compacted_count: usize,
    pub stats: CompactionStats,
}

pub fn plan_compaction(
    messages: &[Message],
    config: &CompactionConfig,
    estimator: &TokenEstimator,
) -> Option<CompactionPlan> {
    let tail_start = tail_start_index(
        messages,
        keep_recent_token_budget(config),
        config.min_keep_messages,
        estimator,
    )?;
    let prefix_start = system_prefix_start(messages);
    let compacted_count = tail_start.saturating_sub(prefix_start);
    if compacted_count == 0 {
        return None;
    }
    Some(CompactionPlan {
        tail_start,
        compacted_count,
    })
}

fn truncate_message_body(content: &str, max_content_chars: usize) -> Option<String> {
    if content.starts_with("[truncated ") {
        return None;
    }
    let char_count = content.chars().count();
    if char_count <= max_content_chars {
        return None;
    }
    Some(format!("[truncated {char_count} chars]"))
}

/// Phase 1: replace very old tool results and long assistant text with short placeholders.
pub fn truncate_old_message_bodies(
    messages: &mut [Message],
    prefix_start: usize,
    tail_start: usize,
    max_tool_result_chars: usize,
    max_assistant_text_chars: usize,
) -> usize {
    let mut truncated = 0usize;
    for message in &mut messages[prefix_start..tail_start] {
        match message.role {
            Role::Tool => {
                let Some(content) = message.content.as_ref() else {
                    continue;
                };
                if let Some(replacement) = truncate_message_body(content, max_tool_result_chars) {
                    message.content = Some(replacement);
                    truncated += 1;
                }
            }
            Role::Assistant if message.kind != Some(MessageKind::CompactionSummary) => {
                let Some(content) = message.content.as_ref() else {
                    continue;
                };
                if let Some(replacement) = truncate_message_body(content, max_assistant_text_chars) {
                    message.content = Some(replacement);
                    truncated += 1;
                }
            }
            _ => {}
        }
    }
    truncated
}

/// Replace very old tool result bodies with a short placeholder (legacy alias).
#[allow(dead_code)]
pub fn truncate_old_tool_results(
    messages: &mut [Message],
    prefix_start: usize,
    tail_start: usize,
    max_content_chars: usize,
) -> usize {
    truncate_old_message_bodies(
        messages,
        prefix_start,
        tail_start,
        max_content_chars,
        TRUNCATE_ASSISTANT_TEXT_MAX_CHARS,
    )
}

pub fn build_sliced_messages(messages: &[Message], tail_start: usize) -> Vec<Message> {
    let prefix_start = system_prefix_start(messages);
    let mut kept = Vec::new();

    if let Some(system) = messages.first().filter(|m| m.role == Role::System) {
        kept.push(system.clone());
    }

    for message in &messages[prefix_start..tail_start] {
        if message.kind == Some(MessageKind::CompactionSummary) {
            kept.push(message.clone());
        }
    }

    kept.extend_from_slice(&messages[tail_start..]);
    kept
}

/// Phase 2: drop old prefix messages, keeping system, compaction summaries, and recent tail.
pub fn apply_slice_keep(messages: &mut Vec<Message>, tail_start: usize) -> usize {
    let prefix_start = system_prefix_start(messages);
    if tail_start <= prefix_start {
        return 0;
    }

    let summaries_kept = messages[prefix_start..tail_start]
        .iter()
        .filter(|m| m.kind == Some(MessageKind::CompactionSummary))
        .count();
    let removed = tail_start.saturating_sub(prefix_start).saturating_sub(summaries_kept);
    let sliced = build_sliced_messages(messages, tail_start);
    *messages = sliced;
    removed
}

fn estimate_session_tokens(
    session: &SessionRecord,
    tools: &[ToolDefinition],
    estimator: &TokenEstimator,
) -> u32 {
    tokens::estimate_context(&session.messages, tools, estimator) as u32
}

fn record_compaction_result(
    session: &mut SessionRecord,
    result: &CompactionResult,
    summary: Option<String>,
) {
    session.record_compaction_event(
        &result.reason,
        result.compacted_count,
        summary,
        Some(result.stats.tokens_before),
        Some(result.stats.tokens_after),
    );
}

fn try_truncate_only_compaction(
    session: &mut SessionRecord,
    config: &CompactionConfig,
    tools: &[ToolDefinition],
    estimator: &TokenEstimator,
) -> Option<CompactionResult> {
    let tokens_before = estimate_session_tokens(session, tools, estimator);
    let plan = plan_compaction(&session.messages, config, estimator)?;
    let prefix_start = system_prefix_start(&session.messages);
    let truncated = truncate_old_message_bodies(
        &mut session.messages,
        prefix_start,
        plan.tail_start,
        TRUNCATE_TOOL_RESULT_MAX_CHARS,
        TRUNCATE_ASSISTANT_TEXT_MAX_CHARS,
    );
    if truncated == 0 {
        return None;
    }

    let tokens_after = estimate_session_tokens(session, tools, estimator);
    if should_compact(tokens_after, config) {
        return None;
    }

    info!(
        reason = "truncate_only",
        truncated_messages = truncated,
        tokens_before,
        tokens_after,
        "context compaction avoided LLM summarize via in-place truncation"
    );

    let result = CompactionResult {
        reason: "truncate_only".to_string(),
        compacted_count: truncated,
        stats: CompactionStats {
            tokens_before,
            tokens_after,
        },
    };
    record_compaction_result(session, &result, None);
    Some(result)
}

fn try_slice_keep_compaction(
    session: &mut SessionRecord,
    config: &CompactionConfig,
    tools: &[ToolDefinition],
    estimator: &TokenEstimator,
) -> Option<CompactionResult> {
    let tokens_before = estimate_session_tokens(session, tools, estimator);
    let plan = plan_compaction(&session.messages, config, estimator)?;

    let sliced = build_sliced_messages(&session.messages, plan.tail_start);
    let tokens_after =
        tokens::estimate_context(&sliced, tools, estimator) as u32;
    if should_compact(tokens_after, config) {
        return None;
    }

    let removed = apply_slice_keep(&mut session.messages, plan.tail_start);
    if removed == 0 {
        return None;
    }

    info!(
        reason = "slice_keep",
        removed_messages = removed,
        tokens_before,
        tokens_after,
        "context compaction avoided LLM summarize via slice keep"
    );

    let result = CompactionResult {
        reason: "slice_keep".to_string(),
        compacted_count: removed,
        stats: CompactionStats {
            tokens_before,
            tokens_after,
        },
    };
    record_compaction_result(session, &result, None);
    Some(result)
}

/// Overflow recovery: apply phase-1 truncation in place before a retry (no threshold check).
pub fn apply_overflow_truncate_pass(
    session: &mut SessionRecord,
    config: &CompactionConfig,
    estimator: &TokenEstimator,
) -> bool {
    let Some(plan) = plan_compaction(&session.messages, config, estimator) else {
        return false;
    };
    let prefix_start = system_prefix_start(&session.messages);
    truncate_old_message_bodies(
        &mut session.messages,
        prefix_start,
        plan.tail_start,
        TRUNCATE_TOOL_RESULT_MAX_CHARS,
        TRUNCATE_ASSISTANT_TEXT_MAX_CHARS,
    ) > 0
}

pub fn subagent_compaction_config(parent: &CompactionConfig) -> CompactionConfig {
    CompactionConfig {
        context_window_tokens: parent.context_window_tokens.min(16_000),
        trigger_ratio: 0.5,
        keep_recent_ratio: parent.keep_recent_ratio.max(0.3),
        min_keep_messages: parent.min_keep_messages,
    }
}

pub fn apply_compaction_to_messages(
    messages: &mut Vec<Message>,
    summary: String,
    tail_start: usize,
    compacted_count: usize,
) {
    let system = messages
        .first()
        .filter(|m| m.role == Role::System)
        .cloned();
    let (last_user, recent) = match last_user_query_index(messages) {
        Some(idx) if idx >= tail_start => {
            let query = messages[idx].clone();
            let recent = messages[idx + 1..].to_vec();
            (Some(query), recent)
        }
        Some(idx) => {
            // Last user query is in the compacted prefix — re-inject it and keep
            // the planned recent tail (which may include later tool work).
            let query = messages[idx].clone();
            let recent = messages[tail_start..].to_vec();
            (Some(query), recent)
        }
        None => (None, messages[tail_start..].to_vec()),
    };

    *messages = assemble_full_replace_history(system, last_user, recent, summary, None);
    let _ = compacted_count;
}

async fn compact_with_phases<F, Fut>(
    messages: &mut Vec<Message>,
    model: &str,
    config: &CompactionConfig,
    reason: &str,
    tools: &[ToolDefinition],
    estimator: &TokenEstimator,
    chat: F,
) -> Result<Option<CompactionResult>>
where
    F: Fn(ChatRequest) -> Fut,
    Fut: std::future::Future<Output = Result<ChatResponse>>,
{
    let tokens_before =
        tokens::estimate_context(messages, tools, estimator) as u32;

    if let Some(result) = try_truncate_only_on_messages(messages, config, tools, estimator) {
        return Ok(Some(result));
    }

    if let Some(result) = try_slice_keep_on_messages(messages, config, tools, estimator) {
        return Ok(Some(result));
    }

    let plan = match plan_compaction(messages, config, estimator) {
        Some(plan) => plan,
        None => return Ok(None),
    };

    let prefix_start = system_prefix_start(messages);
    let prefix = messages[prefix_start..plan.tail_start].to_vec();
    let request = ChatRequest {
        model: model.to_string(),
        messages: vec![
            Message::system(SUMMARY_SYSTEM_PROMPT),
            Message::user(format!(
                "Summarize this conversation for a coding agent to continue work:\n\n{}",
                format_messages_for_summary(&prefix)
            )),
        ],
        tools: Vec::new(),
        stream: false,
    };
    let response = chat(request).await.context("compaction summary")?;
    let summary = response
        .message
        .content
        .unwrap_or_else(|| "(empty summary)".to_string());

    apply_compaction_to_messages(messages, summary.clone(), plan.tail_start, plan.compacted_count);
    let tokens_after = tokens::estimate_context(messages, tools, estimator) as u32;

    info!(
        reason,
        compacted_messages = plan.compacted_count,
        tail_messages = messages.len(),
        summary_chars = summary.len(),
        tokens_before,
        tokens_after,
        "context compaction applied via LLM summarize"
    );

    Ok(Some(CompactionResult {
        reason: reason.to_string(),
        compacted_count: plan.compacted_count,
        stats: CompactionStats {
            tokens_before,
            tokens_after,
        },
    }))
}

fn try_truncate_only_on_messages(
    messages: &mut Vec<Message>,
    config: &CompactionConfig,
    tools: &[ToolDefinition],
    estimator: &TokenEstimator,
) -> Option<CompactionResult> {
    let tokens_before = tokens::estimate_context(messages, tools, estimator) as u32;
    let plan = plan_compaction(messages, config, estimator)?;
    let prefix_start = system_prefix_start(messages);
    let truncated = truncate_old_message_bodies(
        messages,
        prefix_start,
        plan.tail_start,
        TRUNCATE_TOOL_RESULT_MAX_CHARS,
        TRUNCATE_ASSISTANT_TEXT_MAX_CHARS,
    );
    if truncated == 0 {
        return None;
    }
    let tokens_after = tokens::estimate_context(messages, tools, estimator) as u32;
    if should_compact(tokens_after, config) {
        return None;
    }
    Some(CompactionResult {
        reason: "truncate_only".to_string(),
        compacted_count: truncated,
        stats: CompactionStats {
            tokens_before,
            tokens_after,
        },
    })
}

fn try_slice_keep_on_messages(
    messages: &mut Vec<Message>,
    config: &CompactionConfig,
    tools: &[ToolDefinition],
    estimator: &TokenEstimator,
) -> Option<CompactionResult> {
    let tokens_before = tokens::estimate_context(messages, tools, estimator) as u32;
    let plan = plan_compaction(messages, config, estimator)?;
    let sliced = build_sliced_messages(messages, plan.tail_start);
    let tokens_after = tokens::estimate_context(&sliced, tools, estimator) as u32;
    if should_compact(tokens_after, config) {
        return None;
    }
    let removed = apply_slice_keep(messages, plan.tail_start);
    if removed == 0 {
        return None;
    }
    Some(CompactionResult {
        reason: "slice_keep".to_string(),
        compacted_count: removed,
        stats: CompactionStats {
            tokens_before,
            tokens_after,
        },
    })
}

pub async fn compact_message_list_with_chat<F, Fut>(
    messages: &mut Vec<Message>,
    model: &str,
    config: &CompactionConfig,
    reason: &str,
    tools: &[ToolDefinition],
    estimator: &TokenEstimator,
    chat: F,
) -> Result<Option<CompactionResult>>
where
    F: Fn(ChatRequest) -> Fut,
    Fut: std::future::Future<Output = Result<ChatResponse>>,
{
    compact_with_phases(messages, model, config, reason, tools, estimator, chat).await
}

#[allow(dead_code)]
pub async fn summarize_messages_with_chat<F, Fut>(
    model: &str,
    messages: &[Message],
    chat: F,
) -> Result<String>
where
    F: FnOnce(ChatRequest) -> Fut,
    Fut: std::future::Future<Output = Result<ChatResponse>>,
{
    let conversation = format_messages_for_summary(messages);
    let request = ChatRequest {
        model: model.to_string(),
        messages: vec![
            Message::system(SUMMARY_SYSTEM_PROMPT),
            Message::user(format!(
                "Summarize this conversation for a coding agent to continue work:\n\n{conversation}"
            )),
        ],
        tools: Vec::<ToolDefinition>::new(),
        stream: false,
    };

    let response = chat(request).await.context("compaction summary")?;
    Ok(response
        .message
        .content
        .unwrap_or_else(|| "(empty summary)".to_string()))
}

pub async fn compact_session(
    session: &mut SessionRecord,
    client: &ChatClient,
    model: &str,
    config: &CompactionConfig,
    reason: &str,
    tools: &[ToolDefinition],
    estimator: &TokenEstimator,
    background_tasks: &[BackgroundTask],
    prefire: Option<&PrefireCache>,
    memory_block: Option<&str>,
) -> Result<Option<CompactionResult>> {
    if let Some(result) = try_truncate_only_compaction(session, config, tools, estimator) {
        return Ok(Some(result));
    }

    if let Some(result) = try_slice_keep_compaction(session, config, tools, estimator) {
        return Ok(Some(result));
    }

    let tokens_before = estimate_session_tokens(session, tools, estimator);
    let plan = match plan_compaction(&session.messages, config, estimator) {
        Some(plan) => plan,
        None => return Ok(None),
    };

    let prefix_start = system_prefix_start(&session.messages);
    let prefix = session.messages[prefix_start..plan.tail_start].to_vec();
    let summary = summarize_prefix(
        client,
        model,
        &prefix,
        Some(config.context_window_tokens),
        estimator,
        prefire,
        None,
    )
    .await?;

    if let Ok(path) = persist_compaction_segment(&session.meta.id, &prefix, &summary) {
        info!(path = %path.display(), "wrote compaction segment");
    }

    info!(
        reason = reason,
        compacted_messages = plan.compacted_count,
        tail_messages = session.messages.len() - plan.tail_start,
        summary_chars = summary.len(),
        tokens_before,
        "context compaction applying LLM summarize (full-replace)"
    );

    apply_full_replace_to_session(
        session,
        summary,
        plan.tail_start,
        plan.compacted_count,
        reason,
        tokens_before,
        background_tasks,
        memory_block,
    );

    let tokens_after = estimate_session_tokens(session, tools, estimator);
    if let Some(entry) = session.compactions.last_mut() {
        entry.tokens_after = Some(tokens_after);
    }

    Ok(Some(CompactionResult {
        reason: reason.to_string(),
        compacted_count: plan.compacted_count,
        stats: CompactionStats {
            tokens_before,
            tokens_after,
        },
    }))
}

/// Force a compaction pass (manual `/compact`), skipping truncate/slice when
/// `force_summarize` is true.
pub async fn compact_session_forced(
    session: &mut SessionRecord,
    client: &ChatClient,
    model: &str,
    config: &CompactionConfig,
    reason: &str,
    tools: &[ToolDefinition],
    estimator: &TokenEstimator,
    force_summarize: bool,
    user_hint: Option<&str>,
    background_tasks: &[BackgroundTask],
    prefire: Option<&PrefireCache>,
    memory_block: Option<&str>,
) -> Result<Option<CompactionResult>> {
    if !force_summarize {
        return compact_session(
            session,
            client,
            model,
            config,
            reason,
            tools,
            estimator,
            background_tasks,
            prefire,
            memory_block,
        )
        .await;
    }

    let tokens_before = estimate_session_tokens(session, tools, estimator);
    let plan = match plan_compaction(&session.messages, config, estimator) {
        Some(plan) => plan,
        None => {
            let prefix_start = system_prefix_start(&session.messages);
            let min_keep = config.min_keep_messages.min(4).max(1);
            if session.messages.len().saturating_sub(prefix_start) <= min_keep {
                return Ok(None);
            }
            CompactionPlan {
                tail_start: session.messages.len().saturating_sub(min_keep),
                compacted_count: session
                    .messages
                    .len()
                    .saturating_sub(prefix_start)
                    .saturating_sub(min_keep),
            }
        }
    };

    let prefix_start = system_prefix_start(&session.messages);
    let prefix = session.messages[prefix_start..plan.tail_start].to_vec();
    let summary = summarize_prefix(
        client,
        model,
        &prefix,
        Some(config.context_window_tokens),
        estimator,
        prefire,
        user_hint,
    )
    .await?;

    if let Ok(path) = persist_compaction_segment(&session.meta.id, &prefix, &summary) {
        info!(path = %path.display(), "wrote compaction segment");
    }

    apply_full_replace_to_session(
        session,
        summary,
        plan.tail_start,
        plan.compacted_count,
        reason,
        tokens_before,
        background_tasks,
        memory_block,
    );

    let tokens_after = estimate_session_tokens(session, tools, estimator);
    if let Some(entry) = session.compactions.last_mut() {
        entry.tokens_after = Some(tokens_after);
    }

    Ok(Some(CompactionResult {
        reason: reason.to_string(),
        compacted_count: plan.compacted_count,
        stats: CompactionStats {
            tokens_before,
            tokens_after,
        },
    }))
}

fn apply_full_replace_to_session(
    session: &mut SessionRecord,
    summary: String,
    tail_start: usize,
    compacted_count: usize,
    reason: &str,
    tokens_before: u32,
    background_tasks: &[BackgroundTask],
    memory_block: Option<&str>,
) {
    let system = session
        .messages
        .first()
        .filter(|m| m.role == Role::System)
        .cloned();
    let (last_user, recent) = match last_user_query_index(&session.messages) {
        Some(idx) if idx >= tail_start => {
            let query = session.messages[idx].clone();
            let recent = session.messages[idx + 1..].to_vec();
            (Some(query), recent)
        }
        Some(idx) => {
            let query = session.messages[idx].clone();
            let recent = session.messages[tail_start..].to_vec();
            (Some(query), recent)
        }
        None => (None, session.messages[tail_start..].to_vec()),
    };
    let reminder = build_compaction_reminder(session, background_tasks, memory_block);
    session.messages =
        assemble_full_replace_history(system, last_user, recent, summary.clone(), reminder);
    session.record_compaction_event(
        reason,
        compacted_count,
        Some(summary),
        Some(tokens_before),
        None,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use zene_config::CompactionConfig;
    use zene_llm::ToolCall;

    fn estimator() -> TokenEstimator {
        TokenEstimator::default()
    }

    fn user_msg(text: &str) -> Message {
        Message::user(text)
    }

    fn assistant_msg(text: &str) -> Message {
        Message::assistant(text)
    }

    fn tool_msg(content: &str) -> Message {
        Message::tool_result("call_1", "Read", content)
    }

    #[test]
    fn should_compact_when_over_threshold() {
        let config = CompactionConfig {
            trigger_ratio: 0.5,
            keep_recent_ratio: 0.25,
            context_window_tokens: 1000,
            min_keep_messages: 4,
        };
        assert!(!should_compact(499, &config));
        assert!(should_compact(500, &config));
        assert!(should_compact(600, &config));
    }

    #[test]
    fn should_compact_with_estimated_messages() {
        let config = CompactionConfig {
            trigger_ratio: 0.5,
            keep_recent_ratio: 0.25,
            context_window_tokens: 200,
            min_keep_messages: 2,
        };
        let messages: Vec<Message> = (0..30)
            .map(|i| user_msg(&format!("message {i}: {}", "x".repeat(200))))
            .collect();
        let est = tokens::estimate_context(&messages, &[], &estimator()) as u32;
        assert!(should_compact(est, &config));
    }

    #[test]
    fn tail_start_keeps_recent_budget() {
        let messages = vec![
            Message::system("sys"),
            user_msg(&"x".repeat(400)),
            assistant_msg(&"y".repeat(400)),
            user_msg(&"z".repeat(400)),
            assistant_msg("recent"),
        ];
        let tail_start = tail_start_index(&messages, 100, 2, &estimator()).expect("tail start");
        assert!(tail_start >= 3);
        assert!(tail_start < messages.len());
    }

    #[test]
    fn tail_start_respects_message_count_floor() {
        let messages: Vec<Message> = std::iter::once(Message::system("sys"))
            .chain((0..25).map(|i| user_msg(&format!("msg {i}"))))
            .collect();
        let tail_start = tail_start_index(&messages, 10, 20, &estimator()).expect("tail start");
        assert_eq!(messages.len() - tail_start, 20);
    }

    #[test]
    fn tail_start_none_when_below_message_floor() {
        let messages = vec![Message::system("sys"), user_msg("hi")];
        assert!(tail_start_index(&messages, 100, 20, &estimator()).is_none());
    }

    #[test]
    fn plan_compaction_counts_prefix_messages() {
        let messages = vec![
            Message::system("sys"),
            user_msg(&"a".repeat(200)),
            assistant_msg(&"b".repeat(200)),
            user_msg(&"c".repeat(200)),
            assistant_msg("recent"),
        ];
        let config = CompactionConfig {
            trigger_ratio: 0.85,
            keep_recent_ratio: 0.5,
            context_window_tokens: 100,
            min_keep_messages: 2,
        };
        let plan = plan_compaction(&messages, &config, &estimator()).expect("plan");
        assert!(plan.compacted_count >= 1);
        assert!(plan.tail_start > 1);
    }

    #[test]
    fn truncate_old_tool_results_replaces_large_bodies() {
        let mut messages = vec![
            Message::system("sys"),
            tool_msg(&"x".repeat(2000)),
            user_msg("recent"),
        ];
        let truncated = truncate_old_tool_results(&mut messages, 1, 2, 100);
        assert_eq!(truncated, 1);
        assert_eq!(
            messages[1].content.as_deref(),
            Some("[truncated 2000 chars]")
        );
    }

    #[test]
    fn truncate_old_assistant_text_replaces_large_bodies() {
        let mut messages = vec![
            Message::system("sys"),
            assistant_msg(&"x".repeat(3000)),
            user_msg("recent"),
        ];
        let truncated = truncate_old_message_bodies(&mut messages, 1, 2, 800, 500);
        assert_eq!(truncated, 1);
        assert_eq!(
            messages[1].content.as_deref(),
            Some("[truncated 3000 chars]")
        );
    }

    #[test]
    fn truncate_reduces_token_estimate() {
        let est = estimator();
        let mut messages = vec![
            Message::system("sys"),
            tool_msg(&"x".repeat(5000)),
            assistant_msg(&"y".repeat(5000)),
            user_msg("recent"),
        ];
        let before = tokens::estimate_context(&messages, &[], &est);
        let truncated = truncate_old_message_bodies(&mut messages, 1, 3, 800, 1200);
        let after = tokens::estimate_context(&messages, &[], &est);
        assert!(truncated >= 2);
        assert!(after < before);
    }

    #[test]
    fn slice_keep_preserves_compaction_summaries() {
        let mut messages = vec![
            Message::system("sys"),
            Message::compaction_summary("old summary"),
            user_msg("old user"),
            assistant_msg("old assistant"),
            user_msg("recent"),
        ];
        let removed = apply_slice_keep(&mut messages, 4);
        assert_eq!(removed, 2);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, Role::System);
        assert_eq!(messages[1].kind, Some(MessageKind::CompactionSummary));
        assert_eq!(messages[2].content.as_deref(), Some("recent"));
    }

    #[test]
    fn subagent_compaction_config_uses_smaller_threshold() {
        let parent = CompactionConfig {
            trigger_ratio: 0.85,
            keep_recent_ratio: 0.25,
            context_window_tokens: 128_000,
            min_keep_messages: 20,
        };
        let sub = subagent_compaction_config(&parent);
        assert_eq!(sub.context_window_tokens, 16_000);
        assert_eq!(sub.trigger_ratio, 0.5);
        assert_eq!(sub.min_keep_messages, 20);
    }

    #[test]
    fn overflow_error_detection() {
        assert!(is_context_overflow_error(&anyhow::anyhow!(
            "maximum context length exceeded"
        )));
        assert!(is_context_overflow_error(&anyhow::anyhow!("prompt is too long")));
        assert!(!is_context_overflow_error(&anyhow::anyhow!("connection reset")));
    }

    #[test]
    fn degenerate_summary_detection() {
        assert!(is_degenerate_summary(""));
        assert!(is_degenerate_summary("(empty summary)"));
        assert!(is_degenerate_summary("short"));
        assert!(is_degenerate_summary(&"x".repeat(120)));
        assert!(!is_degenerate_summary(&"x".repeat(500)));
    }

    #[test]
    fn full_replace_keeps_last_user_and_summary() {
        let history = assemble_full_replace_history(
            Some(Message::system("sys")),
            Some(Message::user("do the thing")),
            vec![Message::assistant("working")],
            "long enough summary ".repeat(10),
            Some("Active todos:\n- [pending] ship".into()),
        );
        assert_eq!(history[0].role, Role::System);
        assert_eq!(history[1].content.as_deref(), Some("do the thing"));
        assert_eq!(history[2].content.as_deref(), Some("working"));
        assert_eq!(history[3].kind, Some(MessageKind::CompactionSummary));
        assert!(history[4]
            .content
            .as_deref()
            .unwrap()
            .contains("system-reminder"));
    }

    #[test]
    fn assistant_with_tools_in_summary_format() {
        let messages = vec![Message::assistant_with_tools(
            None,
            vec![ToolCall {
                id: "1".into(),
                name: "Read".into(),
                arguments: "{}".into(),
            }],
        )];
        let formatted = format_messages_for_summary(&messages);
        assert!(formatted.contains("tool_call: Read"));
    }
}
