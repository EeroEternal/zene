use std::collections::HashSet;
use std::io::{self, Write};

use serde_json::Value;
use zene_tools::ToolPermission;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PermissionMode {
    #[default]
    Manual,
    Yolo,
}

impl PermissionMode {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "yolo" => Self::Yolo,
            _ => Self::Manual,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptChoice {
    AllowOnce,
    AllowSession,
    Deny,
}

pub type PermissionPrompter = dyn Fn(&str, &str) -> io::Result<PromptChoice> + Send + Sync;

/// Gate for Write / Edit / Bash before execution.
pub struct PermissionGate {
    mode: PermissionMode,
    approved_session: HashSet<String>,
    prompter: Box<PermissionPrompter>,
}

impl PermissionGate {
    pub fn new(mode: PermissionMode) -> Self {
        Self {
            mode,
            approved_session: HashSet::new(),
            prompter: Box::new(default_prompter),
        }
    }

    pub fn with_prompter(mode: PermissionMode, prompter: Box<PermissionPrompter>) -> Self {
        Self {
            mode,
            approved_session: HashSet::new(),
            prompter,
        }
    }

    pub fn mode(&self) -> PermissionMode {
        self.mode
    }

    pub fn requires_confirmation(tool_name: &str) -> bool {
        matches!(tool_name, "Write" | "Edit" | "Bash") || tool_name.starts_with("mcp__")
    }

    /// Returns `Ok(true)` if the tool may run, `Ok(false)` if denied.
    pub fn check(&mut self, tool_name: &str, arguments: &str) -> io::Result<bool> {
        if self.mode == PermissionMode::Yolo || !Self::requires_confirmation(tool_name) {
            return Ok(true);
        }

        if let Some(key) = session_approval_key(tool_name, arguments) {
            if self.approved_session.contains(&key) {
                return Ok(true);
            }
        }

        let preview = truncate(arguments, 120);
        match (self.prompter)(tool_name, &preview)? {
            PromptChoice::AllowOnce => Ok(true),
            PromptChoice::AllowSession => {
                if let Some(key) = session_approval_key(tool_name, arguments) {
                    self.approved_session.insert(key);
                }
                Ok(true)
            }
            PromptChoice::Deny => Ok(false),
        }
    }

    pub fn denied_message(tool_name: &str) -> String {
        format!("Tool `{tool_name}` was denied by the user.")
    }
}

pub fn approve_tool_call(
    gate: &mut PermissionGate,
    tool_name: &str,
    arguments: &str,
) -> io::Result<bool> {
    gate.check(tool_name, arguments)
}

impl ToolPermission for PermissionGate {
    fn approve_tool_call(&mut self, tool_name: &str, arguments: &str) -> io::Result<bool> {
        self.check(tool_name, arguments)
    }

    fn denied_message(tool_name: &str) -> String {
        PermissionGate::denied_message(tool_name)
    }
}

fn default_prompter(tool_name: &str, args_preview: &str) -> io::Result<PromptChoice> {
    eprint!(
        "\nAllow {tool_name}({args_preview})? [y]es / [n]o / [a]pprove for session: "
    );
    let _ = io::stderr().flush();
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    match line.trim().to_lowercase().as_str() {
        "y" | "yes" => Ok(PromptChoice::AllowOnce),
        "a" | "approve" | "always" => Ok(PromptChoice::AllowSession),
        _ => Ok(PromptChoice::Deny),
    }
}

/// Session "approve once" key: `Write:relative/path` or `Bash:command`.
pub fn session_approval_key(tool_name: &str, arguments: &str) -> Option<String> {
    let value: Value = serde_json::from_str(arguments).ok()?;
    match tool_name {
        "Write" | "Edit" => value
            .get("path")
            .and_then(|p| p.as_str())
            .map(|path| format!("{tool_name}:{path}")),
        "Bash" => value
            .get("command")
            .and_then(|c| c.as_str())
            .map(|cmd| format!("Bash:{cmd}")),
        name if name.starts_with("mcp__") => Some(format!("{name}:{arguments}")),
        _ => None,
    }
}

fn truncate(input: &str, max: usize) -> String {
    if input.chars().count() <= max {
        input.to_string()
    } else {
        format!("{}...", input.chars().take(max).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;

    #[test]
    fn yolo_skips_prompt() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = Arc::clone(&calls);
        let gate = PermissionGate::with_prompter(PermissionMode::Yolo, {
            Box::new(move |_tool, _args| {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                Ok(PromptChoice::Deny)
            })
        });
        let mut gate = gate;
        assert!(gate
            .check("Write", r#"{"path":"a.txt","content":"x"}"#)
            .unwrap());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn manual_prompts_and_session_approve() {
        let mut gate = PermissionGate::with_prompter(PermissionMode::Manual, {
            Box::new(|_tool, _args| Ok(PromptChoice::AllowSession))
        });
        let args = r#"{"path":"src/foo.rs","content":"x"}"#;
        assert!(gate.check("Write", args).unwrap());
        assert!(gate.check("Write", args).unwrap());
    }

    #[test]
    fn manual_deny_returns_false() {
        let mut gate = PermissionGate::with_prompter(PermissionMode::Manual, {
            Box::new(|_tool, _args| Ok(PromptChoice::Deny))
        });
        assert!(!gate
            .check("Bash", r#"{"command":"rm -rf /"}"#)
            .unwrap());
    }

    #[test]
    fn mcp_tools_require_confirmation() {
        assert!(PermissionGate::requires_confirmation("mcp__git__status"));
    }

    #[test]
    fn session_key_format() {
        assert_eq!(
            session_approval_key("Write", r#"{"path":"a/b.rs"}"#).as_deref(),
            Some("Write:a/b.rs")
        );
        assert_eq!(
            session_approval_key("Bash", r#"{"command":"ls"}"#).as_deref(),
            Some("Bash:ls")
        );
    }
}
