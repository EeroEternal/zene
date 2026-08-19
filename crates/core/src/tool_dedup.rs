use std::collections::hash_map::DefaultHasher;
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};

use serde_json::Value;

const HISTORY_LEN: usize = 16;

pub const DEDUP_REMINDER: &str = "<system-reminder>You just called this tool with the same arguments. Repeating the same tool call is usually unnecessary unless you received new information. Try a different approach.</system-reminder>";

#[derive(Debug, Clone)]
struct ToolCallRecord {
    name: String,
    args_hash: u64,
}

/// Tracks recent tool calls within a turn; detects consecutive duplicates.
#[derive(Debug, Default)]
pub struct ToolDedup {
    history: VecDeque<ToolCallRecord>,
}

impl ToolDedup {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.history.clear();
    }

    /// Returns reminder text if this call is a consecutive duplicate of the previous one.
    pub fn on_call(&mut self, name: &str, arguments: &str) -> Option<&'static str> {
        let hash = canonical_args_hash(arguments);
        let is_dup = self
            .history
            .back()
            .is_some_and(|prev| prev.name == name && prev.args_hash == hash);

        self.history.push_back(ToolCallRecord {
            name: name.to_string(),
            args_hash: hash,
        });
        if self.history.len() > HISTORY_LEN {
            self.history.pop_front();
        }

        if is_dup {
            Some(DEDUP_REMINDER)
        } else {
            None
        }
    }
}

fn canonical_args_hash(arguments: &str) -> u64 {
    let canonical = serde_json::from_str::<Value>(arguments)
        .ok()
        .map(canonicalize_value)
        .and_then(|v| serde_json::to_string(&v).ok())
        .unwrap_or_else(|| arguments.to_string());

    let mut hasher = DefaultHasher::new();
    canonical.hash(&mut hasher);
    hasher.finish()
}

fn canonicalize_value(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().cloned().collect();
            keys.sort();
            let ordered: serde_json::Map<String, Value> = keys
                .into_iter()
                .map(|k| {
                    let v = map.get(&k).cloned().unwrap_or(Value::Null);
                    (k, canonicalize_value(v))
                })
                .collect();
            Value::Object(ordered)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(canonicalize_value).collect()),
        other => other,
    }
}

pub fn append_reminder(content: &str, reminder: &str) -> String {
    if content.is_empty() {
        reminder.to_string()
    } else {
        format!("{content}\n\n{reminder}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consecutive_duplicate_gets_reminder() {
        let mut dedup = ToolDedup::new();
        let args = r#"{"path":"foo.rs"}"#;
        assert!(dedup.on_call("Read", args).is_none());
        assert_eq!(dedup.on_call("Read", args), Some(DEDUP_REMINDER));
        assert!(dedup.on_call("Read", r#"{"path":"bar.rs"}"#).is_none());
    }

    #[test]
    fn key_order_canonicalized() {
        let mut dedup = ToolDedup::new();
        assert!(dedup.on_call("Read", r#"{"path":"x"}"#).is_none());
        assert_eq!(
            dedup.on_call("Read", r#"{"path":"x"}"#),
            Some(DEDUP_REMINDER)
        );
        let mut dedup2 = ToolDedup::new();
        assert!(dedup2.on_call("Read", r#"{"path":"x"}"#).is_none());
        assert_eq!(dedup2.on_call("Read", r#"{"path":"x","offset":1}"#), None);
    }

    #[test]
    fn append_reminder_joins_content() {
        let out = append_reminder("hello", DEDUP_REMINDER);
        assert!(out.starts_with("hello"));
        assert!(out.contains("system-reminder"));
    }
}
