# Zene decoupling rollout

Phased refactor toward composable agent crates (see `docs/agent-components.md`).

## Phase 1 — Permission + tool output (done)

- **`zene-permission`**: `ToolPermission` trait, `PermissionGate`, modes, rules, policy helpers.
- **`zene-tool-runtime`**: pure `plan_tool_output_bound` + `ToolOutputStore` / `FsToolOutputStore` spill.
- `zene-core` composes spill at runtime; engine logic stays in tool-runtime.

## Phase 2 — Hooks IO外移 (done)

- **`zene-hooks`**: `HookEngine` plans `HookRunRequest`; `HookExecutor` trait + `BashHookExecutor` for subprocess IO.
- `HookRunner` orchestrates plan + execute; core re-exports for CLI/ACP.

## Phase 3 — Workspace / Skills (done)

- **`zene-workspace`**: `WorkspaceProvider` trait + `FsWorkspaceProvider`; pure `build_system_prompt`.
- Agent instructions, directory listing, git branch, skills discovery IO in FS adapter.

## Phase 4 — Context IO 收尾 (done)

- **`CompactionSegmentWrite`** + `ContextEvent::CompactionSegment`; runtime persists via `FsCompactionSegmentStore`.
- **`MemoryStore`** trait + `FsMemoryStore`; memory flush/load IO moved out of `memory.rs` logic.

## Phase 5 — Turn loop → `zene-turn` (done)

- **`zene-turn`**: `TurnId` / `SteerBuffer` / turn guards; `TurnRuntime` trait + `run_turn_loop`.
- `Agent` implements `TurnRuntime`; core keeps step/tools/LLM wiring, loop orchestration in `zene-turn`.

## Phase 6 — Context runtime boundary (done)

- **`ContextEventHandler`** trait: gateway publish, memory flush LLM+store, compaction segment IO.
- **`ContextEvent::MemoryFlush`** + extended **`PublishPrefix`** (session_id + messages); engine no longer calls HTTP/LLM/store directly.
- **`ContextSession::persist_checkpoint`**: checkpoint IO via session trait (not handler).
- **`ContextDeps`**: removed `workdir`; memory reminder via handler + `MemoryStore`.
- **`AgentContextHandler`** in `zene-core` composes FS stores + gateway + memory flush.

## Wave 9–12 — Event, runtime, recovery boundaries (current closeout)

The implementation slices for Wave 9–12 are complete for the current design scope:

- **Conversation SoT**: `SessionEvent` covers message, system prefix, compaction, tool call/result, permission, mode/model change, and branch/fork/rewind facts. `SessionView` projects the active event path; `messages` remains a materialized compatibility cache, with explicit migration and fallback reasons for legacy data.
- **Context projection**: `observe` / `commit` / `project` use the event-backed view by default. `ProjectionExplain` and RuntimeEvent / ACP projection updates expose active path, fallback, injected content, tool truncation/handles, retained turns, and delivery provenance.
- **Model/runtime boundaries**: `zene-model-executor` owns model request assembly and retry seams; `zene-runtime` owns the transport-neutral command/state/response contract and `RuntimeControl`. The Agent-specific actor remains in core behind the private `agent_runtime` module and the `RuntimeHandle` re-export.
- **Recovery / Cloud**: safe model-boundary resume is gated by durable recovery state; `zene-cloud-runtime-client` owns the worker's ACP session, command, event normalization, reconnect, and replay boundary. Attempt/generation fencing and durable event outbox protections are in place.

## Remaining external boundaries

- Move the Agent-specific driver wiring and event sink into a separately composable runtime crate when its dependencies are stable; keep `zene-core` as the default composition root and compatibility facade meanwhile.
- Continue removing only legacy session formats that can be migrated without losing compaction/rewind facts; incomplete legacy records must retain explicit inspection/fallback behavior.
- Finish deployment-level Cloud Run, ACP session, and runtime lifecycle policy, including the documented multi-worker durable-outbox/storage boundary.
