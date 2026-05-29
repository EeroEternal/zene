mod bash;
mod builtin;
mod edit;
mod glob;
mod grep;
mod line_endings;
mod permission;
mod plan;
mod plan_mode;
mod read;
mod registry;
mod skill;
mod subagent;
mod task;
mod write;

pub use builtin::{builtin_tools, tools_for_profile};
pub use subagent::{
    SubagentEnv, SubagentProfile, SubagentRunner, DEFAULT_SUBAGENT_MAX_DEPTH,
};
pub use line_endings::{
    detect_line_ending_style, make_carriage_returns_visible, materialize_model_text,
    to_model_text_view, LineEndingStyle, ModelTextView,
};
pub use permission::{SharedToolPermission, ToolPermission};
pub use plan_mode::{shared_plan_mode, PlanModeState, SharedPlanMode};
pub use registry::{Tool, ToolContext, ToolRegistry, ToolResult};
