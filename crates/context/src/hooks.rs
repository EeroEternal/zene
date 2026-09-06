//! Optional runtime hooks (todos, background tasks, observability).

/// Runtime-provided context for compaction reminders, tail decorations, and lifecycle events.
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

    /// Hook called immediately before session compaction begins (token threshold, overflow, or manual).
    fn on_session_before_compact(&self, _reason: &str, _current_tokens: u32) {}

    /// Hook called during step preparation allowing safe dynamic context mutation (e.g. host guidance,
    /// dynamic steering, or telemetry) without mutating or invalidating the cacheable frozen prefix epoch.
    fn mutate_tail_decorations(&self, _epoch: u64) -> Vec<String> {
        Vec::new()
    }
}

/// No-op hooks for tests and minimal integrations.
pub struct NoContextHooks;

impl ContextHooks for NoContextHooks {}
