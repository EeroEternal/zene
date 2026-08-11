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

## Phase 6 — Context runtime boundary (current)

- **`ContextEventHandler`** trait: gateway publish, memory flush LLM+store, compaction segment IO.
- **`ContextEvent::MemoryFlush`** + extended **`PublishPrefix`** (session_id + messages); engine no longer calls HTTP/LLM/store directly.
- **`ContextSession::persist_checkpoint`**: checkpoint IO via session trait (not handler).
- **`ContextDeps`**: removed `workdir`; memory reminder via handler + `MemoryStore`.
- **`AgentContextHandler`** in `zene-core` composes FS stores + gateway + memory flush.
