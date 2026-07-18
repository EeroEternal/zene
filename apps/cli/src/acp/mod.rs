//! Minimal ACP (Agent Client Protocol) stdio agent.
//!
//! Speaks JSON-RPC 2.0 NDJSON over stdin/stdout:
//! `initialize`, `session/new`, `session/load`, `session/prompt`, `session/cancel`,
//! plus `session/update` notifications and `session/request_permission` requests.

mod protocol;
mod server;

pub use server::run_acp;
