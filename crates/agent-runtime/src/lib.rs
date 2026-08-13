//! Agent-specific runtime actor.
//!
//! Protocol types (`RuntimeCommand`, `ApprovalWaiters`, …) live in
//! [`zene_runtime`]. This crate owns the actor that drives a
//! [`zene_core::Agent`].

mod actor;
mod approval;

pub use actor::RuntimeHandle;
pub use approval::{prompt_choice, RuntimeOwnedBroker};

pub use zene_runtime::{
    ApprovalDecision, ExecutionState, RuntimeCommand, RuntimeControl, RuntimeLifecycle,
    RuntimeRecoveryInfo, RuntimeResponse,
};
