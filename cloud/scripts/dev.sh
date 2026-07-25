#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO_ROOT="$(cd "$ROOT/.." && pwd)"
cd "$ROOT"

mkdir -p data/workspaces

# Optional local GitHub App credentials (copy github.env.example → github.env).
if [[ -f "$ROOT/github.env" ]]; then
  set -a
  # shellcheck disable=SC1091
  source "$ROOT/github.env"
  set +a
fi

export ZENE_CLOUD_DATABASE_URL="${ZENE_CLOUD_DATABASE_URL:-sqlite:$ROOT/data/zene-cloud.db}"
export ZENE_CLOUD_WORKER_TOKEN="${ZENE_CLOUD_WORKER_TOKEN:-dev-worker-token}"
export ZENE_CLOUD_WEB_DIR="${ZENE_CLOUD_WEB_DIR:-$ROOT/apps/web/dist}"
export ZENE_CLOUD_WORKSPACE_ROOT="${ZENE_CLOUD_WORKSPACE_ROOT:-$ROOT/data/workspaces}"
export ZENE_CLOUD_API_URL="${ZENE_CLOUD_API_URL:-http://127.0.0.1:8788}"
export ZENE_CLOUD_PUBLIC_BASE_URL="${ZENE_CLOUD_PUBLIC_BASE_URL:-http://127.0.0.1:8788}"
export ZENE_CLOUD_GITHUB_MODE="${ZENE_CLOUD_GITHUB_MODE:-live}"
export ZENE_CLOUD_PUSH_PR="${ZENE_CLOUD_PUSH_PR:-1}"
export ZENE_CLOUD_ALLOW_MOCK="${ZENE_CLOUD_ALLOW_MOCK:-1}"
export ZENE_CLOUD_ACP_YOLO="${ZENE_CLOUD_ACP_YOLO:-1}"
# Keep localhost API calls off the system proxy (Clash/V2Ray etc. often break claim).
export NO_PROXY="${NO_PROXY:+$NO_PROXY,}127.0.0.1,localhost"
export no_proxy="${no_proxy:+$no_proxy,}127.0.0.1,localhost"

resolve_zene_bin() {
  local candidates=(
    "${ZENE_BIN:-}"
    "$REPO_ROOT/target/debug/zene"
    "$REPO_ROOT/target/release/zene"
    "/workspace/target/debug/zene"
    "/workspace/target/release/zene"
  )
  local path
  for path in "${candidates[@]}"; do
    if [[ -n "$path" && -x "$path" ]]; then
      # Prefer absolute path for worker logs / restart stability.
      (cd "$(dirname "$path")" && echo "$(pwd)/$(basename "$path")")
      return 0
    fi
  done
  return 1
}

if ! ZENE_BIN="$(resolve_zene_bin)"; then
  echo "zene binary not found; building zene-cli at repo root..."
  (cd "$REPO_ROOT" && cargo build -p zene-cli)
  if ! ZENE_BIN="$(resolve_zene_bin)"; then
    echo "error: failed to locate zene after build" >&2
    exit 1
  fi
fi
export ZENE_BIN

echo "building web UI (apps/web → dist)..."
(cd "$ROOT/apps/web" && npm run build)

echo "building zene-cloud-api and zene-cloud-worker..."
cargo build -p zene-cloud-api -p zene-cloud-worker

cleanup() {
  if [[ -n "${API_PID:-}" ]]; then kill "$API_PID" 2>/dev/null || true; fi
  if [[ -n "${WORKER_PID:-}" ]]; then kill "$WORKER_PID" 2>/dev/null || true; fi
}
trap cleanup EXIT

./target/debug/zene-cloud-api \
  --bind 127.0.0.1:8788 \
  --database-url "$ZENE_CLOUD_DATABASE_URL" \
  --worker-token "$ZENE_CLOUD_WORKER_TOKEN" \
  --web-dir "$ZENE_CLOUD_WEB_DIR" \
  --workspace-root "$ZENE_CLOUD_WORKSPACE_ROOT" \
  --public-base-url "$ZENE_CLOUD_PUBLIC_BASE_URL" &
API_PID=$!

sleep 1

WORKER_ARGS=(
  --api-url "$ZENE_CLOUD_API_URL"
  --worker-token "$ZENE_CLOUD_WORKER_TOKEN"
  --workspace-root "$ZENE_CLOUD_WORKSPACE_ROOT"
  --zene-bin "$ZENE_BIN"
  --acp-yolo
  --push-pr
)
if [[ "$ZENE_CLOUD_ALLOW_MOCK" == "1" || "$ZENE_CLOUD_ALLOW_MOCK" == "true" ]]; then
  WORKER_ARGS+=(--allow-mock)
fi
./target/debug/zene-cloud-worker "${WORKER_ARGS[@]}" &
WORKER_PID=$!

echo
echo "Zene Cloud is running:"
echo "  Web/API:     http://127.0.0.1:8788/"
echo "  GitHub mode: $ZENE_CLOUD_GITHUB_MODE"
if [[ "$ZENE_CLOUD_GITHUB_MODE" == "live" ]]; then
  if [[ -n "${GITHUB_APP_ID:-}" && -n "${GITHUB_APP_SLUG:-}" && -f "${GITHUB_APP_PRIVATE_KEY_PATH:-}" ]]; then
    echo "  GitHub App:  $GITHUB_APP_SLUG (id $GITHUB_APP_ID)"
  else
    echo "  GitHub App:  NOT CONFIGURED"
    echo "               1) GitHub App → Generate a private key → save to ~/.zene/github-app.pem"
    echo "               2) cp github.env.example github.env && ./scripts/dev.sh"
  fi
fi
echo "  ZENE_BIN:    $ZENE_BIN"
echo "  Allow mock:  $ZENE_CLOUD_ALLOW_MOCK"
echo
echo "Flow: Register → Settings (LLM BYOK) → Connect GitHub → New Agent → Approve/Files/Diff/PR"
echo "Press Ctrl+C to stop."

wait
