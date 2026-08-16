# Cloud runtime

- Cloud Console UI: `cloud/apps/web/` (served by `zene-cloud-api`). Local: `cd cloud && ./scripts/dev.sh`. Production: GCP VM behind Cloudflare (`zene.run`); see `cloud/deploy/`.
- `zene` (`apps/cli`): `zene acp` stdio ACP for Cloud workers and editors (interactive REPL and headless `-p` were removed).

The agent makes outbound HTTPS calls to an external LLM (OpenAI-compatible or Anthropic). Cloud workers use `zene acp` via `cloud/crates/acp-bridge`. Console users set LLM keys in Settings (BYOK); env / `~/.zene/config.toml` still apply to the ACP process when injected.

Agent `gh` / git must not inherit the worker host login (`GH_TOKEN`, `~/.config/gh`). The worker writes a run-private installation token under the git workspace (`.zene/github/`, gitignored, refreshed via clone-auth) so the sandbox can read it, puts a `gh` wrapper and git credential helper on `PATH`, and points `origin` at the public GitHub URL. Console Push/PR still goes through git-broker.

A Cloud **workspace** is one on-disk checkout per organization + repository (`{workspace_root}/ws/{workspace_id}`). New sessions (runs) for that repo reuse it and skip clone; they only `git checkout -B` a session branch from the current HEAD. Concurrent live agents on the same checkout can conflict.

## UI HMR

Keep the API on `:8788`, then `cd cloud/apps/web && npm run dev` → `:8787` (rewrites `/api/*` to the API). Optional: `ZENE_CLOUD_SKIP_WEB_BUILD=1`.
