//! IO adapters for spilling oversized tool output.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::warn;

use crate::output_bound::{format_bounded_output, tool_max_output_bytes, ToolOutputSpill};

/// Store spilled tool output (filesystem or remote). Runtime provides the impl.
pub trait ToolOutputStore: Send + Sync {
    /// Write full payload and return display path for inline reference.
    fn spill(&self, spill: &ToolOutputSpill) -> Option<String>;
}

/// Default store: `.zene/tool-output/` under the workspace.
pub struct FsToolOutputStore {
    workdir: PathBuf,
}

impl FsToolOutputStore {
    pub fn new(workdir: impl Into<PathBuf>) -> Self {
        Self {
            workdir: workdir.into(),
        }
    }
}

impl ToolOutputStore for FsToolOutputStore {
    fn spill(&self, spill: &ToolOutputSpill) -> Option<String> {
        let dump_dir = self.workdir.join(".zene").join("tool-output");
        if let Err(err) = std::fs::create_dir_all(&dump_dir) {
            warn!(error = %err, "failed to create tool-output dir");
            return None;
        }

        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let safe: String = spill
            .tool_name
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
        if let Err(err) = std::fs::write(&path, &spill.content) {
            warn!(error = %err, "failed to spill tool output");
            return None;
        }
        Some(path.display().to_string())
    }
}

/// Apply bound plan: spill if needed, then format inline session content.
pub fn apply_tool_bound_plan(
    plan: crate::output_bound::ToolBoundPlan,
    store: &dyn ToolOutputStore,
) -> String {
    match plan {
        crate::output_bound::ToolBoundPlan::Inline(content) => content,
        crate::output_bound::ToolBoundPlan::Spill(spill) => {
            let max = tool_max_output_bytes();
            let total = spill.total_bytes;
            let content = spill.content;
            let saved_path = store.spill(&ToolOutputSpill {
                tool_name: spill.tool_name,
                content: content.clone(),
                total_bytes: total,
            });
            format_bounded_output(&content, max, saved_path.as_deref(), total)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output_bound::{plan_tool_output_bound, ToolBoundPlan};
    use tempfile::tempdir;

    #[test]
    fn spills_bash_to_disk() {
        let dir = tempdir().unwrap();
        std::env::set_var("ZENE_MAX_TOOL_OUTPUT_BYTES", "400");
        let plan = plan_tool_output_bound("y".repeat(2000), "Bash");
        std::env::remove_var("ZENE_MAX_TOOL_OUTPUT_BYTES");
        let store = FsToolOutputStore::new(dir.path());
        let out = apply_tool_bound_plan(plan, &store);
        assert!(out.contains("truncated"));
        assert!(out.contains(".zene/tool-output/"));
    }

    #[test]
    fn inline_plan_passthrough() {
        let dir = tempdir().unwrap();
        let plan = ToolBoundPlan::Inline("hi".into());
        let store = FsToolOutputStore::new(dir.path());
        assert_eq!(apply_tool_bound_plan(plan, &store), "hi");
    }
}
