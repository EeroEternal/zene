use std::sync::Arc;

use anyhow::{bail, Result};
use zene_config::{AgentProfile, WebSearchConfig};

use crate::builtin::{agent_tools, tools_for_profile};
use crate::registry::ToolRegistry;
use crate::subagent::{SubagentEnv, SubagentProfile, SubagentRunner, DEFAULT_SUBAGENT_MAX_DEPTH};

/// Capability boundary for a main-agent or child runtime turn.
///
/// Wave 14: construction goes through a scope so profile, nesting depth, and
/// tool catalog stay one injection point. ToolPolicy/SessionPolicy land later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeScope {
    kind: RuntimeKind,
    pub depth: u32,
    pub max_depth: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RuntimeKind {
    Agent {
        profile: AgentProfile,
        web_search: WebSearchConfig,
    },
    Subagent {
        profile: SubagentProfile,
    },
}

impl RuntimeScope {
    /// Root scope for the main Agent (depth `0`).
    ///
    /// Uses [`agent_tools`] — Explore/Coder agent boxes differ from Subagent
    /// profile boxes (collaboration / plan / Task).
    pub fn agent(profile: AgentProfile, web_search: WebSearchConfig) -> Self {
        Self {
            kind: RuntimeKind::Agent {
                profile,
                web_search,
            },
            depth: 0,
            max_depth: DEFAULT_SUBAGENT_MAX_DEPTH,
        }
    }

    /// Build a child scope under `parent_depth`. Depth `0` is the main agent.
    pub fn subagent(profile: SubagentProfile, parent_depth: u32) -> Result<Self> {
        let depth = parent_depth.saturating_add(1);
        if depth > DEFAULT_SUBAGENT_MAX_DEPTH {
            bail!("Subagent nesting limit reached (max depth {DEFAULT_SUBAGENT_MAX_DEPTH})");
        }
        Ok(Self {
            kind: RuntimeKind::Subagent { profile },
            depth,
            max_depth: DEFAULT_SUBAGENT_MAX_DEPTH,
        })
    }

    /// Subagent profile when this scope was built via [`Self::subagent`].
    pub fn subagent_profile(&self) -> Option<SubagentProfile> {
        match self.kind {
            RuntimeKind::Subagent { profile } => Some(profile),
            RuntimeKind::Agent { .. } => None,
        }
    }

    pub fn tools(&self) -> ToolRegistry {
        match &self.kind {
            RuntimeKind::Agent {
                profile,
                web_search,
            } => agent_tools(*profile, web_search.clone()),
            RuntimeKind::Subagent { profile } => tools_for_profile(*profile),
        }
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

    #[test]
    fn agent_scope_is_root_depth_and_uses_agent_tool_boxes() {
        let scope = RuntimeScope::agent(AgentProfile::Explore, WebSearchConfig::default());
        assert_eq!(scope.depth, 0);
        assert!(scope.subagent_profile().is_none());
        let names: Vec<_> = scope
            .definitions()
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        assert!(names.contains(&"Read".into()));
        assert!(names.contains(&"AskUserQuestion".into()));
        assert!(!names.iter().any(|name| name == "Write" || name == "Edit" || name == "Bash"));
        // Agent Explore includes plan/collaboration; Subagent Explore does not include Task.
        assert!(!names.iter().any(|name| name == "Task"));
    }

    #[test]
    fn agent_coder_scope_includes_task() {
        let scope = RuntimeScope::agent(AgentProfile::Coder, WebSearchConfig::default());
        let names: Vec<_> = scope
            .definitions()
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        assert!(names.contains(&"Write".into()));
        assert!(names.contains(&"Task".into()));
    }
}
