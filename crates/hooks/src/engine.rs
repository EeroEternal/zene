use anyhow::{Context, Result};
use serde::Serialize;

/// Hook lifecycle event (aligned with `hooks.json` / config.toml).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
}

impl HookEvent {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "PreToolUse" => Some(Self::PreToolUse),
            "PostToolUse" => Some(Self::PostToolUse),
            _ => None,
        }
    }
}

#[derive(Debug, Serialize)]
struct HookInput<'a> {
    tool: &'a str,
    args: &'a str,
}

/// Planned hook invocation (no IO).
#[derive(Debug, Clone)]
pub struct HookRunRequest {
    pub command: String,
    pub stdin_json: String,
    /// When true, non-zero exit becomes a block (PreToolUse).
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

    pub fn plan_pre_tool_use(&self, tool: &str, args: &str) -> Result<Vec<HookRunRequest>> {
        self.plan_for_event(HookEvent::PreToolUse, tool, args, true)
    }

    pub fn plan_post_tool_use(&self, tool: &str, args: &str) -> Result<Vec<HookRunRequest>> {
        self.plan_for_event(HookEvent::PostToolUse, tool, args, false)
    }

    fn plan_for_event(
        &self,
        event: HookEvent,
        tool: &str,
        args: &str,
        blocking: bool,
    ) -> Result<Vec<HookRunRequest>> {
        let stdin_json = build_hook_input(tool, args)?;
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
    let payload = HookInput { tool, args };
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
        assert_eq!(HookEvent::parse("PostToolUse"), Some(HookEvent::PostToolUse));
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
    }
}
