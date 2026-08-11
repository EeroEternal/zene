#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$ROOT/data/verify-supervisor-run"
rm -rf "$TMP"
mkdir -p "$TMP/workspaces"

export NO_PROXY=127.0.0.1,localhost
export no_proxy=127.0.0.1,localhost
API_URL=http://127.0.0.1:18788
TOKEN=dev-worker-token

# Free the verify port from prior runs.
lsof -tiTCP:18788 -sTCP:LISTEN 2>/dev/null | xargs kill 2>/dev/null || true
sleep 0.3

export ZENE_CLOUD_GITHUB_MODE=mock
"$ROOT/target/debug/zene-cloud-api" \
  --bind 127.0.0.1:18788 \
  --database-url "sqlite:$TMP/db.sqlite" \
  --worker-token "$TOKEN" \
  --web-dir "$ROOT/apps/web/dist" \
  --workspace-root "$TMP/workspaces" \
  --public-base-url "$API_URL" \
  >"$TMP/api.log" 2>&1 &
API_PID=$!

cleanup() {
  if [[ -n "${SUP_PID:-}" ]]; then
    kill "$SUP_PID" 2>/dev/null || true
    pkill -P "$SUP_PID" 2>/dev/null || true
  fi
  kill "$API_PID" 2>/dev/null || true
  wait "$SUP_PID" 2>/dev/null || true
  wait "$API_PID" 2>/dev/null || true
}
trap cleanup EXIT

for _ in $(seq 1 40); do
  if curl -fsS --noproxy '*' "$API_URL/api/v1/health" >/dev/null 2>&1; then
    break
  fi
  sleep 0.25
done
curl -fsS --noproxy '*' "$API_URL/api/v1/health" >/dev/null

# Force MockAgent: clear ZENE_BIN and avoid repo cwd so resolve_zene_bin finds nothing.
(
  cd /tmp
  env -u ZENE_BIN "$ROOT/target/debug/zene-cloud-worker" \
    --supervisor \
    --api-url "$API_URL" \
    --worker-token "$TOKEN" \
    --workspace-root "$TMP/workspaces" \
    --zene-bin /nonexistent-zene-bin \
    --allow-mock \
    --acp-yolo \
    --acp-idle-secs 30 \
    --min-warm 1 \
    --max-active 4 \
    --max-hold 8 \
    --scale-interval-ms 300 \
    >"$TMP/sup.log" 2>&1
) &
SUP_PID=$!

child_count() {
  local pids
  pids=$(pgrep -P "$1" 2>/dev/null || true)
  if [[ -z "$pids" ]]; then
    echo 0
  else
    printf '%s\n' "$pids" | wc -l | tr -d ' '
  fi
}

for _ in $(seq 1 30); do
  n=$(child_count "$SUP_PID")
  if [[ "$n" -ge 1 ]]; then
    echo "warm_executors=$n"
    break
  fi
  sleep 0.25
done
n=$(child_count "$SUP_PID")
echo "children_after_warm=$n"
[[ "$n" -ge 1 ]]

EMAIL="sup-verify-$(date +%s)@example.com"
REG=$(curl -sS --noproxy '*' -X POST "$API_URL/api/v1/auth/register" \
  -H 'content-type: application/json' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"password123\",\"displayName\":\"Sup\"}")
echo "register_ok=$(printf '%s' "$REG" | python3 -c 'import json,sys; print("token" in json.load(sys.stdin))')"
USER_TOKEN=$(printf '%s' "$REG" | python3 -c 'import json,sys; print(json.load(sys.stdin)["token"])')

curl -fsS --noproxy '*' -X PUT "$API_URL/api/v1/settings/llm" \
  -H "authorization: Bearer $USER_TOKEN" -H 'content-type: application/json' \
  -d '{"providerId":"openai","apiKey":"sk-verify","baseUrl":"https://example.invalid/v1","defaultModel":"mock","models":["mock"]}' \
  >/dev/null

REPO=$(curl -sS --noproxy '*' -X POST "$API_URL/api/v1/repositories" \
  -H "authorization: Bearer $USER_TOKEN" -H 'content-type: application/json' \
  -d '{"owner":"ada","name":"demo","defaultBranch":"main"}')
REPO_ID=$(printf '%s' "$REPO" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
echo "repo_id=$REPO_ID"

for title in one two; do
  RUN=$(curl -sS --noproxy '*' -X POST "$API_URL/api/v1/runs" \
    -H "authorization: Bearer $USER_TOKEN" -H 'content-type: application/json' \
    -d "{\"repositoryId\":\"$REPO_ID\",\"prompt\":\"verify $title\",\"model\":\"default\",\"permissionMode\":\"yolo\"}")
  echo "run_$title=$(printf '%s' "$RUN" | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d.get("id",d.get("message","?"))[:36])')"
done

ok=0
saw_two_children=0
for i in $(seq 1 40); do
  STATS=$(curl -fsS --noproxy '*' -H "authorization: Bearer $TOKEN" \
    "$API_URL/internal/v1/queue/stats")
  Q=$(printf '%s' "$STATS" | python3 -c 'import json,sys; print(json.load(sys.stdin)["queued"])')
  A=$(printf '%s' "$STATS" | python3 -c 'import json,sys; print(json.load(sys.stdin)["active"])')
  H=$(printf '%s' "$STATS" | python3 -c 'import json,sys; print(json.load(sys.stdin)["holding"])')
  CHILDREN=$(child_count "$SUP_PID")
  WORKERS=$(sqlite3 "$TMP/db.sqlite" "SELECT COUNT(DISTINCT worker_id) FROM run_attempts;" 2>/dev/null || echo 0)
  echo "t=$i queued=$Q active=$A holding=$H children=$CHILDREN distinct_workers=$WORKERS"
  if [[ "$CHILDREN" -ge 2 ]]; then
    saw_two_children=1
  fi
  # Parallel claim: two distinct executor worker_ids, or supervisor scaled to 2 while work pending.
  if [[ "$WORKERS" -ge 2 ]]; then
    ok=1
    break
  fi
  if [[ "$saw_two_children" -eq 1 && $((A + H + Q)) -ge 1 ]]; then
    ok=1
    break
  fi
  sleep 0.4
done

echo '--- supervisor log ---'
tail -50 "$TMP/sup.log" || true
echo '--- run statuses ---'
sqlite3 "$TMP/db.sqlite" "SELECT substr(id,1,8), status FROM runs ORDER BY created_at;"
echo '--- attempts ---'
sqlite3 "$TMP/db.sqlite" "SELECT substr(run_id,1,8), worker_id, status FROM run_attempts ORDER BY started_at;"
[[ "$ok" -eq 1 ]]
echo VERIFY_OK
