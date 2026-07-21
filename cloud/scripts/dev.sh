#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

mkdir -p data/workspaces

export ZENE_CLOUD_DATABASE_URL="${ZENE_CLOUD_DATABASE_URL:-sqlite:$ROOT/data/zene-cloud.db}"
export ZENE_CLOUD_WORKER_TOKEN="${ZENE_CLOUD_WORKER_TOKEN:-dev-worker-token}"
export ZENE_CLOUD_WEB_DIR="${ZENE_CLOUD_WEB_DIR:-$ROOT/apps/web/dist}"
export ZENE_CLOUD_WORKSPACE_ROOT="${ZENE_CLOUD_WORKSPACE_ROOT:-$ROOT/data/workspaces}"
export ZENE_CLOUD_API_URL="${ZENE_CLOUD_API_URL:-http://127.0.0.1:8788}"

echo "building zene-cloud-api and zene-cloud-worker..."
cargo build -p zene-cloud-api -p zene-cloud-worker

cleanup() {
  if [[ -n "${API_PID:-}" ]]; then kill "$API_PID" 2>/dev/null || true; fi
  if [[ -n "${WORKER_PID:-}" ]]; then kill "$WORKER_PID" 2>/dev/null || true; fi
}
trap cleanup EXIT

cargo run -p zene-cloud-api -- \
  --bind 127.0.0.1:8788 \
  --database-url "$ZENE_CLOUD_DATABASE_URL" \
  --worker-token "$ZENE_CLOUD_WORKER_TOKEN" \
  --web-dir "$ZENE_CLOUD_WEB_DIR" &
API_PID=$!

sleep 1

cargo run -p zene-cloud-worker -- \
  --api-url "$ZENE_CLOUD_API_URL" \
  --worker-token "$ZENE_CLOUD_WORKER_TOKEN" \
  --workspace-root "$ZENE_CLOUD_WORKSPACE_ROOT" &
WORKER_PID=$!

echo
echo "Zene Cloud Phase 0 is running:"
echo "  Web/API:  http://127.0.0.1:8788/"
echo "  Worker:   $WORKER_PID"
echo
echo "Press Ctrl+C to stop."

wait
