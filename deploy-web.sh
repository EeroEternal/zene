#!/usr/bin/env bash
set -euo pipefail

# Zene Web deployment helper script.
# This script makes it easy to preview locally and deploy/sync the website to zene.sh.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "=== Zene Web Deployment Helper ==="
echo "1. Run local preview server (localhost:8000)"
echo "2. Push changes to GitHub (Triggers CI/CD deployment on main)"
echo "3. Direct deploy to Cloudflare Pages via Wrangler"
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
    echo "Sync complete! CI/CD will build and deploy the changes to https://zene.sh/ shortly."
    ;;
  3)
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
