//! Lifecycle hooks: pure planning + runtime executors.

mod engine;
mod executor;
mod runner;

pub use engine::{
    build_hook_input, hook_failure_reason, HookEngine, HookEvent, HookPayload, HookRunRequest,
    HookSpec,
};
pub use executor::{BashHookExecutor, HookBlock, HookExecutor, HookOutcome};
pub use runner::{ExtensionHook, HookRunner};
