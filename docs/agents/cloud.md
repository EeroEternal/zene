# Cloud runtime

- Cloud Console UI: `cloud/apps/web/` (served by `zene-cloud-api`). Local: `cd cloud && ./scripts/dev.sh`. Production: GCP VM behind Cloudflare (`zene.run`); see `cloud/deploy/`.
- `zene` (`apps/cli`): `zene acp` stdio ACP for Cloud workers and editors (interactive REPL and headless `-p` were removed).

The agent makes outbound HTTPS calls to an external LLM (OpenAI-compatible or Anthropic). Cloud workers use `zene acp` via `cloud/crates/acp-bridge`. Console users set LLM keys in Settings (BYOK); env / `~/.zene/config.toml` still apply to the ACP process when injected.

Agent `gh` / git must not inherit the worker host login (`GH_TOKEN`, `~/.config/gh`). The worker writes a run-private installation token under the git workspace (`.zene/github/`, gitignored, refreshed via clone-auth) so clone/fetch can use it if needed. **Publish has one path:** the `PublishGithub` tool (and Console **Commit & Create PR**) call git-broker on `ws/{workspace_id}` — commit dirty files, push the session branch, open a draft PR. Cloud Bash hard-denies `git push` / `gh`; the workspace `pre-push` hook also rejects local pushes. Do not SSH to another host to publish.

A Cloud **workspace** is a logical grouping per organization + repository (bare cache `{workspace_root}/.repo-cache/{repository_id}`). Each run gets its own git worktree at `{workspace_root}/ws/{workspace_id}/runs/{run_id}`, created from `origin/<base_ref>`. Concurrent runs must not share a working directory or inherit another run's dirty files.

## UI HMR

Keep the API on `:8788`, then `cd cloud/apps/web && npm run dev` → `:8787` (rewrites `/api/*` to the API). Optional: `ZENE_CLOUD_SKIP_WEB_BUILD=1`.
