# Architecture & Crates Specification

Zene (*Zen Engine*) is an open-source, headless coding-agent harness and backend. The `zene` binary (`apps/cli`) implements the Agent Client Protocol (`zene acp`) for external clients, workers, and editors.

This document defines the system architectural model, crate component boundaries, and dependency principles.

## 1. Architecture Overview

```text
[ Clients / Editors / Cloud Workers ]
                │
                ▼ (ACP JSON-RPC / CLI stdio)
   ┌──────────────────────────┐
   │  apps/cli (`zene acp`)   │
   └────────────┬─────────────┘
                │
                ▼
   ┌──────────────────────────────────────────────────────────┐
   │  Runtime & Control Layer                                 │
   │  - crates/agent-runtime (State machine, Actor, Recovery) │
   │  - crates/runtime (RuntimeControl trait, Router, State)  │
   └────────────┬─────────────────────────────────────────────┘
                │
                ▼
   ┌──────────────────────────────────────────────────────────┐
   │  Core Orchestration Layer                                │
   │  - crates/core (Agent composition root, plan mode)       │
   │  - crates/turn (Turn/step loop, event ordering)          │
   └──────┬──────────────────────┬──────────────────────┬─────┘
          │                      │                      │
          ▼                      ▼                      ▼
┌──────────────────┐   ┌──────────────────┐   ┌──────────────────┐
│ Context Engine   │   │ Tools & Sandbox  │   │ Session & State  │
│ - context        │   │ - tools          │   │ - session        │
│ - model-executor │   │ - sandbox (Keel) │   │ - workspace      │
│ - index          │   │ - permission     │   │ - config         │
│ - llm            │   │ - tool-runtime   │   │                  │
└──────────────────┘   │ - hooks          │   │                  │
                       │ - mcp            │   │                  │
                       └──────────────────┘   └──────────────────┘
```

## 2. Workspace Crates & Component Boundaries

### Applications (`apps/`)

- **`apps/cli` (`zene`)**:
  - Primary executable. Exposes `zene acp` (Agent Client Protocol stdio server) and utility inspection commands (`doctor`, `config`, `sessions`).
  - Dispatches ACP requests (`session/new`, `session/prompt`, `session/set_mode`, `session/set_config_option`, `session/clear_queue`, `session/cancel`).
- **`apps/inference-gateway`**:
  - Optional smart inference gateway service for upstream LLM load balancing, prefix cache alignment, and retry routing.

### Runtime & Execution Layer (`crates/`)

- **`crates/runtime`**:
  - Transport-neutral control contract (`RuntimeControl`). Defines command and state channels (`Prompt`, `Steer`, `SetMode`, `Approval`, `Shutdown`).
- **`crates/agent-runtime`**:
  - Long-lived Actor runtime managing turn execution, asynchronous approval queues, and durable recovery checkpoints.
- **`crates/core`**:
  - Composition root for the agent. Coordinates context assembly, tool invocation batches, plan mode guards, and subagent delegation.
- **`crates/turn`**:
  - Deterministic turn loop state machine (`TurnId`, `StepResult`, `SteerBuffer`, ordered event sequences).
- **`crates/model-executor`**:
  - Intermediate model request/response boundary bridging turns with provider calls; handles overflow retry ladders.
- **`crates/llm`**:
  - Provider adapters (OpenAI-compatible, Anthropic). Implements streaming delta parsing, retry backoff with 429/413 classification, and reasoning effort propagation.

### Context & Knowledge Layer (`crates/`)

- **`crates/context`**:
  - Semantic context engine: token estimation, context water level, automatic multi-stage compaction (truncate → slice → summarize), memory extraction, and KV cache prefix layout.
- **`crates/index`**:
  - Codebase symbol graph and indexing engine powering the `RepoMap` tool.
- **`crates/workspace`**:
  - Discovers and loads project workspace context: `AGENTS.md`, active git branch, directory structure, and `.agents/skills/`.

### Tooling, Sandbox & Security (`crates/`)

- **`crates/sandbox`**:
  - Local process isolation based on Keel. Enforces workspace confinement, symlink escape checks, sensitive path deny rules (`.env*`, `.git/`, SSH/cloud credentials), and network egress rules.
- **`crates/tools`**:
  - Built-in agent tools: `Read`, `Write`, `Edit`, `Bash`, `Task`, `TaskOutput`, `WebSearch`, `TodoWrite`.
- **`crates/tool-runtime`**:
  - Large tool output spilling, paging, and output bound enforcement.
- **`crates/permission`**:
  - Fine-grained permission gates (allow / deny / ask), pattern-based matching, and interactive approval broker interfaces.
- **`crates/hooks`**:
  - Lifecycle hook configuration, in-process extension traits (`ExtensionHook`), and execution (`PreToolUse`, `PostToolUse`, `BeforeAgentStart`, `SessionBeforeCompact`, `ContextMutate`). Supports tool gating (`block` / `terminate`).
- **`crates/mcp`**:
  - Model Context Protocol (MCP) client manager over stdio and HTTP transports.

### Configuration & Persistence (`crates/`)

- **`crates/config`**:
  - TOML configuration loader (`~/.zene/config.toml` + project `.zene/config.toml` + environment variable overrides).
- **`crates/session`**:
  - Append-only event-sourced conversation transcript store, checkpoint snapshots, rewind operations, and Cellz storage engine.

## 3. Core Design Principles

1. **Session is the Source of Truth, Context is a Projection**:
   - `crates/session` records immutable facts (what happened).
   - `crates/context` calculates the ephemeral prompt view (what the model sees now).
2. **Zero UI in Core Framework**:
   - `zene` does not ship embedded web frontends or admin panels. Web management and multi-tenant UI belong to `zene-cloud`.
3. **Strict Sandbox Containment**:
   - All file and shell interactions must be validated against canonical paths before execution. Symlinks pointing outside the workspace or into sensitive credential files are denied.
4. **Zen Engine: Composable & Standalone Bricks**:
   - Designed like a headless harness (similar to Flue). Modules like `zene-sandbox`, `zene-context`, `zene-tools`, and `zene-session` must strive for standalone usability so developers can embed or adopt them independently without pulling in the entire framework.

## 4. Host-Facing Extension & Lifecycle Integration

Zene exposes a typed, deterministic host extension and lifecycle surface designed for Cloud orchestrators, ACP editors, and embedded custom runtimes:

### 4.1 Lifecycle Events

| Event | Trigger Point | Payload / Context | Capabilities |
| --- | --- | --- | --- |
| `before_agent_start` | Turn start before first model step | `session_id`, `prompt` | Can block turn initialization with reason |
| `pre_tool_use` | Immediately prior to tool execution | `tool`, `arguments` | Can allow, block (error back to LLM), or terminate (end turn) |
| `post_tool_use` | Immediately after tool execution | `tool`, `arguments` | Post-execution auditing / side-effect sync |
| `session_before_compact` | Prior to context compaction | `session_id`, `turn_count`, `token_count` | Pre-compaction notifications & reminders |
| `context_mutate` | During tail context decoration assembly | `step`, `plan_mode_active` | Mutates trailing ephemeral sections while keeping prefix cache epoch stable |

### 4.2 Integration Modes

1. **In-Process Rust Trait (`ExtensionHook`)**:
   - Injected into `AgentBuilder` via `builder.extension_hook(Arc::new(MyHook))` or `agent.add_extension_hook(...)`.
   - Async, zero serialization overhead, returns typed `HookOutcome::Allow` or `HookOutcome::Block(HookBlock)`.
2. **Process Hooks (`[[hooks]]` in `.zene/config.toml`)**:
   - Shell command executables receiving JSON payload on `stdin`.
   - Exit code `0`: Allow.
   - Exit code `1` (or stdout JSON `{"block": true}`): Block tool with error output returned to LLM.
   - Exit code `2` (or stdout JSON `{"block": true, "terminate": true}`): Block and terminate tool batch immediately.
3. **ACP Lifecycle Streaming**:
   - Remote hosts receive `sessionUpdate: "lifecycle_event"` notifications in real time via JSON-RPC.

### 4.3 Zero UI Guardrail

Extensions remain strictly headless (Rust traits, stdin/stdout subprocesses, or ACP JSON-RPC notifications). Embedding JavaScript/TypeScript webview runtimes or HTML/CSS control consoles in core is explicitly disallowed.
