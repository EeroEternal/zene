use std::sync::Arc;

use anyhow::{bail, Result};

use crate::builtin::tools_for_profile;
use crate::registry::{ToolCatalog, ToolRegistry};
use crate::subagent::{SubagentEnv, SubagentProfile, SubagentRunner, DEFAULT_SUBAGENT_MAX_DEPTH};

/// Capability boundary for a child/runtime turn.
///
/// Wave 14 first slice: Subagent construction goes through a scope so profile,
/// nesting depth, and tool catalog stay one injection point. Full-agent scopes
/// and ToolPolicy/SessionPolicy land in later slices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeScope {
    pub profile: SubagentProfile,
    pub depth: u32,
    pub max_depth: u32,
}

impl RuntimeScope {
    /// Build a child scope under `parent_depth`. Depth `0` is the main agent.
    pub fn subagent(profile: SubagentProfile, parent_depth: u32) -> Result<Self> {
        let depth = parent_depth.saturating_add(1);
        if depth > DEFAULT_SUBAGENT_MAX_DEPTH {
            bail!("Subagent nesting limit reached (max depth {DEFAULT_SUBAGENT_MAX_DEPTH})");
        }
        Ok(Self {
            profile,
            depth,
            max_depth: DEFAULT_SUBAGENT_MAX_DEPTH,
        })
    }

    pub fn tools(&self) -> ToolRegistry {
        tools_for_profile(self.profile)
    }

    pub fn catalog(&self) -> ToolRegistry {
        self.tools()
    }

    pub fn definitions(&self) -> Vec<zene_llm::ToolDefinition> {
        self.catalog().definitions()
    }

    pub fn env(&self, runner: Arc<dyn SubagentRunner>) -> SubagentEnv {
        SubagentEnv {
            depth: self.depth,
            max_depth: self.max_depth,
            runner,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explore_scope_exposes_read_only_catalog() {
        let scope = RuntimeScope::subagent(SubagentProfile::Explore, 0).expect("scope");
        assert_eq!(scope.depth, 1);
        let names: Vec<_> = scope
            .definitions()
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        assert!(names.contains(&"Read".into()));
        assert!(names.contains(&"Grep".into()));
        assert!(!names.iter().any(|name| name == "Write" || name == "Edit"));
    }

    #[test]
    fn coder_scope_includes_write_tools() {
        let scope = RuntimeScope::subagent(SubagentProfile::Coder, 0).expect("scope");
        let names: Vec<_> = scope
            .definitions()
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        assert!(names.contains(&"Write".into()));
        assert!(names.contains(&"Bash".into()));
    }

    #[test]
    fn nested_scope_rejects_over_max_depth() {
        let err = RuntimeScope::subagent(SubagentProfile::Explore, DEFAULT_SUBAGENT_MAX_DEPTH)
            .expect_err("over max");
        assert!(err.to_string().contains("nesting limit"));
    }
}
