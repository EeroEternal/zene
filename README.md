# Zene

Zene is an open-source coding-agent framework and CLI toolchain. The `zene` binary speaks Agent Client Protocol (`zene acp`) for external clients, workers, and editors.

For the multi-tenant hosted web platform and console, see [zene-cloud](https://github.com/EeroEternal/zene-cloud).

## Install `zene` (ACP binary)

Needed for Cloud workers / editors. From the repo:

```bash
./scripts/install.sh
```

Or manually:

```bash
cargo install --path apps/cli --locked
```

Pre-built binaries are published on [GitHub Releases](https://github.com/ParaTensor/zene/releases) when a version tag (`v*`) is pushed. Each release includes `zene` for Linux and macOS (x86_64 + Apple Silicon). Download install (no compile):

```bash
curl -fsSL https://raw.githubusercontent.com/ParaTensor/zene/main/scripts/install-release.sh | bash
```

Or build without installing:

```bash
cargo build -p zene-cli
# ./target/debug/zene acp
```

## Configure (ACP / agent runtime)

On first run, Zene creates `~/.zene/config.toml`. Cloud Console users normally set LLM keys in **Settings** (BYOK). For local ACP / env overrides:

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

Custom Keel profiles can also live in `~/.zene/sandbox.toml` / `.zene/sandbox.toml` (same shape as Keel `[profiles.*]`). `ZENE_SANDBOX` overrides config. Explore agent profile defaults to `read-only` when sandbox profile is unset.

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

## `zene` commands

The interactive local REPL and headless `-p` were removed. Product UI is Cloud Console.

```bash
zene acp               # Agent Client Protocol over stdio (Cloud workers / editors)
zene acp --yolo        # auto-approve Write / Edit / Bash
zene sessions          # list saved sessions for current workdir
zene config            # show config paths
zene export --session <id> --output out.zip
zene mcp doctor        # probe configured MCP servers
```
 
For detailed system layering and crate breakdown, see [docs/architecture.md](docs/architecture.md).

## Architecture

Zene is organized into a modular workspace of crates and binaries:

```
zene/
├── apps/
│   ├── cli/               # zene binary: ACP server & CLI subcommands
│   └── inference-gateway/ # Local reverse proxy for session prefix caching
├── crates/                # 17 domain-isolated crates (context, tools, sandbox, session, turn...)
└── docs/                  # System architecture, engine specs, and research
```

See [docs/architecture.md](docs/architecture.md) for crate boundaries and [docs/context-engine.md](docs/context-engine.md) for semantic context management.

## License

MIT
