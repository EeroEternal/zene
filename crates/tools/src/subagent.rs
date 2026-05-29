use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SubagentProfile {
    #[default]
    Explore,
    Coder,
}

pub const DEFAULT_SUBAGENT_MAX_DEPTH: u32 = 1;

#[async_trait]
pub trait SubagentRunner: Send + Sync {
    async fn run_subagent(
        &self,
        prompt: &str,
        profile: SubagentProfile,
        cwd: Option<&Path>,
        parent_ctx: &crate::registry::ToolContext,
    ) -> anyhow::Result<String>;
}

#[derive(Clone)]
pub struct SubagentEnv {
    pub depth: u32,
    pub max_depth: u32,
    pub runner: Arc<dyn SubagentRunner>,
}

impl SubagentEnv {
    pub fn child(&self, depth: u32) -> Self {
        Self {
            depth,
            max_depth: self.max_depth,
            runner: Arc::clone(&self.runner),
        }
    }
}
