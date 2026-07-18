//! Bound large tool results before they enter session history (intra-lite).
//!
//! MCP already spills via `zene_mcp::truncate_mcp_output`. This covers Bash and
//! other verbose tools so they do not force premature auto-compact.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::warn;

/// Default inline cap for non-MCP tool output in session history (bytes).
pub const TOOL_MAX_OUTPUT_BYTES: usize = 30_000;

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

/// Truncate oversized tool output; spill full payload under `.zene/tool-output/`.
/// Skips MCP tools (handled by the MCP layer) and tiny results.
pub fn bound_tool_result(content: String, workdir: &Path, tool_name: &str) -> String {
    if tool_name.starts_with("mcp__") {
        return content;
    }
    let max = tool_max_output_bytes();
    if content.len() <= max {
        return content;
    }

    let dump_dir = workdir.join(".zene").join("tool-output");
    if let Err(err) = fs::create_dir_all(&dump_dir) {
        warn!(error = %err, "failed to create tool-output dir");
        return truncate_inline(&content, max, None);
    }

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let safe: String = tool_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let path = dump_dir.join(format!("{safe}-{ts}.txt"));
    if let Err(err) = fs::write(&path, &content) {
        warn!(error = %err, "failed to spill tool output");
        return truncate_inline(&content, max, None);
    }
    truncate_inline(&content, max, Some(path.display().to_string()))
}

fn truncate_inline(content: &str, max: usize, saved_path: Option<String>) -> String {
    let preview_budget = max.saturating_sub(220).max(256);
    let preview = match content.char_indices().nth(preview_budget) {
        Some((idx, _)) => &content[..idx],
        None => content,
    };
    let omitted = content.len().saturating_sub(preview.len());
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
    use tempfile::tempdir;

    #[test]
    fn skips_small_and_mcp() {
        let dir = tempdir().unwrap();
        assert_eq!(
            bound_tool_result("hi".into(), dir.path(), "Bash"),
            "hi"
        );
        let big = "x".repeat(40_000);
        let mcp = bound_tool_result(big.clone(), dir.path(), "mcp__s__t");
        assert_eq!(mcp.len(), big.len());
    }

    #[test]
    fn spills_bash() {
        let dir = tempdir().unwrap();
        std::env::set_var("ZENE_MAX_TOOL_OUTPUT_BYTES", "400");
        let out = bound_tool_result("y".repeat(2000), dir.path(), "Bash");
        std::env::remove_var("ZENE_MAX_TOOL_OUTPUT_BYTES");
        assert!(out.contains("truncated"));
        assert!(out.contains(".zene/tool-output/"));
    }
}
