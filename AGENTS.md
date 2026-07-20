# AGENTS.md

## Cursor Cloud specific instructions

Zene is a local coding-agent product (workspace version in `Cargo.toml`). Primary binaries:
- `zene` (`apps/cli`): REPL/TUI, headless `-p`, and `zene acp` stdio Agent Client Protocol
- `zene-gateway` (`apps/gateway`): thin local HTTP gateway that spawns `zene acp` and serves a minimal Web Agent UI via long-polling (see `docs/WEB_AGENT_GATEWAY.md`)

The `web/`, `www/`, and root `package.json` files are still mostly static-site copy stubs, not the Agent UI. At runtime the agent makes outbound HTTPS calls to an external LLM provider (OpenAI-compatible or Anthropic); nothing is self-hosted except the optional local gateway.

Toolchain caveat (non-obvious): a dependency (`unigateway-sdk`) requires Rust `edition2024`, so the toolchain must be Rust >= 1.85. The base image historically defaulted to an older `rustc` (1.83); the update script pins `rustup default stable`. If you ever hit `feature edition2024 is required`, run `rustup default stable`.

Standard commands (see `README.md`, `.github/workflows/ci.yml`, `install.sh`):
- Build: `cargo build --workspace --locked`
- Test (CI gate): `cargo test --workspace --locked`
- Lint: `cargo clippy --workspace --locked` (note: `cargo fmt --all --check` currently reports pre-existing formatting diffs and is NOT enforced in CI)
- Run: `cargo run -p zene-cli` or the built `./target/debug/zene`
- Install: `./install.sh` (= `cargo install --path apps/cli --locked`)

Running the agent requires an LLM API key (`ZENE_API_KEY`/`OPENAI_API_KEY`, or `ANTHROPIC_API_KEY` with `ZENE_PROVIDER=anthropic`). Config lives at `~/.zene/config.toml` (auto-created on first run) and env vars override it (`ZENE_MODEL`, `ZENE_BASE_URL`, etc.). Sessions persist under `~/.zene/sessions/`.

Testing the agent loop without a paid key: point it at a local OpenAI-compatible mock via `ZENE_BASE_URL` and run headless, e.g. `zene --yolo -p "<prompt>" --output-format json`. The mock only needs to serve `POST /chat/completions` returning `choices[0].message` (optionally with `tool_calls`); `--output-format json` forces non-streaming so a plain JSON completion is sufficient. `--yolo` auto-approves Write/Edit/Bash tools so headless tool calls run unattended.
