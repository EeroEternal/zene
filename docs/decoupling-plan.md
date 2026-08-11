# Zene decoupling rollout

Phased refactor toward composable agent crates (see `docs/agent-components.md`).

## Phase 1 — Permission + tool output (done)

- **`zene-permission`**: `ToolPermission` trait, `PermissionGate`, modes, rules, policy helpers.
- **`zene-tool-runtime`**: pure `plan_tool_output_bound` + `ToolOutputStore` / `FsToolOutputStore` spill.
- `zene-core` composes spill at runtime; engine logic stays in tool-runtime.

## Phase 2 — Hooks IO外移 (current)

- **`zene-hooks`**: `HookEngine` plans `HookRunRequest`; `HookExecutor` trait + `BashHookExecutor` for subprocess IO.
- `HookRunner` orchestrates plan + execute; core re-exports for CLI/ACP.

## Phase 3 — Workspace / Skills (planned)

- `WorkspaceProvider` trait; default FS impl outside core turn loop.

## Phase 4 — Context IO 收尾 (planned)

- `compaction_segments` / `memory` → `ContextEvent` or `MemoryStore` trait.

## Phase 5 — Turn loop → `zene-turn` (planned)

- Extract `run_turn` / `run_step` from `Agent`; core keeps `AgentBuilder` only.
