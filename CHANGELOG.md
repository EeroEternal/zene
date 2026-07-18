# Changelog

## v0.1.7 (2026-07-18)

This release finishes the **grok-build long-context / sampling alignment** pass after v0.1.6: cheaper compaction paths, better token accounting, memory hygiene, and OpenAI-accurate BPE counts.

### Added
- Prefire two-pass compaction with `compaction_segments` persistence (NOTE₁ cache + sync merge).
- Memory flush / injection into context, with content-fingerprint dedup across turns.
- Intra **steps-first** pass: truncate current-turn tool results before full summarize when that alone frees enough budget.
- Intra-lite tool output bounds for non-MCP tools; MCP oversized results truncate-to-disk.
- OpenAI path **`tiktoken-rs`**: known models (`gpt-4o` → o200k, `gpt-4` / `gpt-3.5-turbo` → cl100k, etc.) use real BPE; unknown openai-compatible names and Anthropic keep the script-aware heuristic.
- Script-aware token heuristic (Latin vs CJK) as the non-tiktoken default.
- `/context` (alias `/tokens`) context report; preflight compact when estimate exceeds the hard window.
- Docs: Cloudflare Pages pause guide; `deploy-web.sh` gated behind `ZENE_PAGES_DEPLOY=1`.

### Changed
- Stronger compaction ladder (reject thin summaries, tool-pair snap, sticky suppress after failed summarize).
- Compaction / water-level behavior tuned closer to grok-build Inter/Intra lite semantics.

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

