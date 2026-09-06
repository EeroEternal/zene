use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Hook lifecycle event (aligned with `hooks.json` / config.toml and host extensions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    BeforeAgentStart,
    SessionBeforeCompact,
    ContextMutate,
}

impl HookEvent {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "PreToolUse" | "pre_tool_use" => Some(Self::PreToolUse),
            "PostToolUse" | "post_tool_use" => Some(Self::PostToolUse),
            "BeforeAgentStart" | "before_agent_start" => Some(Self::BeforeAgentStart),
            "SessionBeforeCompact" | "session_before_compact" => Some(Self::SessionBeforeCompact),
            "ContextMutate" | "context_mutate" => Some(Self::ContextMutate),
            _ => None,
        }
    }

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::PreToolUse => "PreToolUse",
            Self::PostToolUse => "PostToolUse",
            Self::BeforeAgentStart => "BeforeAgentStart",
            Self::SessionBeforeCompact => "SessionBeforeCompact",
            Self::ContextMutate => "ContextMutate",
        }
    }
}

impl std::fmt::Display for HookEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Structured input payload delivered to hook runners and extensions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookPayload {
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u64>,
}

impl HookPayload {
    pub fn pre_tool_use(tool: impl Into<String>, args: impl Into<String>) -> Self {
        Self {
            event: HookEvent::PreToolUse.to_string(),
            tool: Some(tool.into()),
            args: Some(args.into()),
            session_id: None,
            prompt: None,
            reason: None,
            tokens: None,
            epoch: None,
        }
    }

    pub fn post_tool_use(tool: impl Into<String>, args: impl Into<String>) -> Self {
        Self {
            event: HookEvent::PostToolUse.to_string(),
            tool: Some(tool.into()),
            args: Some(args.into()),
            session_id: None,
            prompt: None,
            reason: None,
            tokens: None,
            epoch: None,
        }
    }

    pub fn before_agent_start(session_id: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            event: HookEvent::BeforeAgentStart.to_string(),
            tool: None,
            args: None,
            session_id: Some(session_id.into()),
            prompt: Some(prompt.into()),
            reason: None,
            tokens: None,
            epoch: None,
        }
    }

    pub fn session_before_compact(
        session_id: impl Into<String>,
        reason: impl Into<String>,
        tokens: u32,
    ) -> Self {
        Self {
            event: HookEvent::SessionBeforeCompact.to_string(),
            tool: None,
            args: None,
            session_id: Some(session_id.into()),
            prompt: None,
            reason: Some(reason.into()),
            tokens: Some(tokens),
            epoch: None,
        }
    }

    pub fn context_mutate(session_id: impl Into<String>, epoch: u64) -> Self {
        Self {
            event: HookEvent::ContextMutate.to_string(),
            tool: None,
            args: None,
            session_id: Some(session_id.into()),
            prompt: None,
            reason: None,
            tokens: None,
            epoch: Some(epoch),
        }
    }
}

/// Planned hook invocation (no IO).
#[derive(Debug, Clone)]
pub struct HookRunRequest {
    pub command: String,
    pub stdin_json: String,
    /// When true, non-zero exit becomes a block (PreToolUse, BeforeAgentStart).
    pub blocking: bool,
}

/// One configured hook entry.
#[derive(Debug, Clone)]
pub struct HookSpec {
    pub event: String,
    pub command: String,
}

/// Pure hook registry and run planning.
#[derive(Debug, Clone)]
pub struct HookEngine {
    hooks: Vec<HookSpec>,
}

impl HookEngine {
    pub fn new(hooks: Vec<HookSpec>) -> Self {
        Self { hooks }
    }

    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    pub fn extend(&mut self, hooks: Vec<HookSpec>) {
        self.hooks.extend(hooks);
    }

    pub fn plan_pre_tool_use(&self, tool: &str, args: &str) -> Result<Vec<HookRunRequest>> {
        let payload = HookPayload::pre_tool_use(tool, args);
        self.plan_for_event(HookEvent::PreToolUse, &payload, true)
    }

    pub fn plan_post_tool_use(&self, tool: &str, args: &str) -> Result<Vec<HookRunRequest>> {
        let payload = HookPayload::post_tool_use(tool, args);
        self.plan_for_event(HookEvent::PostToolUse, &payload, false)
    }

    pub fn plan_before_agent_start(
        &self,
        session_id: &str,
        prompt: &str,
    ) -> Result<Vec<HookRunRequest>> {
        let payload = HookPayload::before_agent_start(session_id, prompt);
        self.plan_for_event(HookEvent::BeforeAgentStart, &payload, true)
    }

    pub fn plan_session_before_compact(
        &self,
        session_id: &str,
        reason: &str,
        tokens: u32,
    ) -> Result<Vec<HookRunRequest>> {
        let payload = HookPayload::session_before_compact(session_id, reason, tokens);
        self.plan_for_event(HookEvent::SessionBeforeCompact, &payload, false)
    }

    pub fn plan_context_mutate(&self, session_id: &str, epoch: u64) -> Result<Vec<HookRunRequest>> {
        let payload = HookPayload::context_mutate(session_id, epoch);
        self.plan_for_event(HookEvent::ContextMutate, &payload, false)
    }

    pub fn plan_for_event(
        &self,
        event: HookEvent,
        payload: &HookPayload,
        blocking: bool,
    ) -> Result<Vec<HookRunRequest>> {
        let stdin_json = serde_json::to_string(payload).context("serialize hook payload")?;
        Ok(self
            .hooks
            .iter()
            .filter(|hook| HookEvent::parse(&hook.event) == Some(event))
            .map(|hook| HookRunRequest {
                command: hook.command.clone(),
                stdin_json: stdin_json.clone(),
                blocking,
            })
            .collect())
    }
}

pub fn build_hook_input(tool: &str, args: &str) -> Result<String> {
    let payload = HookPayload::pre_tool_use(tool, args);
    serde_json::to_string(&payload).context("serialize hook input")
}

pub fn hook_failure_reason(stderr: &[u8], stdout: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = String::from_utf8_lossy(stdout).trim().to_string();
    if !stdout.is_empty() {
        return stdout;
    }
    "hook exited with non-zero status".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_event_parsing() {
        assert_eq!(HookEvent::parse("PreToolUse"), Some(HookEvent::PreToolUse));
        assert_eq!(
            HookEvent::parse("PostToolUse"),
            Some(HookEvent::PostToolUse)
        );
        assert_eq!(
            HookEvent::parse("before_agent_start"),
            Some(HookEvent::BeforeAgentStart)
        );
        assert_eq!(
            HookEvent::parse("SessionBeforeCompact"),
            Some(HookEvent::SessionBeforeCompact)
        );
        assert_eq!(
            HookEvent::parse("context_mutate"),
            Some(HookEvent::ContextMutate)
        );
        assert_eq!(HookEvent::parse("Unknown"), None);
    }

    #[test]
    fn plans_matching_hooks() {
        let engine = HookEngine::new(vec![
            HookSpec {
                event: "PreToolUse".into(),
                command: "echo pre".into(),
            },
            HookSpec {
                event: "PostToolUse".into(),
                command: "echo post".into(),
            },
            HookSpec {
                event: "BeforeAgentStart".into(),
                command: "echo start".into(),
            },
            HookSpec {
                event: "SessionBeforeCompact".into(),
                command: "echo compact".into(),
            },
            HookSpec {
                event: "ContextMutate".into(),
                command: "echo mutate".into(),
            },
        ]);
        let pre = engine
            .plan_pre_tool_use("Read", r#"{"path":"a"}"#)
            .expect("plan pre");
        assert_eq!(pre.len(), 1);
        assert!(pre[0].blocking);
        let post = engine
            .plan_post_tool_use("Read", r#"{"path":"a"}"#)
            .expect("plan post");
        assert_eq!(post.len(), 1);
        assert!(!post[0].blocking);

        let start = engine
            .plan_before_agent_start("sess-1", "hello")
            .expect("plan start");
        assert_eq!(start.len(), 1);
        assert!(start[0].blocking);

        let compact = engine
            .plan_session_before_compact("sess-1", "token_threshold", 5000)
            .expect("plan compact");
        assert_eq!(compact.len(), 1);
        assert!(!compact[0].blocking);

        let mutate = engine
            .plan_context_mutate("sess-1", 3)
            .expect("plan mutate");
        assert_eq!(mutate.len(), 1);
        assert!(!mutate[0].blocking);
    }
}
