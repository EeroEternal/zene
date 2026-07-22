# Changelog

## Unreleased

### Changed
- Bump Keel (`eero-keel-core`) from `0.0.11` to `0.0.15` (baseline credential denies, audit hash chain, Windows Job/AppContainer). On Linux hosts where Keel-style bubblewrap deny binds cannot run, Zene soft-falls back to the process-guard backend while keeping host `path_policy`.

## v0.1.7 (2026-07-20)

Headless **Web Agent** becomes the default UI: local `zene-gateway` serves the browser UI over HTTP (long-polling + optional SSE), with `zene` / `zene web` as the launch entry. Releases and `www/install.sh` now ship both `zene` and `zene-gateway` binaries.

### Added
- Headless Web direction: `docs/WEB_AGENT_GATEWAY.md` design for HTTP Gateway + Web Agent UI (long-polling first; SSE optional; WebSocket not required).
- New `zene-gateway` binary (`apps/gateway`): thin local HTTP bridge over `zene acp` with token/Origin checks, `POST /api/v1/agents/{id}/messages`, cursor-based `GET /api/v1/agents/{id}/events` long polling, bootstrap/health, embedded Web Agent UI, and mock-ACP integration tests.
- Gateway phase B: optional SSE (`GET /events/stream`) with Web long-poll fallback, controller lease APIs, `apps/web-agent` UI (sessions/tool cards/usage/SSE), `--yolo`/`--sandbox-off`/`--acp-env`, and real `zene acp` + mock LLM smoke test.
- Gateway phase C: local ACP `terminal/*` host with Web terminal panel, Plan/Todo/background-task panels, mode switch + session close UI, and terminal roundtrip tests.
- Gateway phase D: on-disk event journal + agent meta, `restart`/`attach` recovery, poll backpressure and payload limits, `zene web` launcher, and `docs/GATEWAY_OPS.md`.
- Gateway phase E: AskUser over standard `session/request_permission`, Web `session/resume`, default `zene` launches Web Agent, remove ratatui TUI (`docs/TUI_MIGRATION.md`).
- Prefire two-pass compaction with `compaction_segments` persistence (NOTE₁ cache + sync merge).
- Memory flush / injection into context, with content-fingerprint dedup across turns.
- Intra **steps-first** pass: truncate current-turn tool results before full summarize when that alone frees enough budget.
- Intra-lite tool output bounds for non-MCP tools; MCP oversized results truncate-to-disk.
- OpenAI path **`tiktoken-rs`**: known models (`gpt-4o` → o200k, `gpt-4` / `gpt-3.5-turbo` → cl100k, etc.) use real BPE; unknown openai-compatible names and Anthropic keep the script-aware heuristic.
- Script-aware token heuristic (Latin vs CJK) as the non-tiktoken default.
- `/context` (alias `/tokens`) context report; preflight compact when estimate exceeds the hard window.
- Configurable Keel sandbox profiles: `--sandbox` / `ZENE_SANDBOX` / `[sandbox]` in config, plus `~/.zene/sandbox.toml` custom profiles (`off` | `workspace` | `read-only` | `strict` | custom).
- Host-side egress gating for `FetchUrl`, `WebSearch`, and HTTP MCP via Keel `check_egress`; `allow_hosts` allowlist support.
- Default credential path denies (read + Keel policy) for `~/.ssh`, `~/.aws`, `**/.env*`, `**/*.pem`, etc.; Read/Write prefer Keel `SpaceFs` when enforced.
- `[sandbox] auto_allow_bash` to skip Bash prompts while a sandbox profile is active.
- Docs: Cloudflare Pages pause guide; `deploy-web.sh` gated behind `ZENE_PAGES_DEPLOY=1`.

### Changed
- Default interactive entry is Web Agent (`zene` / `zene web`); `zene --tui` errors with a migration hint; debug line UI remains as `zene --repl`.
- ACP: bridge `tool_call` / `tool_call_update` / `plan` / `usage_update` / `current_mode_update` / `available_commands_update` / `agent_thought_chunk`; replay history on `session/load`; implement `session/list`, `session/close`, `session/set_mode`, `session/resume`; optional client FS + terminal bridges; FIFO prompt queue with in-turn cancel; correlate permission `toolCallId`; accept embedded prompt context; tighten JSON-RPC error codes.
- Stronger compaction ladder (reject thin summaries, tool-pair snap, sticky suppress after failed summarize).
- Compaction / water-level behavior tuned closer to grok-build Inter/Intra lite semantics.
- Landing page (`www/`) refreshed for current Zene features.
- GitHub Releases and `www/install.sh` publish/install both `zene` and `zene-gateway`; gateway serves UI with `Cache-Control: no-store`.

### Fixed
- Flaky `PreToolUse` hook test: ignore stdin BrokenPipe when the hook exits before reading payload.

## v0.1.6 (2026-07-18)

### Added
- Usage-driven context water level, full-replace compaction, and input ladder (`verbatim → fitted → lossy`).
- Permission modes: `default` / `accept_edits` / `dont_ask` / `bypass`, plus allow/deny/ask rules.
- Session recovery: compaction checkpoints, `/rewind`, `/fork`, `/session-info`, `/compact`.
- Background `Bash`/`Task` with `TaskOutput`; `zene --worktree` session git worktrees.
- MCP HTTP transport alongside stdio; `zene mcp doctor`.
- Headless `zene -p` with `--output-format json`.
- Minimal ACP stdio agent: `zene acp` (`initialize`, `session/*`, permission bridge).

### Changed
- LLM retry classification for overflow / rate-limit / transient errors.
- Grok-alignment roadmap items P1–P6 marked complete in `docs/ROADMAP.md` / `docs/ENGINE.md`.

## v0.1.5 (2026-05-31)

### Changed
- Default CLI startup to TUI; `--repl` for line REPL.
- TUI turn UX, model/provider configuration, and permission prompting improvements.

## v0.1.4 (2026-05-30)

### Added
- **WebSearch** and **FetchUrl** tools for DuckDuckGo search and page fetch.
- **Todo** tool with session-persisted todo lists (`TodoWrite` / store).
- **AskUser** collaboration tool; parallel tool execution via `tool_scheduler`.
- **Agent profiles** in config (`agent_profile`) for model/tool presets.
- Compaction **v2**: improved context trimming and token accounting.
- `docs/ENGINE.md` architecture notes.

### Fixed
- **unigateway-sdk** pinned to crates.io `2.1.1` (CI/release builds no longer need a local path).

### Changed
- Session records and ROADMAP/README updates for Batch 7 capabilities.
