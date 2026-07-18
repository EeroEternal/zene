#!/usr/bin/env bash
set -euo pipefail

# Zene Web deployment helper script.
# Cloudflare Pages auto-deploy (GitHub App → zene-docs) is PAUSED until the
# project is stable. Re-enable in the Cloudflare dashboard:
#   Workers & Pages → zene-docs → Settings → Builds
#   - Preview branch: None
#   - Disable automatic production branch deployments
# Direct Wrangler deploy requires ZENE_PAGES_DEPLOY=1.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

pages_deploy_allowed() {
  [[ "${ZENE_PAGES_DEPLOY:-0}" == "1" ]]
}

echo "=== Zene Web Deployment Helper ==="
echo "1. Run local preview server (localhost:8000)"
echo "2. Push website changes to GitHub (Pages auto-deploy currently paused)"
echo "3. Direct deploy to Cloudflare Pages via Wrangler (requires ZENE_PAGES_DEPLOY=1)"
echo "4. Exit"
echo

read -p "Select an option [1-4]: " OPTION

case "$OPTION" in
  1)
    echo "Starting Python HTTP server on port 8000..."
    echo "Open http://localhost:8000 in your browser."
    python3 -m http.server 8000 --directory web
    ;;
  2)
    echo "Staging files in web/..."
    git add web/

    echo "Enter commit message (default: 'update website content'):"
    read -r MSG
    MSG=${MSG:-"update website content"}

    git commit -m "$MSG"

    echo "Pushing to main branch..."
    git push origin main
    echo "Pushed. Note: Cloudflare Pages auto-deploy is paused until the project is stable."
    ;;
  3)
    if ! pages_deploy_allowed; then
      echo "Cloudflare Pages deploy is paused (project not stable yet)."
      echo "Set ZENE_PAGES_DEPLOY=1 to override, or re-enable builds in the Cloudflare dashboard."
      exit 1
    fi
    echo "Checking if wrangler is installed..."
    if ! command -v npx &> /dev/null; then
      echo "Error: npx/npm is not installed on this machine."
      exit 1
    fi
    echo "Deploying 'web' directory to Cloudflare Pages..."
    npx wrangler pages deploy web --project-name zene-docs --branch main
    ;;
  *)
    echo "Exited."
    exit 0
    ;;
esac
