# AGENTS.md

Zene is a coding-agent product: Cloud Console lives in `cloud/apps/web/`; the `zene` binary (`apps/cli`) speaks `zene acp` for Cloud workers and editors.

Primary toolchain is Cargo (Rust workspace). `cloud/apps/web/` uses npm. Rust >= 1.85 (`unigateway-sdk` needs `edition2024`); if you hit `feature edition2024 is required`, run `rustup default stable`.

## Commands

- Build: `cargo build --workspace --locked`
- Test: `cargo test --workspace --locked`
- Lint: `cargo clippy --workspace --locked` (`cargo fmt --all --check` is not in CI)
- Local Cloud: `cd cloud && ./scripts/dev.sh`
- Install ACP binary: `./scripts/install.sh`

## Read when relevant

- [Repository layout](docs/agents/layout.md) — where scripts belong
- [Console UI](docs/agents/console-ui.md) — when changing `cloud/apps/web/`
- [Console capabilities](docs/agents/console-capabilities.md) — `import { … } from "@/cap/<id>"`; `./cloud/scripts/use-capability.sh`
- [Console feature slice](docs/agents/console-feature.md) — add a UI+API capability (`cloud/scripts/new-feature.sh`)
- [Cloud runtime](docs/agents/cloud.md) — ACP, workers, BYOK, local HMR, deploy
