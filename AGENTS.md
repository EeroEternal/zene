# AGENTS.md — Code Agent Collaboration Specification

Zene is a coding-agent product: Cloud Console lives in `cloud/apps/web/`; the `zene` binary (`apps/cli`) speaks `zene acp` for Cloud workers and editors.

Primary toolchain is Cargo (Rust workspace). `cloud/apps/web/` uses npm. Rust >= 1.85 (`unigateway-sdk` needs `edition2024`); if you hit `feature edition2024 is required`, run `rustup default stable`.

## Commands & Local Gates

- Build: `cargo build --workspace --locked`
- Test: `cargo test --workspace --locked`
- Lint: `cargo clippy --workspace --locked`
- Web: `cd cloud/apps/web && npm run typecheck && npm test && npm run build`
- Local Cloud: `cd cloud && ./scripts/dev.sh`
- Install ACP binary: `./scripts/install.sh`

## Knowledge Tiering & Token Budget Discipline

| Tier | Content | Entry Point |
| --- | --- | --- |
| **Standing Constraints** | Inviolable rules across all tasks | This file ("Always Active"); expanded in [`docs/ai/agents/`](docs/ai/agents/) |
| **Reusable Workflows** | Domain procedures & validation commands | `.agents/skills/*/SKILL.md` (Authoritative) |
| **Visual & UI Specs** | Console UI design tokens, layout & surfaces | [`docs/design.md`](docs/design.md) + [`docs/design/tokens.md`](docs/design/tokens.md) |
| **Console Capabilities**| Feature slices & reuse | `cloud/apps/web/lib/capabilities.ts` + `@/cap/<id>` |

- **Token Budget & Zero-Sum Updates**: As a resident system prompt weight, this file has a strict hard limit of **80 lines / 1200 Tokens**. Follow the **zero-sum rule (add one, remove one)**.
- **Anti-Anecdote & Batch Threshold**: Never add global rules based on isolated single-session mistakes. Rules must appear in **≥ 2 independent session transcripts** and be refined via skill [`promote-lesson`](.agents/skills/promote-lesson/SKILL.md) with explicit human review.

## Agent Reading Map

| Task Signal | Required Reading |
| --- | --- |
| Any visible page / report / landing / HTML / Admin UI | skill [`admin-ui-change`](.agents/skills/admin-ui-change/SKILL.md) → [`docs/design.md`](docs/design.md); details in [`ui-entry.md`](docs/ai/agents/ui-entry.md) |
| Admin domain modules / API contract tiering | skill [`admin-domain-resource`](.agents/skills/admin-domain-resource/SKILL.md) |
| `git stash` operations | skill [`git-stash-safe`](.agents/skills/git-stash-safe/SKILL.md) |
| Writing design docs in `docs/` / DDL / Mermaid | skill [`verify-design-doc`](.agents/skills/verify-design-doc/SKILL.md) |
| Code review / PR audit / acceptance verification | skill [`review`](.agents/skills/review/SKILL.md) (Independent read-only context) |
| Pre-push local quality gates | skill [`pre-push-local-gates`](.agents/skills/pre-push-local-gates/SKILL.md) |
| Release / tagging / production deployment | skill [`release`](.agents/skills/release/SKILL.md) |
| Commit message conventions | [`commit-style.md`](docs/ai/agents/commit-style.md) |
| Autonomous agent loops / engineering daemons | [`engineering.md`](docs/ai/agents/engineering.md) + [`loop-charter.md`](docs/ai/agents/loop-charter.md) |

## Always Active (Highest Standing Constraints)

1. **No Piggybacking**: Commits/PRs must not carry unrelated changes; unannounced tuning, repo-wide formatting, undocumented `#[allow]`, and cross-module opportunistic refactoring are strictly prohibited.
2. **Zero Hallucination Code**: Every definition must have callers; every cache field must have a store policy; metrics must track both success and failure; every `TODO` must reference an issue.
3. **Console UI & Capability Stack**: UI lives strictly in `cloud/apps/web/` using named capabilities (`@/cap/<id>`). Never invent parallel fetchers/pickers. Pages must not call raw path strings (`fetch` or `api("/api/v1/...")`). No native `<select>` or `window.alert`/`confirm`.
4. **UI Tokens & Quiet Selection**: Always use semantic HSL tokens (`primary`, `card`, `border`, `muted`). Brand identity is `#2744A5`. Active/selected state is quiet selection: light primary fill `bg-primary/10` without theme-colored left border or accent bars.
5. **One Primary Button Per View**: Only one primary filled action button per view. Secondary / ghost for the rest.
6. **Dialog Viewport & Dismiss**: Dialogs must enforce `max-h-[85vh]` (or `max-h-[90vh]`) with `overflow-y-auto`. Large code blocks must use collapsible accordions/tabs. Overlay and Escape dismiss are mandatory.
7. **No Casual Subtitles**: Do not add subtitles under titles that merely repeat titles or fill space. Explanatory copy belongs in empty states, dialogs, or docs.
8. **Release Guardrail & Gate Check**: Run full local quality gates before push via skill [`pre-push-local-gates`](.agents/skills/pre-push-local-gates/SKILL.md). Merging to main or creating release tags is strictly prohibited without explicit human approval.
9. **Workflow & PRs**: Code and docs changes go through GitHub PRs (`branch → PR → review → merge`; reference `Refs: ParaTensor/zene#N`). Direct pushes to `main` are limited to release chores and trivial fixes.
