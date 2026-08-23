# AGENTS.md

Zene is a coding-agent product: Cloud Console lives in `cloud/apps/web/`; the `zene` binary (`apps/cli`) speaks `zene acp` for Cloud workers and editors.

Primary toolchain is Cargo (Rust workspace). `cloud/apps/web/` uses npm. Rust >= 1.85 (`unigateway-sdk` needs `edition2024`); if you hit `feature edition2024 is required`, run `rustup default stable`.

## Commands

- Build: `cargo build --workspace --locked`
- Test: `cargo test --workspace --locked`
- Lint: `cargo clippy --workspace --locked` (`cargo fmt --all --check` is not in CI)
- Local Cloud: `cd cloud && ./scripts/dev.sh`
- Install ACP binary: `./scripts/install.sh`

## Console generation (required)

When adding or rewriting Cloud Console UI or API, **reuse named capabilities**. Do not copy `NewAgent.tsx` / `RunView.tsx` or invent a parallel picker/fetch/composer.

1. Import from `@/cap/<id>` (mix ids in the same file):
   `import { Composer, useComposerText } from "@/cap/composer";`
   `import { ProjectPicker } from "@/cap/project-picker";`
2. List ids / print import lines: `./cloud/scripts/use-capability.sh` or `./cloud/scripts/use-capability.sh llm composer project-picker`.
3. Catalog: `cloud/apps/web/lib/capabilities.ts`. Barrels: `cloud/apps/web/lib/cap/<id>.ts`.
4. If the ability does not exist: `./cloud/scripts/new-feature.sh <kebab-name>` (API + client + hook + `@/cap/<id>`), then import that id. Follow [feature slice](docs/agents/console-feature.md).
5. Pages must not call `api("/api/v1/...")` or `fetch` with path strings. No native `<select>`. No `window.alert` / `confirm` / `prompt`.

Ids: `llm`, `repositories`, `github`, `session`, `runs`, `composer`, `project-picker`, `branch-picker`, `model-picker`, `attach-menu`, `picker`, `menu`, `dialogs`, `http`.

## Workflow

- Code and docs changes go through GitHub PRs (branch → PR → review → merge); reference the issue (`Refs: ParaTensor/zene#N`, `Closes #N`). Direct pushes to `main` are limited to release chores (version bump / lockfile / changelog) and trivial fixes.
- Version bumps follow `scripts/publish-crates.sh` order (`zene-config → zene-llm → zene-model-executor → zene-session → zene-context`); run `--verify` before publishing.

## Read when relevant

- [Repository layout](docs/agents/layout.md) — where scripts belong
- [Console UI](docs/agents/console-ui.md) — when changing `cloud/apps/web/`
- [Console capabilities](docs/agents/console-capabilities.md) — `@/cap/<id>`
- [Console feature slice](docs/agents/console-feature.md) — add a UI+API capability (`cloud/scripts/new-feature.sh`)
- [Cloud runtime](docs/agents/cloud.md) — ACP, workers, BYOK, local HMR, deploy
