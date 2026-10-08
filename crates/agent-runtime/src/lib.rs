//! Agent-specific runtime actor.
//!
//! Protocol types (`RuntimeCommand`, `ApprovalWaiters`, …) live in
//! [`runtime`]. This crate owns the actor that drives a
//! [`zene_core::Agent`].

mod actor;
mod approval;
pub mod runtime;

pub use actor::RuntimeHandle;
pub use approval::{prompt_choice, RuntimeOwnedBroker};

pub use runtime::{
    ApprovalDecision, ExecutionState, RuntimeCommand, RuntimeControl, RuntimeLifecycle,
    RuntimeRecoveryInfo, RuntimeResponse,
};
