//! No-op memory hooks when `memory` feature is disabled.

use std::path::Path;

use anyhow::Result;
use zene_llm::Message;

pub const MEMORY_CONTEXT_OPEN: &str = "<memory-context>";
pub const MEMORY_CONTEXT_CLOSE: &str = "</memory-context>";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlushResult {
    NothingToStore,
    Accepted,
    Rejected,
}

pub fn memory_enabled() -> bool {
    false
}

pub fn memory_root(_workdir: &Path) -> std::path::PathBuf {
    std::path::PathBuf::new()
}

pub fn ensure_memory_in_system(_messages: &mut [Message], _store: &dyn crate::memory_store::MemoryStore) {}

pub fn conversation_has_memory_context(_messages: &[Message]) -> bool {
    false
}

pub fn memory_reminder(_workdir: &Path) -> Option<String> {
    None
}

pub fn memory_reminder_from_store(_store: &dyn crate::memory_store::MemoryStore) -> Option<String> {
    None
}

pub fn format_flush_input(_messages: &[Message]) -> String {
    String::new()
}

pub fn should_flush(_usage_percent: u8, _threshold: u8, _already_flushed: bool) -> bool {
    false
}

pub async fn run_memory_flush(
    _client: &zene_llm::ChatClient,
    _model: &str,
    _conversation: &str,
    _store: &dyn crate::memory_store::MemoryStore,
) -> Result<FlushResult> {
    Ok(FlushResult::NothingToStore)
}

pub fn append_daily_log(_workdir: &Path, _content: &str) -> Result<std::path::PathBuf> {
    Ok(std::path::PathBuf::new())
}

pub fn daily_log_path(_workdir: &Path) -> std::path::PathBuf {
    std::path::PathBuf::new()
}

pub fn format_memory_context_block(_body: &str) -> String {
    String::new()
}

pub fn load_recent_memory(_workdir: &Path) -> Option<String> {
    None
}

pub fn load_recent_memory_from_store(_store: &dyn crate::memory_store::MemoryStore) -> Option<String> {
    None
}

pub fn is_duplicate_flush(_workdir: &Path, _content: &str) -> bool {
    false
}

pub fn process_flush_response(_content: &str) -> Result<Option<String>, FlushResult> {
    Err(FlushResult::NothingToStore)
}
