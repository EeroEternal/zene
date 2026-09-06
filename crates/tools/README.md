# zene-tools

Production-ready, deterministic tool implementations for AI coding agents.

Part of the composable [Zene (Zen Engine)](https://github.com/ParaTensor/zene) agent stack. Usable standalone or embedded into any agent framework.

## Built-in Tools

- **File Manipulation**:
  - `Read`: Chunked reading with byte limits, line count limits, directory listing, and CRLF-to-LF view normalization.
  - `Write`: Atomic file write with directory auto-creation and line-ending preservation.
  - `Edit`: Precise search-and-replace with uniqueness checks (refuses ambiguous matches or no-op replacements).
- **Execution & Diagnostics**:
  - `Bash`: Sandboxed command runner with timeout, output bounds, and background task management.
  - `OutputSanitizer`: Filters massive `test ... ok` success logs, preserving failed assertions and panic traces to protect the LLM context window.
  - `Grep` / `Glob`: Ripgrep-backed pattern search and fast file discovery.
- **Workflow & Coordination**:
  - `Task` / `TaskOutput`: Asynchronous background jobs with cancellation and polling.
  - `WebSearch` / `FetchUrl`: Grounded search and web content extraction.
  - `AskUser`: Interactive permission and prompter questions.
  - `TodoWrite`: Structured task list and milestone management.

## Usage

```rust
use std::sync::Arc;
use zene_sandbox::{LocalSandbox, SandboxOptions};
use zene_tools::{default_builtin_tools, ToolRegistry};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let sandbox = Arc::new(LocalSandbox::new("./workspace", SandboxOptions::default())?);
    
    // Create registry with default coder tools
    let registry = Arc::new(ToolRegistry::new());
    for tool in default_builtin_tools(sandbox) {
        registry.register(tool);
    }

    // Inspect definitions for LLM function calling
    let definitions = registry.definitions();
    println!("Registered {} tools", definitions.len());
    Ok(())
}
```
