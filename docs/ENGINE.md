# Zene Engine Notes

Core agent loop lives in `crates/core`. This document tracks engine-level behaviors (turn flow, context, permissions) beyond the milestone checklist in [ROADMAP.md](./ROADMAP.md).

## Turn flow & steer

- One **active turn** per `Agent` (`TurnState` in `turn.rs`).
- `Agent::prompt()` starts a turn; concurrent `prompt()` calls fail with an error that suggests `steer()`.
- **`Agent::steer(text)`** queues follow-up user guidance in `SteerBuffer` (kimi `steerBuffer` analogue). Messages are injected as `Message::user` **after the current step completes** (post-tool or post-assistant), not as a new turn.
- CLI REPL: `/steer <message>` when a turn is active (typically from TUI/async callers; blocking REPL waits on `prompt()`).
- Event: `AgentEvent::SteerInput { text }` for UI/replay hooks.

## Token estimation (v2 heuristic)

Implemented in `tokens.rs` as `TokenEstimator` — no external tokenizer dependency (Option B). Uses configurable `chars_per_token` (default 4) from global config or per-model `model_chars_per_token`.

**Per-message estimate** (`estimate_message_tokens` / `TokenEstimator::estimate_message_tokens`):

| Component | Heuristic |
|-----------|-----------|
| Role framing | system +8, user +4, assistant +4, tool +8 tokens |
| Compaction summary kind | +4 tokens on top of assistant framing |
| Text content | `ceil(char_count / chars_per_token)` |
| Tool calls (assistant) | +12 framing per call + id/name/arguments length |
| JSON tool arguments | string length **plus** structural punctuation (`{}[]:,"`) counted separately |
| Tool result metadata | `tool_call_id`, `name`, error flag (+2) |

**Request estimate**: `estimate_context(messages, tools, estimator)` = sum of message tokens + serialized tool-definition JSON (+4 framing). Used consistently before compaction triggers and inside `tail_start_index` / compaction planning.

Warn log when estimate ≥ 90% of `compaction.context_window_tokens`.

Future: Option A (`tiktoken-rs` for OpenAI provider) can replace the char heuristic without changing call sites.

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

1. **First retry** — phase-1 truncate pass only (`apply_overflow_truncate_pass`)
2. **Second retry** — full compaction pipeline (phases 1→3) with input ladder

Avoids paying for LLM summarize when truncation alone fixes the overflow.

### Usage-driven water level

`ContextWaterLevel` (`context_water.rs`) tracks the last provider `prompt_tokens` and the heuristic estimate. Auto-compact triggers on `max(usage, estimate)` vs `context_window * trigger_ratio` (default 85%). Session persists `context_window_usage` / `context_tokens_used`; TUI status shows `ctx N%`.

### Full-replace assemble + input ladder

LLM summarize rebuilds history as:

`system + last_user_query + recent_after_query + compaction_summary + optional <system-reminder> (todos)`

Summarizer input steps `verbatim → fitted → lossy` on overflow or degenerate (too-short) summaries (`input_ladder.rs`). Manual `/compact [hint]` forces summarize and writes compaction checkpoints.

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

`zene acp` (--yolo optional) speaks a minimal Agent Client Protocol subset over stdin/stdout NDJSON JSON-RPC:

- Requests: `initialize`, `session/new`, `session/load`, `session/prompt`
- Notifications in: `session/cancel`
- Notifications out: `session/update` (`agent_message_chunk`)
- Requests out: `session/request_permission` (client replies with `optionId`)

Stdout is reserved for protocol frames; logs go to stderr.

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
| `explore` | Read/Grep/Glob + Skill + AskUser/Todo/FetchUrl/WebSearch + plan mode |
| `coder` | Read/Write/Edit/Bash/Grep/Glob + Skill + Task + collaboration + plan mode |

MCP tools are always merged regardless of profile.
