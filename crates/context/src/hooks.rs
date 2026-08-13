//! Optional runtime hooks (todos, background tasks, observability).

/// Runtime-provided context for compaction reminders and side effects.
pub trait ContextHooks: Send + Sync {
    /// Extra sections appended to post-compaction `<system-reminder>` (todos, background tasks, …).
    fn compaction_reminder_sections(&self) -> Vec<String> {
        Vec::new()
    }

    /// Live decorations projected at the **tail** of each LLM request.
    ///
    /// Must not be written into the frozen system prefix. Defaults to the same
    /// sections as compaction reminders so todos / background status stay visible
    /// without resizing the cacheable prefix.
    fn step_tail_decorations(&self) -> Vec<String> {
        self.compaction_reminder_sections()
    }
}

/// No-op hooks for tests and minimal integrations.
pub struct NoContextHooks;

impl ContextHooks for NoContextHooks {}
