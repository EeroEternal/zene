# Zene

Zene is a local coding agent CLI written in Rust. It runs in your project directory, reads and edits files, executes shell commands, and keeps conversation sessions on disk.

## Install

From the repo:

```bash
./install.sh
```

Or manually:

```bash
cargo install --path apps/cli --locked
```

Pre-built binaries are published on [GitHub Releases](https://github.com/ParaTensor/zene/releases) when a version tag (`v*`) is pushed. Each release includes `zene` and `zene-gateway` for Linux and macOS (x86_64 + Apple Silicon). Download install (no compile):

```bash
curl -fsSL https://raw.githubusercontent.com/ParaTensor/zene/main/www/install.sh | bash
```

Or run directly without installing:

```bash
cargo run -p zene-cli
```

## Configure

On first run, Zene creates `~/.zene/config.toml`. Set your API key there or via environment variables:

```bash
# OpenAI-compatible (default provider)
export OPENAI_API_KEY=sk-...
export ZENE_API_KEY=sk-...
export ZENE_MODEL=gpt-4o
export ZENE_BASE_URL=https://api.openai.com/v1

# Anthropic
export ZENE_PROVIDER=anthropic
export ANTHROPIC_API_KEY=sk-ant-...
export ZENE_MODEL=claude-3-5-sonnet-20241022
export ZENE_ANTHROPIC_BASE_URL=https://api.anthropic.com  # optional
```

Optional flags in `~/.zene/config.toml`:

```toml
provider = "openai"  # or "anthropic"
include_workspace_context = true  # inject AGENTS.md, directory listing, git branch

[[hooks]]
event = "PreToolUse"
command = "./scripts/pre-tool.sh"

[web_search]
provider = "tavily"   # or "duckduckgo" (no API key, limited HTML parsing)
api_key = "tvly-..."  # or export ZENE_WEB_SEARCH_API_KEY

[sandbox]
profile = "workspace"          # off | workspace | read-only | strict | custom
# allow_hosts = ["api.github.com:443"]
# auto_allow_bash = false      # skip Bash prompts when sandbox is active
```

Custom Keel profiles can also live in `~/.zene/sandbox.toml` / `.zene/sandbox.toml` (same shape as Keel `[profiles.*]`). CLI `--sandbox` and `ZENE_SANDBOX` override config. Explore agent profile defaults to `read-only` when sandbox profile is unset.

Per-project overrides in `.zene/config.toml` (merged over global; project wins on key collision):

```toml
# your-repo/.zene/config.toml
model = "gpt-4o"
permission_mode = "manual"

[compaction]
trigger_ratio = 0.9
```

### Config files

| Path | Purpose |
|------|---------|
| `~/.zene/config.toml` | Model, provider, API keys, compaction, permission mode, sandbox, inline hooks |
| `.zene/config.toml` | Project-level config overrides (merged over global) |
| `~/.zene/sandbox.toml` | Custom Keel sandbox profiles (`[profiles.<name>]`) |
| `.zene/sandbox.toml` | Project-level custom sandbox profiles (additive names only) |
| `~/.zene/hooks.json` | Additional lifecycle hooks (`PreToolUse`, `PostToolUse`) |
| `~/.zene/mcp.json` | Global MCP server definitions (merged with project config) |
| `.zene/mcp.json` | Project-level MCP server overrides |
| `~/.zene/sessions/` | Saved conversation sessions and JSONL records |

Hooks receive JSON on stdin: `{"tool":"<name>","args":"<json string>"}`. A non-zero exit from a `PreToolUse` hook blocks the tool; the stderr message is returned to the model.

Example `~/.zene/hooks.json`:

```json
{
  "hooks": [
    { "event": "PostToolUse", "command": "./scripts/log-tool.sh" }
  ]
}
```

Skills live under `.agents/skills/*/SKILL.md`. Zene lists discovered skills in the system prompt; use the `Skill` tool to load a skill's instructions.

**Web search:** Configure `[web_search]` in `~/.zene/config.toml`. With `provider = "tavily"` and an API key ([Tavily](https://tavily.com/) — simple REST API), results include title, URL, and snippet. Without a key, `provider = "duckduckgo"` scrapes DuckDuckGo HTML (fragile, fewer results, may break if markup changes).

## Usage

```bash
cd your-project
zene                   # opens local Web Agent UI (zene-gateway)
```

CLI commands:

```bash
zene sessions          # list saved sessions for current workdir
zene config            # show config paths
zene -p "prompt"       # headless single prompt
zene --yolo            # auto-approve Write / Edit / Bash (also forwarded to web)
zene --sandbox strict  # Keel profile: off | workspace | read-only | strict | custom
zene acp               # Agent Client Protocol over stdio (for editors / gateway)
zene web --yolo --sandbox-off   # explicit Web Agent launch
zene --repl            # debug line REPL (not the default UI)
```

```bash
cargo run -p zene-cli -- --yolo --sandbox-off
# open the printed http://127.0.0.1:8787/#token=... URL
```

`zene-gateway` is a thin local HTTP bridge: the Web UI prefers SSE and falls back to long-polling; the gateway speaks ACP NDJSON to a `zene acp` child process. Journals persist under `~/.zene/gateway` for restart/attach recovery. UI sources live in `apps/web-agent/`. See [docs/WEB_AGENT_GATEWAY.md](docs/WEB_AGENT_GATEWAY.md), [docs/GATEWAY_OPS.md](docs/GATEWAY_OPS.md), and [docs/TUI_MIGRATION.md](docs/TUI_MIGRATION.md).

## Cloud Platform (experimental)

Multi-user Cloud Agent control plane lives in [`cloud/`](cloud/). Phase 0 provides local auth, repositories, runs, worker claim, mock agent events, and a Cursor-style Web UI:

```bash
cd cloud && ./scripts/dev.sh
# open http://127.0.0.1:8788/
```

## Architecture

```
zene/
├── apps/cli/          # web / headless / ACP / debug REPL entrypoint
├── apps/gateway/      # local HTTP gateway for Web Agent UI
└── crates/
    ├── core/          # agent turn loop, hooks
    ├── llm/           # OpenAI-compatible (unigateway-sdk from crates.io) + Anthropic clients
    ├── sandbox/       # local filesystem + shell
    ├── tools/         # Read/Write/Edit/Bash/Grep/Glob/Task
    ├── session/       # session persistence (~/.zene/sessions/)
    ├── mcp/           # MCP server integration
    └── config/        # config loading
```

## License

MIT
