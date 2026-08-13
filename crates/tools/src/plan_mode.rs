use std::sync::Arc;

use parking_lot::Mutex;

/// Shared plan-mode flag for the main agent (and tool context).
#[derive(Debug, Clone, Default)]
pub struct PlanModeState {
    pub active: bool,
}

impl PlanModeState {
    pub fn enter(&mut self) {
        self.active = true;
    }

    pub fn exit(&mut self) {
        self.active = false;
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Whether `name` may run in the current mode.
    pub fn is_tool_allowed(&self, name: &str) -> bool {
        if !self.active {
            return true;
        }
        matches!(
            name,
            "Read"
                | "Grep"
                | "Glob"
                | "RepoMap"
                | "Skill"
                | "AskUserQuestion"
                | "TodoWrite"
                | "TodoList"
                | "FetchUrl"
                | "WebSearch"
                | "ExitPlanMode"
        )
    }

    pub fn blocked_message(tool_name: &str) -> String {
        format!(
            "Tool `{tool_name}` is blocked in plan mode. Only Read, Grep, Glob, RepoMap, and Skill are allowed until you call ExitPlanMode and the user approves your plan."
        )
    }
}

pub type SharedPlanMode = Arc<Mutex<PlanModeState>>;

pub fn shared_plan_mode() -> SharedPlanMode {
    Arc::new(Mutex::new(PlanModeState::default()))
}
