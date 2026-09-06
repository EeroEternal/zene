# Zene Engine Notes

Core agent loop lives in `crates/core`. This document tracks engine-level behaviors (turn flow, context, permissions) beyond the milestone checklist in [ROADMAP.md](./ROADMAP.md). For the Session-vs-Context architecture model, see [session-as-source-of-truth.md](./session-as-source-of-truth.md). For Context Engine (projection, prefix cache, epoch/delta, **index vs Select**), see [context-engine.md](./context-engine.md). For system architecture and crate boundaries, see [architecture.md](./architecture.md). For historical runtime waves, see [agent-runtime-optimization.md](./archive/agent-runtime-optimization.md). For Pi agent-harness comparisons, see [pi-agent-harness.md](./research/pi-agent-harness.md).

## Turn flow & steer

- One **active turn** per `Agent` (`TurnState` in `turn.rs`).
- `Agent::prompt()` starts a turn; concurrent `prompt()` calls fail with an error that suggests `steer()`.
- **`Agent::steer(text)`** queues follow-up user guidance in `SteerBuffer` (kimi `steerBuffer` analogue). Messages are injected as `Message::user` **after the current step completes** (post-tool or post-assistant), not as a new turn.
- Steer is typically used from Cloud / ACP / async callers (the interactive local REPL was removed).
- Event: `AgentEvent::SteerInput { text }` for UI/replay hooks.

## Token estimation

Implemented in `tokens.rs` as `TokenEstimator`. Call sites use `Agent::token_estimator()` → `TokenEstimator::for_provider(provider, model, chars_per_token)`.

**OpenAI path (Option A)**: when `provider` is OpenAI/openai-compatible **and** the model name maps in `tiktoken-rs` (`gpt-4o` → `o200k_base`, `gpt-4` / `gpt-3.5-turbo` → `cl100k_base`, etc.), text is counted with real BPE (`encode_ordinary`). Message framing follows the OpenAI cookbook (~3 tokens/message + 3 reply priming on the request). Unknown openai-compatible model names (e.g. DeepSeek) fall back to the heuristic below.

**Heuristic path (Option B)**: default **script-aware** mode for Anthropic and unmapped models — Latin/code runs use configurable `chars_per_token` (default 4); CJK ≈1 token/char. Uniform legacy mode remains available via `EstimateMode::Uniform`.

**Per-message estimate** (`estimate_message_tokens` / `TokenEstimator::estimate_message_tokens`):

| Component | Heuristic | Tiktoken (OpenAI) |
|-----------|-----------|-------------------|
| Role framing | system +8, user/assistant +4, tool +8 | +3 per message |
| Compaction summary kind | +4 on top of framing | +4 on top of framing |
| Text content | script-aware / uniform chars | BPE `encode_ordinary` length |
| Tool calls (assistant) | +12 framing + id/name/args | +1 framing + BPE id/name/args |
| JSON tool arguments | length + structural punctuation | BPE of full JSON string |
| Tool result metadata | `tool_call_id`, `name`, error (+2) | same fields via BPE |
| Request priming | — | +3 once (`estimate_request_tokens`) |

**Request estimate**: `estimate_context(messages, tools, estimator)` = sum of message tokens + serialized tool-definition JSON (+4 framing) [+ reply priming in tiktoken mode]. Used before compaction triggers and inside `tail_start_index` / compaction planning.

Warn log when estimate ≥ 90% of `compaction.context_window_tokens`.

## Compaction (v2)

Three phases in `compaction.rs`, cheapest first. Session history records every pass in `CompactionEntry` with `reason`, `tokens_before`, `tokens_after`.

### Phase 1 — Truncate (in place)

Before any message removal or LLM call, replace oversized bodies in the **compactable prefix** (between system and recent tail):

- Tool results longer than **800 chars** → `[truncated N chars]`
- Assistant text (not compaction summaries) longer than **1200 chars** → same placeholder

If estimated tokens drop below threshold → stop (`reason: truncate_only`).

### Phase 2 — Slice keep

Simulate dropping the prefix while keeping:

- Leading system message
- All existing `CompactionSummary` messages in the prefix
- Recent tail (`keep_recent_ratio` token budget, floored by `min_keep_messages`, default 20)

If the sliced estimate is below threshold → apply slice and stop (`reason: slice_keep`). No LLM cost.

### Phase 3 — LLM summarize

Only when still over threshold after truncate + slice. Summarize the prefix via a side LLM call, replace with a single compaction summary message + tail (`reason: llm_summarize` / `token_threshold` / `context_overflow`).

Config (`compaction` in `config.toml`):

- `context_window_tokens` (default 128_000)
- `trigger_ratio` (default 0.85)
- `keep_recent_ratio` (default 0.25)
- `min_keep_messages` (default 20)

### Overflow recovery

On provider context-overflow errors in `run_llm_step`:

1. **First retry** — steps-first truncate of the **current turn** tool tail only (`apply_steps_truncate_pass`). Older history is left byte-stable for prefix cache.
2. **Second retry** — full compaction pipeline (phases 1→3) with input ladder (`epoch++`, a legal cache break)

Avoids paying for LLM summarize when truncation alone fixes the overflow.

### Usage-driven water level

`ContextWaterLevel` (`context_water.rs`) tracks the last provider `prompt_tokens` and the heuristic estimate. Auto-compact triggers on `max(usage, estimate)` vs `context_window * trigger_ratio` (default 85%). After tool results, a preflight pass also compacts when the estimate exceeds the hard window. Failed summarize sets sticky suppression until a successful `/compact`. Session persists `context_window_usage` / `context_tokens_used`; Cloud Console shows usage/context.

### Full-replace assemble + input ladder

LLM summarize rebuilds history as:

`system + last_user_query + recent_after_query + compaction_summary`. Volatile `<system-reminder>` (todos, plan, background, memory) is **not** persisted into history; `project()` appends it at the request tail. See [context-engine.md](./context-engine.md).

Tail selection snaps so assistant `tool_calls` are never split from their tool results. Summarizer input steps `verbatim → fitted → lossy` (`input_ladder.rs`): fitted shrinks bodies and drops oldest whole turns; lossy flattens tool results. Summaries shorter than **500** chars are rejected (retries, then hard fail — aligned with grok-build). Manual `/compact [hint]` forces summarize and writes compaction checkpoints.

### MCP output bounding

MCP tool results over `ZENE_MAX_MCP_OUTPUT_BYTES` / `MAX_MCP_OUTPUT_BYTES` (default 20_000) are truncated inline and spilled to `.zene/tool-output/` so large payloads do not force premature auto-compact.

### Deferred tool schemas

The tool registry separates registered tools from the active model-facing
schema. Built-in tools remain active by default; MCP tools are registered
deferred, so a large MCP catalog does not appear in every `tools[]` payload.
Call `Agent::activate_tools` / `deactivate_tools` (or ACP
`session/activate_tools` / `session/deactivate_tools`) to change the active
set for subsequent turns. The runtime returns the names it changed and ignores
unknown names.

Activation is additive at the request-shaping layer: the next request includes
the newly active definitions, while existing conversation messages and the
stable system prefix remain unchanged. Providers that support additive tool
updates can preserve their prefix cache; activation still changes the tool
portion of the request and may require provider-side schema reprocessing.

### Prefire two-pass + segments

When usage reaches auto-compact threshold minus `ZENE_PREFIRE_LEAD_PERCENT` (default 10pp), a background pass1 summarizes ~95% of history into NOTE₁. At compact time, pass2 merges NOTE₁ with the recent tail (prefire hit). Without a valid cache, large prefixes still use synchronous two-pass. Compacted prefixes are also written under `~/.zene/sessions/<id>/compaction_segments/` for recovery.

### Memory flush + injection

Near compact time, zene may run a no-tools flush turn that extracts durable lessons into `{workdir}/.zene/memory/daily/YYYY-MM-DD.md` (disable with `ZENE_MEMORY=0`). Session start injects recent memory into the system prompt once inside `<memory-context>` (kept stable for KV-friendly prefixes). Post-compact reminders also re-inject memory alongside todos/background tasks. Optional curated notes can live in `{workdir}/.zene/memory/MEMORY.md`.

### Intra-lite tool bounding

Non-MCP tool results over `ZENE_MAX_TOOL_OUTPUT_BYTES` (default 30_000) are truncated into session history and spilled to `.zene/tool-output/` (MCP uses its own 20KB path).

### Intra Steps-first

When auto-compact is about to fire, zene first runs a Steps-first pass (`compaction.intra_steps_first`, default true): tool results after the last user message are truncated to ~200 chars. If that alone brings usage under the threshold, full summarize is skipped (grok Intra `StepsOnly` / HistoryThenSteps lite).

## Sandbox profiles (Keel)

Production entry points build `LocalSandbox::with_options` from config / CLI:

| Profile | Filesystem | Child network | Notes |
|---------|------------|---------------|-------|
| `off` | path_policy only | unrestricted | No Keel space |
| `workspace` (default) | broad read; write workspace + temps | unrestricted | Everyday coding |
| `read-only` | broad read; write keel home + temps | deny-all | Explore default when unset |
| `strict` | workspace + system paths only | deny-all | Untrusted repos |
| custom | from `~/.zene/sandbox.toml` | per profile | Keel `SandboxConfig` loader |

Overrides: CLI `--sandbox` > `ZENE_SANDBOX` > `[sandbox] profile` in config. `allow_hosts` (config / `ZENE_SANDBOX_ALLOW_HOSTS`) turns network into an allowlist and is enforced for Bash children (Keel egress proxy) plus host tools (`FetchUrl`, `WebSearch`, HTTP MCP) via `LocalSandbox::authorize_egress`. Keel ≥0.0.12 baseline credential denies apply on macOS/Windows; on Linux Zene strips those FS denies before creating the space (Keel 0.0.15 outer-`bwrap` + Landlock `pre_exec` breaks userns) and relies on host `path_policy` / `check_read_allowed` for credential gating, with Landlock still isolating child writes. File I/O prefers Keel `SpaceFs` when a space is active. `[sandbox] auto_allow_bash = true` skips Bash permission prompts while enforcement is on.

## Permission modes (grok-aligned)

| Mode | Behavior |
|------|----------|
| `default` / `manual` | Ask for Write/Edit/Bash/MCP |
| `accept_edits` | Auto-approve Write/Edit; ask Bash/MCP |
| `dont_ask` | Deny gated tools without prompting |
| `bypass` / `yolo` | Auto-approve gated tools |

Config `[permission_rules]` supports `allow` / `deny` / `ask` patterns (`Bash`, `mcp__*`). Hard deny Write/Edit under `node_modules` / `.git` still applies in all modes. Explicit `ask` rules force a prompt even under bypass.

## Session recovery

Before/after compaction, checkpoints are saved under `~/.zene/sessions/<id>/compaction_checkpoints/`. Slash commands: `/rewind [id]`, `/fork`, `/session-info`.

## Background tasks

`Bash` and `Task` accept `run_in_background=true`. They return a `task_id` immediately; poll or cancel with `TaskOutput` (`action=list|get|kill`). Background Bash uses a longer timeout (30m). Store lives on the main `Agent` for the session.

## Git worktree

`zene --worktree` creates (or reuses) `.zene/worktrees/<session-slug>` via `git worktree add -B zene/<slug>` and runs the agent sandbox there.

## MCP transports

`~/.zene/mcp.json` / `.zene/mcp.json` servers may use:

- **stdio**: `{ "command", "args", "env" }`
- **HTTP**: `{ "url", "headers" }` (Streamable HTTP / JSON-RPC POST; SSE `data:` frames accepted)

`zene mcp doctor` probes configured servers.

## ACP stdio

`zene acp` (--yolo optional) speaks an Agent Client Protocol subset over stdin/stdout NDJSON JSON-RPC:

- Requests: `initialize`, `session/new`, `session/load`, `session/resume`, `session/list`, `session/close`, `session/set_mode`, `session/activate_tools`, `session/deactivate_tools`, `session/prompt`
- Notifications in: `session/cancel` (honored during an active prompt)
- Notifications out: `session/update` (`agent_message_chunk`, `agent_thought_chunk`, `user_message_chunk`, `tool_call`, `tool_call_update`, `plan`, `current_mode_update`, `available_commands_update`, `usage_update`)
- Requests out: `session/request_permission` (client replies with `optionId`; `toolCallId` matches the live tool call)
- When the client advertises FS capabilities, text Read/Write/Edit go through `fs/read_text_file` / `fs/write_text_file`
- When the client advertises `terminal`, Bash goes through `terminal/create|wait_for_exit|output|kill|release`
- `session/new` / `load` / `resume` return `modes` (`default` / `plan`); `load` replays history, `resume` does not
- Concurrent prompts on one session are queued FIFO; cancel aborts the active turn
- Prompt content accepts `text`, embedded `resource`, and `resource_link` blocks

Stdout is reserved for protocol frames; logs go to stderr.

## Cloud ACP bridge

Cloud workers speak ACP to a `zene acp` child via `cloud/crates/acp-bridge`. There is no local browser gateway; the product UI is Cloud Console (`cloud/apps/web`).

## LLM layer

- `OpenAiCompatibleProvider` (`crates/llm`) intentionally depends on [`unigateway-sdk`](https://crates.io/crates/unigateway-sdk) from **crates.io** (not a sibling path repo). It drives proxy chat via `UniGatewayEngine` (pools, retry, streaming).
- Anthropic uses a separate native Messages API client in the same crate.
- Retry classification (`LlmErrorClass`): context overflow (no transport retry), rate-limit (capped), transient/empty-response (retry), fatal auth/4xx.

## Coordination

- Tool scheduler / parallel tool execution: owned by separate workstream — avoid large edits to `run_tools` beyond permission messaging.

## Agent profile

Configure main-agent tool subsets via `agent_profile` in `~/.zene/config.toml` or `ZENE_AGENT_PROFILE`:

| Profile | Built-in tools |
|---------|----------------|
| `full` (default) | All built-in tools |
| `explore` | Read/Grep/Glob/RepoMap + Skill + AskUser/Todo/FetchUrl/WebSearch + plan mode |
| `coder` | Read/Write/Edit/Bash/Grep/Glob/RepoMap + Skill + Task + collaboration + plan mode |

MCP tools are always merged regardless of profile.

Code index / Repo Map is **Select**, not ContextEngine: hits must land as tool results in the Body (or a session-frozen prefix). Do not inject a resizing documents block. See [context-engine.md](./context-engine.md) §5. Implemented as `zene-index` (`{workdir}/.zene/index/v1.json`) plus the `RepoMap` tool.
