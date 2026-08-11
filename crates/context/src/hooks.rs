//! Optional runtime hooks (todos, background tasks, observability).

/// Runtime-provided context for compaction reminders and side effects.
pub trait ContextHooks: Send + Sync {
    /// Extra sections appended to post-compaction `<system-reminder>` (todos, background tasks, …).
    fn compaction_reminder_sections(&self) -> Vec<String> {
        Vec::new()
    }
}

/// No-op hooks for tests and minimal integrations.
pub struct NoContextHooks;

impl ContextHooks for NoContextHooks {}
