//! Compatibility facade for the Agent-specific runtime actor.
//!
//! The implementation lives in the private `agent_runtime` module. This
//! module remains the stable internal path used by `zene_core::RuntimeHandle`.

pub use crate::agent_runtime::RuntimeHandle;
