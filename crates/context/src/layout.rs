//! Outbound projection layout: frozen prefix, append-only body, tail decorations.
//!
//! Prefix cache only survives when bytes to the left of the first change stay
//! identical. Volatile reminders therefore belong at the tail, never between
//! the system prefix and conversation history. See `docs/context-engine.md`.

use serde::{Deserialize, Serialize};
use zene_llm::{Message, Role};

use crate::assemble::stable_system_boundary;
use crate::two_pass::fingerprint_messages;

const REMINDER_OPEN_HYPHEN: &str = "<system-reminder>";
const REMINDER_OPEN_UNDERSCORE: &str = "<system_reminder>";

/// Why the stable prefix likely missed provider cache versus the previous call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PrefixCacheBreakKind {
    #[default]
    None,
    Compact,
    SystemResize,
    InjectedResize,
    BodyMutate,
    Unknown,
}

impl PrefixCacheBreakKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Compact => "compact",
            Self::SystemResize => "system_resize",
            Self::InjectedResize => "injected_resize",
            Self::BodyMutate => "body_mutate",
            Self::Unknown => "unknown",
        }
    }
}

/// Zone boundaries of one outbound `messages[]` (indices are exclusive ends).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProjectionLayout {
    pub prefix_end: usize,
    pub body_end: usize,
    pub tail_decoration_count: usize,
}

impl ProjectionLayout {
    pub fn tail_start(&self) -> usize {
        self.body_end
    }
}

/// Cache-oriented explain fields for one projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PrefixCacheExplain {
    pub prefix_end: usize,
    pub body_end: usize,
    pub tail_decoration_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix_fingerprint: Option<String>,
    pub break_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unchanged_reprocessed_est: Option<u64>,
}

/// Where a new model-visible injection may live. `BodyInsert` is intentionally
/// omitted — that is the cache-killing `<agent_documents_index>` pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectionZone {
    FrozenPrefix,
    TailDecorations,
}

pub fn is_step_decoration(message: &Message) -> bool {
    if message.role != Role::User {
        return false;
    }
    message.content.as_deref().is_some_and(content_is_reminder)
}

pub fn content_is_reminder(content: &str) -> bool {
    content.contains(REMINDER_OPEN_HYPHEN) || content.contains(REMINDER_OPEN_UNDERSCORE)
}

pub fn split_layout(messages: &[Message]) -> ProjectionLayout {
    let prefix_end = stable_system_boundary(messages).min(messages.len());
    let mut tail_decoration_count = 0usize;
    while tail_decoration_count < messages.len().saturating_sub(prefix_end)
        && is_step_decoration(&messages[messages.len() - 1 - tail_decoration_count])
    {
        tail_decoration_count += 1;
    }
    let body_end = messages.len().saturating_sub(tail_decoration_count);
    ProjectionLayout {
        prefix_end,
        body_end,
        tail_decoration_count,
    }
}

/// Drop trailing live decorations, then append a single reminder from `sections`.
///
/// Historical reminders that sit in the body (from older compact snapshots) are
/// left untouched so their bytes stay frozen.
pub fn apply_tail_decorations(messages: &mut Vec<Message>, sections: &[String]) {
    while messages.last().is_some_and(is_step_decoration) {
        messages.pop();
    }
    let mut parts = Vec::new();
    for section in sections {
        let trimmed = inner_reminder_text(section.trim());
        if !trimmed.is_empty() {
            parts.push(trimmed);
        }
    }
    if parts.is_empty() {
        return;
    }
    messages.push(Message::user(format!(
        "{REMINDER_OPEN_HYPHEN}\n{}\n</system-reminder>",
        parts.join("\n\n")
    )));
}

pub fn prefix_fingerprint(messages: &[Message], prefix_end: usize) -> Option<String> {
    let end = prefix_end.min(messages.len());
    if end == 0 {
        return None;
    }
    Some(format!("{:016x}", fingerprint_messages(&messages[..end])))
}

/// Index of a live reminder sitting immediately after the pinned prefix with
/// conversation still following it (the DeepSeek msg[1] pattern).
pub fn prefix_adjacent_decoration_index(messages: &[Message]) -> Option<usize> {
    let prefix_end = stable_system_boundary(messages).min(messages.len());
    if prefix_end < messages.len()
        && is_step_decoration(&messages[prefix_end])
        && messages.len() > prefix_end + 1
    {
        Some(prefix_end)
    } else {
        None
    }
}

/// Move prefix-adjacent reminders to the end of the outbound list.
///
/// Does not rewrite Session facts. `apply_tail_decorations` then replaces any
/// trailing reminders with the current step's decorations.
pub fn relocate_prefix_adjacent_decorations(messages: &mut Vec<Message>) -> usize {
    let prefix_end = stable_system_boundary(messages).min(messages.len());
    let mut moved = 0usize;
    while prefix_end < messages.len()
        && is_step_decoration(&messages[prefix_end])
        && messages.len() > prefix_end + 1
    {
        let decoration = messages.remove(prefix_end);
        messages.push(decoration);
        moved += 1;
    }
    moved
}

pub fn classify_prefix_break(
    previous_fingerprint: Option<&str>,
    current_fingerprint: Option<&str>,
    epoch_bumped: bool,
    epoch_reason: Option<&str>,
) -> PrefixCacheBreakKind {
    match (previous_fingerprint, current_fingerprint) {
        (None, _) => PrefixCacheBreakKind::Unknown,
        (_, None) => PrefixCacheBreakKind::Unknown,
        (Some(prev), Some(cur)) if prev == cur => PrefixCacheBreakKind::None,
        (Some(_), Some(_)) if epoch_bumped => match epoch_reason {
            Some("compaction") | Some("manual_compaction") | Some("overflow_compaction") => {
                PrefixCacheBreakKind::Compact
            }
            Some(reason) if reason.contains("system") || reason == "plan_mode" => {
                PrefixCacheBreakKind::SystemResize
            }
            _ => PrefixCacheBreakKind::Compact,
        },
        (Some(_), Some(_)) => PrefixCacheBreakKind::InjectedResize,
    }
}

fn inner_reminder_text(section: &str) -> String {
    let trimmed = section.trim();
    for (open, close) in [
        (REMINDER_OPEN_HYPHEN, "</system-reminder>"),
        (REMINDER_OPEN_UNDERSCORE, "</system_reminder>"),
    ] {
        if let Some(start) = trimmed.find(open) {
            let inner_start = start + open.len();
            let inner = if let Some(rel) = trimmed[inner_start..].find(close) {
                &trimmed[inner_start..inner_start + rel]
            } else {
                &trimmed[inner_start..]
            };
            return inner.trim().to_string();
        }
    }
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zene_llm::Message;

    #[test]
    fn decorations_only_count_at_tail() {
        let messages = vec![
            Message::system("sys"),
            Message::user("hello"),
            Message::assistant("ok"),
            Message::user("<system-reminder>\nActive todos\n</system-reminder>"),
        ];
        let layout = split_layout(&messages);
        assert_eq!(layout.prefix_end, 1);
        assert_eq!(layout.body_end, 3);
        assert_eq!(layout.tail_decoration_count, 1);
    }

    #[test]
    fn mid_body_reminder_stays_in_body_zone() {
        let messages = vec![
            Message::system("sys"),
            Message::user("hello"),
            Message::user("<system-reminder>\nold compact note\n</system-reminder>"),
            Message::assistant("go"),
        ];
        let layout = split_layout(&messages);
        assert_eq!(layout.prefix_end, 1);
        assert_eq!(layout.body_end, 4);
        assert_eq!(layout.tail_decoration_count, 0);
        assert!(prefix_adjacent_decoration_index(&messages).is_none());
        let mut cloned = messages.clone();
        assert_eq!(relocate_prefix_adjacent_decorations(&mut cloned), 0);
        assert!(cloned[2]
            .content
            .as_deref()
            .unwrap()
            .contains("old compact"));
    }

    #[test]
    fn relocates_index_block_after_system() {
        let mut messages = vec![
            Message::system("sys"),
            Message::user("<system-reminder>\n<agent_documents_index>\n</system-reminder>"),
            Message::user("hello"),
            Message::assistant("ok"),
        ];
        let prefix = prefix_fingerprint(&messages, 1);
        assert_eq!(prefix_adjacent_decoration_index(&messages), Some(1));
        assert_eq!(relocate_prefix_adjacent_decorations(&mut messages), 1);
        assert!(prefix_adjacent_decoration_index(&messages).is_none());
        assert_eq!(messages[1].content.as_deref(), Some("hello"));
        assert!(is_step_decoration(messages.last().unwrap()));
        assert_eq!(prefix, prefix_fingerprint(&messages, 1));
        apply_tail_decorations(&mut messages, &["fresh tail".into()]);
        assert!(prefix_adjacent_decoration_index(&messages).is_none());
        assert!(messages
            .last()
            .unwrap()
            .content
            .as_deref()
            .unwrap()
            .contains("fresh tail"));
        assert!(!messages.iter().any(|m| m
            .content
            .as_deref()
            .is_some_and(|c| c.contains("agent_documents_index"))));
    }

    #[test]
    fn apply_replaces_trailing_decoration_without_touching_prefix() {
        let mut messages = vec![
            Message::system("sys"),
            Message::user("hello"),
            Message::user("<system-reminder>\nold todos\n</system-reminder>"),
        ];
        let before = prefix_fingerprint(&messages, 1);
        apply_tail_decorations(&mut messages, &["Active todos:\n- [pending] ship".into()]);
        let after = prefix_fingerprint(&messages, 1);
        assert_eq!(before, after);
        let layout = split_layout(&messages);
        assert_eq!(layout.tail_decoration_count, 1);
        assert!(messages
            .last()
            .unwrap()
            .content
            .as_deref()
            .unwrap()
            .contains("ship"));
        assert!(!messages
            .last()
            .unwrap()
            .content
            .as_deref()
            .unwrap()
            .contains("old todos"));
    }

    #[test]
    fn changing_tail_does_not_change_prefix_fingerprint() {
        let mut messages = vec![
            Message::system("frozen"),
            Message::compaction_summary("summary"),
            Message::user("task"),
        ];
        let first = prefix_fingerprint(&messages, split_layout(&messages).prefix_end);
        apply_tail_decorations(&mut messages, &["plan mode on".into()]);
        let second = prefix_fingerprint(&messages, split_layout(&messages).prefix_end);
        apply_tail_decorations(&mut messages, &["plan mode off".into()]);
        let third = prefix_fingerprint(&messages, split_layout(&messages).prefix_end);
        assert_eq!(first, second);
        assert_eq!(second, third);
    }

    #[test]
    fn classify_same_prefix_as_none() {
        assert_eq!(
            classify_prefix_break(Some("aaa"), Some("aaa"), false, None),
            PrefixCacheBreakKind::None
        );
    }

    #[test]
    fn classify_epoch_compaction_as_compact() {
        assert_eq!(
            classify_prefix_break(Some("aaa"), Some("bbb"), true, Some("compaction")),
            PrefixCacheBreakKind::Compact
        );
    }

    #[test]
    fn classify_prefix_change_without_epoch_as_injected_resize() {
        assert_eq!(
            classify_prefix_break(Some("aaa"), Some("bbb"), false, None),
            PrefixCacheBreakKind::InjectedResize
        );
    }

    #[test]
    fn classify_system_prefix_reason_as_system_resize() {
        assert_eq!(
            classify_prefix_break(Some("aaa"), Some("bbb"), true, Some("system_prefix")),
            PrefixCacheBreakKind::SystemResize
        );
    }

    #[test]
    fn injection_zone_has_no_body_insert() {
        match InjectionZone::TailDecorations {
            InjectionZone::FrozenPrefix | InjectionZone::TailDecorations => {}
        }
    }
}
