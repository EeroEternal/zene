# AGENTS.md

## Repository layout

- **Scripts**：仓库级可执行脚本放在根目录 [`scripts/`](scripts/)（如 `scripts/install.sh`、`scripts/install-release.sh`）。不要把安装 / 运维脚本堆在仓库根目录。Cloud 子项目脚本仍放在 [`cloud/scripts/`](cloud/scripts/)。

## Design

UI 视觉与布局以根目录 [`DESIGN.md`](DESIGN.md) 为准；细则见 [`docs/Designs.md`](docs/Designs.md)。对照稿在 [`docs/design/`](docs/design/)（Cursor 风格：Inter + `#0090FF`）。改 `cloud/apps/web/` 时必须遵守。

**布局 IA 保持现有 Console 结构**（侧栏 272px、New Agent 居中 composer、Run = 对话左 + CodePanel 右）；视觉 token 使用 cursor 设计系统，不要擅自改成三栏 IDE 骨架，除非产品明确要求。

### Dialogs / confirmations

禁止使用浏览器原生弹窗（`window.alert` / `window.confirm` / `window.prompt`，以及等价的同步阻塞对话框）。确认删除、危险操作等一律用应用内自定义模态框（styled modal / `<dialog>`），视觉与交互需符合 Console 设计系统。

### Icons

Cloud Console 图标统一使用 [Lucide](https://lucide.dev/icons)：

- **React / Next**（`cloud/apps/web/`）：安装 `lucide-react`，在 `lib/icons.tsx` 集中 re-export（保持 `Icon*` 命名），禁止在各组件内手写 SVG。
- **小尺寸**（≤16px）：`strokeWidth={2}` + `absoluteStrokeWidth`，保证选中勾、分支节点等在列表里清晰可辨。
- **Lucide 无对应图标时**：优先选语义最接近的 Lucide 图标；仍无法满足再手写 SVG（24×24 viewBox、2px stroke、`strokeLinecap="round"`），并同样保证小尺寸可读。
- **品牌图标**（GitHub / GitLab 等）：从官方资源获取，不得手写近似图形。源文件放在 `cloud/apps/web/public/icons/`，并在 `lib/icons.tsx` 内联同源 path；GitHub 用 [Brand Toolkit / Mark](https://brand.github.com/foundations/logo)，GitLab 用 [gitlab-artwork](https://gitlab.com/gitlab-com/gitlab-artwork) 的 logomark（菜单内用 `currentColor` 单色，不改形）。

## Cursor Cloud specific instructions

Zene is a coding-agent product (workspace version in `Cargo.toml`). Primary surfaces:

- Cloud Console UI: `cloud/apps/web/` (served by `zene-cloud-api`). Local: `cd cloud && ./scripts/dev.sh`. Production deploy: GCP VM behind Cloudflare (`zene.run`); see `cloud/deploy/`
- `zene` (`apps/cli`): `zene acp` stdio Agent Client Protocol for Cloud workers / editors (interactive REPL and headless `-p` were removed)

At runtime the agent makes outbound HTTPS calls to an external LLM provider (OpenAI-compatible or Anthropic). Cloud workers use `zene acp` via `cloud/crates/acp-bridge`. Console users set LLM keys in Settings (BYOK); env / `~/.zene/config.toml` still apply to the ACP process when injected.

Toolchain caveat (non-obvious): a dependency (`unigateway-sdk`) requires Rust `edition2024`, so the toolchain must be Rust >= 1.85. The base image historically defaulted to an older `rustc` (1.83); the update script pins `rustup default stable`. If you ever hit `feature edition2024 is required`, run `rustup default stable`.

Standard commands (see `README.md`, `.github/workflows/ci.yml`, `scripts/install.sh`):

- Build: `cargo build --workspace --locked`
- Test (CI gate): `cargo test --workspace --locked`
- Lint: `cargo clippy --workspace --locked` (note: `cargo fmt --all --check` currently reports pre-existing formatting diffs and is NOT enforced in CI)
- Local Cloud: `cd cloud && ./scripts/dev.sh` (builds `zene` for ACP if needed). UI HMR: keep API on `:8788`, then `cd cloud/apps/web && npm run dev` → `:8787` (rewrites `/api/*` → API; optional `ZENE_CLOUD_SKIP_WEB_BUILD=1`)
- Install ACP binary: `./scripts/install.sh` (= `cargo install --path apps/cli --locked`)
