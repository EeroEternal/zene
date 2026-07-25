# AGENTS.md

## Repository layout

- **Scripts**：仓库级可执行脚本放在根目录 [`scripts/`](scripts/)（如 `scripts/install.sh`、`scripts/install-release.sh`）。不要把安装 / 运维脚本堆在仓库根目录。Cloud 子项目脚本仍放在 [`cloud/scripts/`](cloud/scripts/)。

## Design

UI 视觉与布局以根目录 [`DESIGN.md`](DESIGN.md) 为准；细则见 [`docs/Designs.md`](docs/Designs.md)。对照稿在 [`docs/design/`](docs/design/)（Cursor 风格：Inter + `#0090FF`）。改 `cloud/apps/web/` 时必须遵守。

**布局 IA 保持现有 Console 结构**（侧栏 272px、New Agent 居中 composer、Run = 对话左 + CodePanel 右）；视觉 token 使用 cursor 设计系统，不要擅自改成三栏 IDE 骨架，除非产品明确要求。

### Icons

Cloud Console 图标统一使用 [Lucide](https://lucide.dev/icons)：

- **React / Next**（`cloud/apps/web/`）：安装 `lucide-react`，在 `lib/icons.tsx` 集中 re-export（保持 `Icon*` 命名），禁止在各组件内手写 SVG。
- **小尺寸**（≤16px）：`strokeWidth={2}` + `absoluteStrokeWidth`，保证选中勾、分支节点等在列表里清晰可辨。
- **Lucide 无对应图标时**：优先选语义最接近的 Lucide 图标；仍无法满足再手写 SVG（24×24 viewBox、2px stroke、`strokeLinecap="round"`），并同样保证小尺寸可读。
- **品牌图标**（GitHub / GitLab 等）：从官方资源获取，不得手写近似图形。源文件放在 `cloud/apps/web/public/icons/`，并在 `lib/icons.tsx` 内联同源 path；GitHub 用 [Brand Toolkit / Mark](https://brand.github.com/foundations/logo)，GitLab 用 [gitlab-artwork](https://gitlab.com/gitlab-com/gitlab-artwork) 的 logomark（菜单内用 `currentColor` 单色，不改形）。

## Cursor Cloud specific instructions

Zene is a coding-agent product (workspace version in `Cargo.toml`). Primary surfaces:

- `zene` (`apps/cli`): REPL (`--repl` / bare `zene`), headless `-p`, and `zene acp` stdio Agent Client Protocol
- Cloud Console UI: `cloud/apps/web/` (served by `zene-cloud-api`). Production deploy: GCP VM behind Cloudflare (`zene.run`); see `cloud/deploy/`

At runtime the agent makes outbound HTTPS calls to an external LLM provider (OpenAI-compatible or Anthropic). Cloud workers use `zene acp` via `cloud/crates/acp-bridge`.

Toolchain caveat (non-obvious): a dependency (`unigateway-sdk`) requires Rust `edition2024`, so the toolchain must be Rust >= 1.85. The base image historically defaulted to an older `rustc` (1.83); the update script pins `rustup default stable`. If you ever hit `feature edition2024 is required`, run `rustup default stable`.

Standard commands (see `README.md`, `.github/workflows/ci.yml`, `scripts/install.sh`):

- Build: `cargo build --workspace --locked`
- Test (CI gate): `cargo test --workspace --locked`
- Lint: `cargo clippy --workspace --locked` (note: `cargo fmt --all --check` currently reports pre-existing formatting diffs and is NOT enforced in CI)
- Run: `cargo run -p zene-cli` or the built `./target/debug/zene`
- Install: `./scripts/install.sh` (= `cargo install --path apps/cli --locked`)

Running the agent requires an LLM API key (`ZENE_API_KEY`/`OPENAI_API_KEY`, or `ANTHROPIC_API_KEY` with `ZENE_PROVIDER=anthropic`). Config lives at `~/.zene/config.toml` (auto-created on first run) and env vars override it (`ZENE_MODEL`, `ZENE_BASE_URL`, etc.). Sessions persist under `~/.zene/sessions/`.

Testing the agent loop without a paid key: point it at a local OpenAI-compatible mock via `ZENE_BASE_URL` and run headless, e.g. `zene --yolo -p "<prompt>" --output-format json`. The mock only needs to serve `POST /chat/completions` returning `choices[0].message` (optionally with `tool_calls`); `--output-format json` forces non-streaming so a plain JSON completion is sufficient. `--yolo` auto-approves Write/Edit/Bash tools so headless tool calls run unattended.
