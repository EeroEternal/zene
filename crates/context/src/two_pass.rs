//! Prefire / two-pass compaction helpers (grok-build aligned).
//!
//! Pass1 summarizes ~95% of the compactable prefix → NOTE₁.
//! Pass2 merges NOTE₁ with the remaining ~5% tail into the final summary.

use std::hash::{Hash, Hasher};

use zene_llm::{Message, Role};

use crate::tokens::TokenEstimator;

/// Default history fraction covered by pass1.
pub const TWO_PASS_DEFAULT_SPLIT_FRACTION: f64 = 0.95;
const TWO_PASS_MAX_NOTE1_CHARS: usize = 12_000;

#[derive(Debug, Clone, Copy)]
pub struct TwoPassSplit {
    pub split_idx: usize,
}

/// Cheap fingerprint of a message prefix for prefire NOTE₁ validity.
pub fn fingerprint_messages(messages: &[Message]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    messages.len().hash(&mut h);
    for m in messages {
        let tag: u8 = match m.role {
            Role::System => 0,
            Role::User => 1,
            Role::Assistant => 2,
            Role::Tool => 3,
        };
        tag.hash(&mut h);
        if let Some(content) = &m.content {
            content.hash(&mut h);
        }
        if let Some(calls) = &m.tool_calls {
            for c in calls {
                c.id.hash(&mut h);
                c.name.hash(&mut h);
                c.arguments.hash(&mut h);
            }
        }
        if let Some(name) = &m.name {
            name.hash(&mut h);
        }
    }
    h.finish()
}

fn split_index_by_token_fraction(weights: &[u32], fraction: f64) -> usize {
    if weights.is_empty() {
        return 0;
    }
    let frac = fraction.clamp(0.05, 0.95);
    let total = weights.iter().copied().sum::<u32>().max(1);
    let target = frac * f64::from(total);
    let mut acc = 0u32;
    let mut split_idx = weights.len().saturating_sub(1).max(1);
    for (i, w) in weights.iter().enumerate() {
        acc = acc.saturating_add(*w);
        if f64::from(acc) >= target {
            split_idx = (i + 1).max(1);
            break;
        }
    }
    if split_idx >= weights.len() && weights.len() > 1 {
        split_idx = weights.len() - 1;
    }
    split_idx
}

fn snap_split_idx_to_tool_boundaries(messages: &[Message], mut split_idx: usize) -> usize {
    let n = messages.len();
    if n == 0 {
        return 0;
    }
    split_idx = split_idx.min(n);
    while split_idx < n && messages[split_idx].role == Role::Tool {
        split_idx += 1;
    }
    if split_idx < n
        && messages[split_idx].role == Role::Assistant
        && messages[split_idx]
            .tool_calls
            .as_ref()
            .is_some_and(|c| !c.is_empty())
    {
        split_idx += 1;
        while split_idx < n && messages[split_idx].role == Role::Tool {
            split_idx += 1;
        }
    }
    if split_idx >= n && n > 1 {
        split_idx = n - 1;
        while split_idx > 1 && messages[split_idx].role == Role::Tool {
            split_idx -= 1;
        }
    }
    split_idx.min(n)
}

/// Split compactable messages into pass1 prefix / pass2 tail.
pub fn split_messages_for_two_pass(
    messages: &[Message],
    estimator: &TokenEstimator,
    split_fraction: f64,
) -> TwoPassSplit {
    let weights: Vec<u32> = messages
        .iter()
        .map(|m| estimator.estimate_message_tokens(m))
        .collect();
    let mut split_idx = split_index_by_token_fraction(&weights, split_fraction);
    split_idx = snap_split_idx_to_tool_boundaries(messages, split_idx);
    TwoPassSplit {
        split_idx: split_idx.min(messages.len()),
    }
}

/// Cap NOTE₁ embedded into pass2.
pub fn note_for_pass2(pass1_raw: &str) -> String {
    let mut note = pass1_raw.trim().to_string();
    let n = note.chars().count();
    if n > TWO_PASS_MAX_NOTE1_CHARS {
        note = note.chars().take(TWO_PASS_MAX_NOTE1_CHARS).collect();
        note.push_str("\n\n[… NOTE₁ truncated for pass2 input budget …]");
    }
    note
}

pub fn pass2_user_prompt(note1: &str, hint: Option<&str>) -> String {
    let note1 = note_for_pass2(note1);
    let guidance =
        hint.unwrap_or("Merge prior summary with recent turns into one self-contained note.");
    format!(
        "This is a two-pass / hierarchical compaction.\n\
         You are writing the *final* compaction note a successor assistant will rely on.\n\n\
         Critical requirements:\n\
         - Incorporate the **entire** prior summary below — do not omit sections.\n\
         - Merge that prior summary with the more recent conversation turns above into \
         one coherent, faithful, self-contained summary.\n\
         - Preserve concrete values, file paths, errors/blockers, and pending tasks.\n\n\
         Prior summary to incorporate in full:\n\n\
         <summary_content>\n{note1}\n</summary_content>\n\n\
         Compaction guidance:\n{guidance}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use zene_llm::Message;

    #[test]
    fn split_leaves_tail() {
        let messages: Vec<_> = (0..10).map(|i| Message::user(format!("m{i}"))).collect();
        let split = split_messages_for_two_pass(&messages, &TokenEstimator::default(), 0.9);
        assert!(split.split_idx < messages.len());
        assert!(split.split_idx >= 1);
    }

    #[test]
    fn fingerprint_changes_with_content() {
        let a = vec![Message::user("hello")];
        let b = vec![Message::user("HELLO")];
        assert_ne!(fingerprint_messages(&a), fingerprint_messages(&b));
    }

    #[test]
    fn note_caps_length() {
        let huge = "x".repeat(20_000);
        let note = note_for_pass2(&huge);
        assert!(note.chars().count() < 13_000);
        assert!(note.contains("truncated"));
    }
}
