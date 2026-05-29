use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::Deserialize;
use zene_config::sessions_dir;
use zene_tools::{PlanModeState, ToolResult};

pub const PLAN_MODE_REMINDER: &str = r#"<system_reminder>
You are in Plan mode. Only read-only tools (Read, Grep, Glob, Skill) and ExitPlanMode are available. Do not use Write, Edit, Bash, or Task until you call ExitPlanMode with your plan and the user approves it.
</system_reminder>"#;

pub type PlanApprovalPrompter =
    Arc<dyn Fn(&Path, &str) -> io::Result<bool> + Send + Sync>;

#[derive(Debug, Deserialize)]
pub struct EnterPlanModeArgs {
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExitPlanModeArgs {
    pub plan: String,
}

pub fn plan_mode_system_suffix(active: bool) -> Option<&'static str> {
    if active {
        Some(PLAN_MODE_REMINDER)
    } else {
        None
    }
}

pub fn build_effective_system_prompt(base: &str, plan_active: bool) -> String {
    match plan_mode_system_suffix(plan_active) {
        Some(reminder) => format!("{base}\n\n{reminder}"),
        None => base.to_string(),
    }
}

pub fn resolve_plan_path(workdir: &Path, session_id: &str) -> PathBuf {
    let project_plan = workdir.join(".zene/plan.md");
    if project_plan.parent().is_some() {
        if let Some(parent) = project_plan.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if parent_writable(&project_plan) {
            return project_plan;
        }
    }
    let session_plan = sessions_dir().join(session_id).join("plan.md");
    if let Some(parent) = session_plan.parent() {
        let _ = fs::create_dir_all(parent);
    }
    session_plan
}

fn parent_writable(path: &Path) -> bool {
    path.parent()
        .map(|p| p.exists() || fs::create_dir_all(p).is_ok())
        .unwrap_or(false)
}

pub fn handle_enter_plan_mode(
    state: &mut PlanModeState,
    arguments: &str,
) -> ToolResult {
    if state.is_active() {
        return ToolResult {
            content: "Already in plan mode.".to_string(),
            is_error: true,
        };
    }
    let args: EnterPlanModeArgs = match serde_json::from_str(arguments) {
        Ok(a) => a,
        Err(err) => {
            return ToolResult {
                content: format!("Invalid EnterPlanMode arguments: {err}"),
                is_error: true,
            };
        }
    };
    state.enter();
    let mut msg = "Entered plan mode. Use Read/Grep/Glob/Skill to explore, then ExitPlanMode with your plan.".to_string();
    if let Some(reason) = args.reason.filter(|r| !r.trim().is_empty()) {
        msg.push_str(&format!("\nReason: {reason}"));
    }
    ToolResult {
        content: msg,
        is_error: false,
    }
}

pub fn handle_exit_plan_mode(
    state: &mut PlanModeState,
    arguments: &str,
    workdir: &Path,
    session_id: &str,
    prompter: &PlanApprovalPrompter,
) -> Result<ToolResult> {
    if !state.is_active() {
        return Ok(ToolResult {
            content: "Not in plan mode.".to_string(),
            is_error: true,
        });
    }

    let args: ExitPlanModeArgs = serde_json::from_str(arguments)
        .context("parse ExitPlanMode args")?;
    if args.plan.trim().is_empty() {
        return Ok(ToolResult {
            content: "ExitPlanMode requires a non-empty `plan`.".to_string(),
            is_error: true,
        });
    }

    let plan_path = resolve_plan_path(workdir, session_id);
    fs::write(&plan_path, &args.plan)
        .with_context(|| format!("write plan to {}", plan_path.display()))?;

    eprintln!("\n--- Plan ({}) ---\n{}\n--- end plan ---\n", plan_path.display(), args.plan);

    let approved = prompter(&plan_path, &args.plan)?;
    if !approved {
        return Ok(ToolResult {
            content: format!(
                "Plan saved to {} but not approved. Still in plan mode.",
                plan_path.display()
            ),
            is_error: true,
        });
    }

    state.exit();
    Ok(ToolResult {
        content: format!(
            "Plan approved. Exited plan mode. Plan file: {}",
            plan_path.display()
        ),
        is_error: false,
    })
}

pub fn default_plan_approval_prompter(plan_path: &Path, _plan_body: &str) -> io::Result<bool> {
    eprint!(
        "Approve plan at {} and exit plan mode? [y]es / [n]o: ",
        plan_path.display()
    );
    let _ = io::stderr().flush();
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(matches!(
        line.trim().to_lowercase().as_str(),
        "y" | "yes"
    ))
}

pub fn tool_visible_in_definitions(name: &str, plan_active: bool) -> bool {
    if plan_active {
        matches!(name, "Read" | "Grep" | "Glob" | "Skill" | "ExitPlanMode")
    } else {
        name != "ExitPlanMode"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    #[test]
    fn reminder_only_when_active() {
        assert!(plan_mode_system_suffix(true).is_some());
        assert!(plan_mode_system_suffix(false).is_none());
    }

    #[test]
    fn enter_sets_active() {
        let mut state = PlanModeState::default();
        let result = handle_enter_plan_mode(&mut state, "{}");
        assert!(!result.is_error);
        assert!(state.is_active());
    }

    #[test]
    fn exit_requires_approval() {
        let dir = TempDir::new().unwrap();
        let mut state = PlanModeState::default();
        state.enter();
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_c = Arc::clone(&calls);
        let prompter: PlanApprovalPrompter = Arc::new(move |_path, _body| {
            calls_c.fetch_add(1, Ordering::SeqCst);
            Ok(false)
        });
        let result = handle_exit_plan_mode(
            &mut state,
            r##"{"plan":"# Plan\n\nDo thing."}"##,
            dir.path(),
            "sess-1",
            &prompter,
        )
        .unwrap();
        assert!(result.is_error);
        assert!(state.is_active());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn exit_approved_leaves_plan_mode() {
        let dir = TempDir::new().unwrap();
        let mut state = PlanModeState::default();
        state.enter();
        let prompter: PlanApprovalPrompter =
            Arc::new(|_path, _body| Ok(true));
        let result = handle_exit_plan_mode(
            &mut state,
            r##"{"plan":"# Plan\n\nDo thing."}"##,
            dir.path(),
            "sess-1",
            &prompter,
        )
        .unwrap();
        assert!(!result.is_error);
        assert!(!state.is_active());
    }

    #[test]
    fn definitions_filter_by_mode() {
        assert!(tool_visible_in_definitions("Write", false));
        assert!(!tool_visible_in_definitions("Write", true));
        assert!(tool_visible_in_definitions("Read", true));
        assert!(!tool_visible_in_definitions("ExitPlanMode", false));
        assert!(tool_visible_in_definitions("ExitPlanMode", true));
    }
}
