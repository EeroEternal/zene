//! ACP (Agent Client Protocol) stdio agent.
//!
//! Speaks JSON-RPC 2.0 NDJSON over stdin/stdout:
//! `initialize`, `session/new`, `session/load`, `session/resume`, `session/list`,
//! `session/close`, `session/set_mode`, `session/prompt`, `session/cancel`,
//! plus outbound `session/update` notifications
//! (`agent_message_chunk`, `agent_thought_chunk`, `user_message_chunk`, `tool_call`,
//! `tool_call_update`, `plan`, `current_mode_update`, `available_commands_update`,
//! `usage_update`) and client requests `session/request_permission`,
//! `fs/read_text_file`, `fs/write_text_file`, `terminal/*` (when advertised).

mod fs_bridge;
mod protocol;
mod server;
mod terminal_bridge;
mod transport;
mod updates;

pub use server::run_acp;
