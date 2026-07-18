//! Bound MCP tool output so large payloads do not flood the context window.
//!
//! Aligned with grok-build's MCP truncate-to-disk: keep a short inline preview,
//! spill the remainder under `.zene/tool-output/`, and steer the model to Read
//! the saved file.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::warn;

/// Default inline cap for MCP tool output (bytes).
pub const MCP_MAX_OUTPUT_BYTES: usize = 20_000;

/// Env overrides (first wins): `ZENE_MAX_MCP_OUTPUT_BYTES`, `MAX_MCP_OUTPUT_BYTES`.
pub fn mcp_max_output_bytes() -> usize {
    for key in ["ZENE_MAX_MCP_OUTPUT_BYTES", "MAX_MCP_OUTPUT_BYTES"] {
        if let Ok(raw) = std::env::var(key) {
            if let Ok(n) = raw.trim().parse::<usize>() {
                if n > 0 {
                    return n;
                }
            }
        }
    }
    MCP_MAX_OUTPUT_BYTES
}

/// Truncate `content` if over the byte cap; spill the full payload to disk.
pub fn truncate_mcp_output(content: String, workdir: &Path, tool_name: &str) -> String {
    let max = mcp_max_output_bytes();
    if content.len() <= max {
        return content;
    }

    let dump_dir = workdir.join(".zene").join("tool-output");
    if let Err(err) = fs::create_dir_all(&dump_dir) {
        warn!(error = %err, "failed to create MCP tool-output dir");
        return truncate_inline(&content, max, None);
    }

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let safe_name = tool_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect::<String>();
    let ext = if content.trim_start().starts_with('{') || content.trim_start().starts_with('[') {
        "json"
    } else {
        "txt"
    };
    let path = dump_dir.join(format!("{safe_name}-{ts}.{ext}"));
    if let Err(err) = fs::write(&path, &content) {
        warn!(error = %err, "failed to spill MCP output");
        return truncate_inline(&content, max, None);
    }

    truncate_inline(&content, max, Some(path.display().to_string()))
}

fn truncate_inline(content: &str, max: usize, saved_path: Option<String>) -> String {
    let preview_budget = max.saturating_sub(200).max(256);
    let preview = match content.char_indices().nth(preview_budget) {
        Some((idx, _)) => &content[..idx],
        None => content,
    };
    let omitted = content.len().saturating_sub(preview.len());
    match saved_path {
        Some(path) => format!(
            "{preview}\n\n[truncated {omitted} bytes; full output saved to {path}. Use the Read tool on that path if you need more.]"
        ),
        None => format!(
            "{preview}\n\n[truncated {omitted} bytes; full MCP output omitted to protect context]"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn leaves_small_output_alone() {
        let dir = tempdir().unwrap();
        let out = truncate_mcp_output("hello".into(), dir.path(), "mcp__s__t");
        assert_eq!(out, "hello");
    }

    #[test]
    fn spills_large_output() {
        let dir = tempdir().unwrap();
        std::env::set_var("ZENE_MAX_MCP_OUTPUT_BYTES", "500");
        let big = "x".repeat(2000);
        let out = truncate_mcp_output(big, dir.path(), "mcp__demo__tool");
        std::env::remove_var("ZENE_MAX_MCP_OUTPUT_BYTES");
        assert!(out.contains("truncated"));
        assert!(out.contains(".zene/tool-output/"));
        let entries: Vec<_> = fs::read_dir(dir.path().join(".zene/tool-output"))
            .unwrap()
            .collect();
        assert_eq!(entries.len(), 1);
    }
}
