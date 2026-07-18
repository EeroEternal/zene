# Cloudflare Pages (paused)

The `zene-docs` site is connected via the **Cloudflare Workers and Pages** GitHub App (not a workflow under `.github/workflows/`). That is why PRs get `cloudflare-workers-and-pages` deploy comments even though `ci.yml` only runs `cargo test`.

## Temporary pause (until project is stable)

In the Cloudflare dashboard:

1. Open **Workers & Pages** → project **zene-docs**
2. **Settings** → **Builds** (branch control)
3. Set **Preview branch** to **None** (disable automatic preview deployments)
4. Turn off **Enable automatic production branch deployments**

Direct uploads via `deploy-web.sh` option 3 stay blocked unless `ZENE_PAGES_DEPLOY=1`.

## Re-enable later

Restore preview/production automatic deployments in the same settings when ready, and remove the `ZENE_PAGES_DEPLOY` gate from `deploy-web.sh` if desired.
