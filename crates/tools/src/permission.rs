use std::io;
use std::sync::Arc;

use parking_lot::Mutex;

/// Shared permission gate used by main agent and subagents.
pub trait ToolPermission: Send + Sync {
    fn approve_tool_call(&mut self, tool_name: &str, arguments: &str) -> io::Result<bool>;
    fn denied_message(tool_name: &str) -> String
    where
        Self: Sized;
}

pub type SharedToolPermission = Arc<Mutex<dyn ToolPermission>>;
