//! Re-export permission types from `zene-permission`.

pub use zene_permission::{
    approve_tool_call, policy_denied, session_approval_key, PermissionGate, PermissionMode,
    PermissionPrompter, PermissionRule, PromptChoice, RuleAction, SharedToolPermission,
    ToolPermission,
};
