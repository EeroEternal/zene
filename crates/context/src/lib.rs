mod assemble;
mod compaction;
mod config;
mod context_water;
mod engine;
mod event_handler;
mod events;
mod hooks;
mod input_ladder;
mod memory_store;
mod segment_store;
mod session;
mod tokens;
mod two_pass;

#[cfg(feature = "gateway")]
mod gateway;
#[cfg(not(feature = "gateway"))]
mod gateway_stub;

#[cfg(feature = "memory")]
mod memory;
#[cfg(not(feature = "memory"))]
mod memory_stub;
mod model;

#[cfg(feature = "prefire")]
mod prefire;
#[cfg(not(feature = "prefire"))]
mod prefire_stub;

pub use assemble::{
    assemble_outbound, delivery_mode_from_env, stable_system_boundary, AssembledOutbound,
    DeliveryMode,
};
pub use compaction::{
    apply_compaction_to_messages, apply_overflow_truncate_pass, apply_slice_keep,
    apply_steps_truncate_pass, assemble_full_replace_history, build_compaction_reminder,
    build_sliced_messages, compact_message_list_with_chat, compact_session, compact_session_forced,
    is_context_overflow_error, is_degenerate_summary, keep_recent_token_budget,
    last_user_query_index, plan_compaction, plan_compaction_segment, should_compact,
    subagent_compaction_config, tail_start_index, truncate_old_message_bodies,
    truncate_old_tool_results, CompactionPlan, CompactionResult, CompactionStats,
    MIN_SUMMARY_SEED_CHARS,
};
pub use config::{CompactionConfig, DEFAULT_CONTEXT_WINDOW_TOKENS};
pub use context_water::ContextWaterLevel;
pub use engine::{
    ContextDeps, ContextEngine, ContextObservation, ContextUsageUpdate, ForcedCompactResult,
    InjectedSource, OverflowHandleResult, PrefireClientFactory, PrepareStepResult,
    ProjectionExplain, StepContext, ToolOutputProvenance,
};
pub use event_handler::{
    write_compaction_segment_via, ContextEventHandler, EventOutcome, NoopContextEventHandler,
    RecordingContextEventHandler,
};
pub use events::ContextEvent;
#[cfg(feature = "gateway")]
pub use gateway::{
    close_session, external_session_id_from_env, gateway_configured, publish_prefix,
};
#[cfg(not(feature = "gateway"))]
pub use gateway_stub::{
    close_session, external_session_id_from_env, gateway_configured, publish_prefix,
};
pub use hooks::{ContextHooks, NoContextHooks};
pub use input_ladder::{fit_messages_to_budget, prepare_summary_input, InputLadderStage};
#[cfg(feature = "memory")]
pub use memory::{
    append_daily_log, conversation_has_memory_context, daily_log_path, ensure_memory_in_system,
    format_flush_input, format_memory_context_block, is_duplicate_flush, load_recent_memory,
    load_recent_memory_from_store, memory_enabled, memory_reminder, memory_reminder_from_store,
    memory_root, process_flush_response, run_memory_flush, should_flush, FlushResult,
    MEMORY_CONTEXT_CLOSE, MEMORY_CONTEXT_OPEN,
};
pub use memory_store::{FsMemoryStore, MemoryStore};
#[cfg(not(feature = "memory"))]
pub use memory_stub::{
    append_daily_log, conversation_has_memory_context, daily_log_path, ensure_memory_in_system,
    format_flush_input, format_memory_context_block, is_duplicate_flush, load_recent_memory,
    load_recent_memory_from_store, memory_enabled, memory_reminder, memory_reminder_from_store,
    memory_root, process_flush_response, run_memory_flush, should_flush, FlushResult,
    MEMORY_CONTEXT_CLOSE, MEMORY_CONTEXT_OPEN,
};
pub use model::ContextModel;
#[cfg(feature = "prefire")]
pub use prefire::{prefire_lead_percent, PrefireCache, PrefireState};
#[cfg(not(feature = "prefire"))]
pub use prefire_stub::{prefire_lead_percent, PrefireCache, PrefireState};
pub use segment_store::{CompactionSegmentStore, CompactionSegmentWrite, FsCompactionSegmentStore};
pub use session::ContextSession;
pub use tokens::{
    estimate_chars_as_tokens, estimate_context, estimate_message_tokens, estimate_messages_tokens,
    estimate_request_tokens, estimate_tools_tokens, EstimateMode, EstimateProvider,
    TiktokenEncoding, TokenEstimator,
};
pub use two_pass::{
    fingerprint_messages, note_for_pass2, pass2_user_prompt, split_messages_for_two_pass,
    TwoPassSplit, TWO_PASS_DEFAULT_SPLIT_FRACTION,
};
