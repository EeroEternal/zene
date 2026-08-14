# Cloud runtime

- Cloud Console UI: `cloud/apps/web/` (served by `zene-cloud-api`). Local: `cd cloud && ./scripts/dev.sh`. Production: GCP VM behind Cloudflare (`zene.run`); see `cloud/deploy/`.
- `zene` (`apps/cli`): `zene acp` stdio ACP for Cloud workers and editors (interactive REPL and headless `-p` were removed).

The agent makes outbound HTTPS calls to an external LLM (OpenAI-compatible or Anthropic). Cloud workers use `zene acp` via `cloud/crates/acp-bridge`. Console users set LLM keys in Settings (BYOK); env / `~/.zene/config.toml` still apply to the ACP process when injected.

## UI HMR

Keep the API on `:8788`, then `cd cloud/apps/web && npm run dev` → `:8787` (rewrites `/api/*` to the API). Optional: `ZENE_CLOUD_SKIP_WEB_BUILD=1`.
