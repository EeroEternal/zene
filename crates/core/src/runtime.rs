//! Private compatibility module for the Agent-specific runtime actor.
//!
//! The actor implementation lives in `agent_runtime`. This module preserves
//! the existing `zene_core::RuntimeHandle` re-export while the generic
//! command/event contract lives in `zene-runtime`.

pub use crate::agent_runtime::RuntimeHandle;
