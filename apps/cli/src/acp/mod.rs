//! ACP (Agent Client Protocol) stdio agent.
//!
//! Speaks JSON-RPC 2.0 NDJSON over stdin/stdout:
//! `initialize`, `session/new`, `session/load`, `session/list`, `session/prompt`,
//! `session/cancel`, plus outbound `session/update` notifications
//! (`agent_message_chunk`, `user_message_chunk`, `tool_call`, `tool_call_update`,
//! `plan`) and `session/request_permission` requests.

mod protocol;
mod server;
mod updates;

pub use server::run_acp;
