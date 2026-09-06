# AGENTS.md — Code Agent Collaboration Specification

Zene is an open-source coding-agent framework and CLI toolchain. The `zene` binary (`apps/cli`) speaks Agent Client Protocol (`zene acp`) for external clients, workers, and editors. For the hosted web platform, see `zene-cloud`.

Primary toolchain is Cargo (Rust workspace). Rust >= 1.85 (`unigateway-sdk` needs `edition2024`); if you hit `feature edition2024 is required`, run `rustup default stable`.

## Commands & Local Gates

- Build: `cargo build --workspace --locked`
- Test: `cargo test --workspace --locked`
- Lint: `cargo clippy --workspace --locked`
- Install binary: `./scripts/install.sh`

## Knowledge Tiering & Token Budget Discipline

| Tier | Content | Entry Point |
| --- | --- | --- |
| **Standing Constraints** | Inviolable rules across all tasks | This file ("Always Active"); expanded in [`docs/ai/agents/`](docs/ai/agents/) |
| **Reusable Workflows** | Domain procedures & validation commands | `.agents/skills/*/SKILL.md` (Authoritative) |
| **Commit Conventions** | Conventional commits & PR standards | [`commit-style.md`](docs/ai/agents/commit-style.md) |
| **Daemon Loops** | Engineering loops & charters | [`engineering.md`](docs/ai/agents/engineering.md) + [`loop-charter.md`](docs/ai/agents/loop-charter.md) |

- **Token Budget & Zero-Sum Updates**: As a resident system prompt weight, this file has a strict hard limit of **80 lines / 1200 Tokens**. Follow the **zero-sum rule (add one, remove one)**.
- **Anti-Anecdote & Batch Threshold**: Never add global rules based on isolated single-session mistakes. Rules must appear in **≥ 2 independent session transcripts** and be refined via skill [`promote-lesson`](.agents/skills/promote-lesson/SKILL.md) with explicit human review.

## Agent Reading Map

| Task Signal | Required Reading |
| --- | --- |
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
3. **Release Guardrail & Gate Check**: Run full local quality gates before push via skill [`pre-push-local-gates`](.agents/skills/pre-push-local-gates/SKILL.md). Merging to main or creating release tags is strictly prohibited without explicit human approval.
4. **Workflow & PRs**: Code and docs changes go through GitHub PRs (`branch → PR → review → merge`; reference `Refs: ParaTensor/zene#N`). Direct pushes to `main` are limited to release chores and trivial fixes.
