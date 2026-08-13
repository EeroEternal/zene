//! Zene-specific [`ContextHooks`] (todos + background tasks for compaction reminders).

use zene_context::ContextHooks;
use zene_session::{SessionRecord, TodoStatus};
use zene_tools::{BackgroundTask, BackgroundTaskKind, BackgroundTaskStatus};

use crate::plan_mode::plan_mode_tail_section;

pub struct ZeneContextHooks {
    sections: Vec<String>,
}

impl ZeneContextHooks {
    pub fn new(
        session: &SessionRecord,
        background_tasks: &[BackgroundTask],
        plan_active: bool,
    ) -> Self {
        let mut sections = Vec::new();

        if let Some(plan) = plan_mode_tail_section(plan_active) {
            sections.push(plan.to_string());
        }

        let actionable: Vec<_> = session
            .todos
            .iter()
            .filter(|item| !matches!(item.status, TodoStatus::Completed))
            .collect();
        if !actionable.is_empty() {
            let mut lines = vec!["Active todos:".to_string()];
            for item in actionable {
                let status = match item.status {
                    TodoStatus::Pending => "pending",
                    TodoStatus::InProgress => "in_progress",
                    TodoStatus::Completed => "completed",
                };
                lines.push(format!("- [{status}] {}", item.content));
            }
            sections.push(lines.join("\n"));
        }

        let running: Vec<_> = background_tasks
            .iter()
            .filter(|t| t.status == BackgroundTaskStatus::Running)
            .collect();
        if !running.is_empty() {
            let mut lines = vec!["Background tasks still running:".to_string()];
            for task in running {
                let kind = match task.kind {
                    BackgroundTaskKind::Bash => "bash",
                    BackgroundTaskKind::Subagent => "task",
                };
                lines.push(format!("- {} ({kind}): {}", task.id, task.label));
            }
            sections.push(lines.join("\n"));
        }

        Self { sections }
    }
}

impl ContextHooks for ZeneContextHooks {
    fn compaction_reminder_sections(&self) -> Vec<String> {
        self.sections.clone()
    }

    fn step_tail_decorations(&self) -> Vec<String> {
        self.sections.clone()
    }
}
