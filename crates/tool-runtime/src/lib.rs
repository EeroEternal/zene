//! Tool output bounding: pure planning + optional filesystem spill store.

mod output_bound;
mod spill_store;

pub use output_bound::{
    plan_tool_output_bound, tool_max_output_bytes, tool_output_handles_enabled,
    ToolBoundPlan, ToolOutputSpill, TOOL_MAX_OUTPUT_BYTES,
};
pub use spill_store::{apply_tool_bound_plan, FsToolOutputStore, ToolOutputStore};
