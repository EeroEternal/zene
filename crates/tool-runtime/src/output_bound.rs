//! Pure planning for large tool results before they enter session history.

/// Default inline cap for non-MCP tool output in session history (bytes).
pub const TOOL_MAX_OUTPUT_BYTES: usize = 30_000;

const ENV_TOOL_OUTPUT_HANDLES: &str = "ZENE_TOOL_OUTPUT_HANDLES";

/// When true, spilled tool output is referenced by handle only (no large inline preview).
///
/// Defaults **on**: commit-time shaping (#130) prefers a short immutable handle
/// over a large inline body that compaction would later rewrite (BodyMutate).
/// Set `ZENE_TOOL_OUTPUT_HANDLES=0` to restore the inline-preview fallback.
pub fn tool_output_handles_enabled() -> bool {
    std::env::var(ENV_TOOL_OUTPUT_HANDLES)
        .ok()
        .map(|v| {
            !matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            )
        })
        .unwrap_or(true)
}

pub fn tool_max_output_bytes() -> usize {
    for key in ["ZENE_MAX_TOOL_OUTPUT_BYTES", "MAX_TOOL_OUTPUT_BYTES"] {
        if let Ok(raw) = std::env::var(key) {
            if let Ok(n) = raw.trim().parse::<usize>() {
                if n > 0 {
                    return n;
                }
            }
        }
    }
    TOOL_MAX_OUTPUT_BYTES
}

/// Full payload to spill when inline cap is exceeded.
#[derive(Debug, Clone)]
pub struct ToolOutputSpill {
    pub tool_name: String,
    pub content: String,
    pub total_bytes: usize,
}

/// Result of planning tool output bounds (no IO).
#[derive(Debug, Clone)]
pub enum ToolBoundPlan {
    /// Content fits inline; no spill needed.
    Inline(String),
    /// Content exceeds cap; runtime should spill then format inline preview.
    Spill(ToolOutputSpill),
}

/// Plan how to bound tool output. Skips MCP tools (handled by MCP layer).
pub fn plan_tool_output_bound(content: String, tool_name: &str) -> ToolBoundPlan {
    if tool_name.starts_with("mcp__") {
        return ToolBoundPlan::Inline(content);
    }
    let max = tool_max_output_bytes();
    if content.len() <= max {
        return ToolBoundPlan::Inline(content);
    }
    let total_bytes = content.len();
    ToolBoundPlan::Spill(ToolOutputSpill {
        tool_name: tool_name.to_string(),
        content,
        total_bytes,
    })
}

/// Format inline preview after optional spill path is known.
pub fn format_bounded_output(
    content: &str,
    max_bytes: usize,
    saved_path: Option<&str>,
    total_bytes: usize,
) -> String {
    if tool_output_handles_enabled() {
        if let Some(path) = saved_path {
            return format!("[zene-tool-output path=\"{path}\" bytes={total_bytes}]");
        }
    }
    let preview_budget = max_bytes.saturating_sub(220).max(256);
    let preview = match content.char_indices().nth(preview_budget) {
        Some((idx, _)) => &content[..idx],
        None => content,
    };
    let omitted = total_bytes.saturating_sub(preview.len());
    match saved_path {
        Some(path) => format!(
            "{preview}\n\n[truncated {omitted} bytes; full output saved to {path}. Use Read if needed.]"
        ),
        None => format!("{preview}\n\n[truncated {omitted} bytes]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn skips_small_and_mcp() {
        assert!(matches!(
            plan_tool_output_bound("hi".into(), "Bash"),
            ToolBoundPlan::Inline(_)
        ));
        let big = "x".repeat(40_000);
        assert!(matches!(
            plan_tool_output_bound(big.clone(), "mcp__s__t"),
            ToolBoundPlan::Inline(_)
        ));
    }

    #[test]
    fn plans_spill_for_large_bash() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("ZENE_MAX_TOOL_OUTPUT_BYTES", "400");
        let plan = plan_tool_output_bound("y".repeat(2000), "Bash");
        std::env::remove_var("ZENE_MAX_TOOL_OUTPUT_BYTES");
        assert!(matches!(plan, ToolBoundPlan::Spill(_)));
    }

    #[test]
    fn handles_default_on_without_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("ZENE_TOOL_OUTPUT_HANDLES");
        assert!(tool_output_handles_enabled());
        let out = format_bounded_output("yyyy", 400, Some("/tmp/out.txt"), 2000);
        assert!(out.starts_with("[zene-tool-output path="));
        assert!(!out.contains("yyyy"));
    }

    #[test]
    fn handles_can_be_disabled() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("ZENE_TOOL_OUTPUT_HANDLES", "0");
        let out = format_bounded_output("yyyy", 400, Some("/tmp/out.txt"), 2000);
        std::env::remove_var("ZENE_TOOL_OUTPUT_HANDLES");
        assert!(out.contains("yyyy"));
        assert!(out.contains("[truncated"));
    }

    #[test]
    fn handle_only_when_enabled() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("ZENE_TOOL_OUTPUT_HANDLES", "1");
        let out = format_bounded_output("yyyy", 400, Some("/tmp/out.txt"), 2000);
        std::env::remove_var("ZENE_TOOL_OUTPUT_HANDLES");
        assert!(out.starts_with("[zene-tool-output path="));
        assert!(out.contains("bytes=2000"));
        assert!(!out.contains("yyyy"));
    }
}
