# Changelog

## Unreleased

### Added
- Configurable Keel sandbox profiles: `--sandbox` / `ZENE_SANDBOX` / `[sandbox]` in config, plus `~/.zene/sandbox.toml` custom profiles (`off` | `workspace` | `read-only` | `strict` | custom).
- Host-side egress gating for `FetchUrl`, `WebSearch`, and HTTP MCP via Keel `check_egress`; `allow_hosts` allowlist support.
- Default credential path denies (read + Keel policy) for `~/.ssh`, `~/.aws`, `**/.env*`, `**/*.pem`, etc.; Read/Write prefer Keel `SpaceFs` when enforced.
- `[sandbox] auto_allow_bash` to skip Bash prompts while a sandbox profile is active.

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

