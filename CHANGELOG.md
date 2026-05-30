# Changelog

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

