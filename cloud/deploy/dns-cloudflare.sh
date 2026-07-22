#!/usr/bin/env bash
# Upsert Cloudflare A records for zene.run → STATIC_IP (proxied).
# Requires: CLOUDFLARE_API_TOKEN, STATIC_IP
set -euo pipefail

: "${CLOUDFLARE_API_TOKEN:?}"
: "${STATIC_IP:?}"
ZONE_NAME="${ZONE_NAME:-zene.run}"

api() {
  local method="$1" path="$2"
  shift 2
  curl -sS -X "$method" "https://api.cloudflare.com/client/v4${path}" \
    -H "Authorization: Bearer ${CLOUDFLARE_API_TOKEN}" \
    -H "Content-Type: application/json" \
    "$@"
}

ZONE_JSON="$(api GET "/zones?name=${ZONE_NAME}")"
ZONE_ID="$(python3 -c 'import json,sys; d=json.load(sys.stdin); assert d.get("success"), d; print(d["result"][0]["id"])' <<<"$ZONE_JSON")"
echo "Zone ${ZONE_NAME} id=${ZONE_ID}"

upsert_a() {
  local name="$1"
  local list
  list="$(api GET "/zones/${ZONE_ID}/dns_records?type=A&name=${name}")"
  local rid
  rid="$(python3 -c 'import json,sys; d=json.load(sys.stdin); r=d.get("result") or []; print(r[0]["id"] if r else "")' <<<"$list")"
  local body
  body="$(python3 -c "import json; print(json.dumps({'type':'A','name':'${name}','content':'${STATIC_IP}','ttl':1,'proxied':True}))")"
  if [[ -n "$rid" ]]; then
    api PUT "/zones/${ZONE_ID}/dns_records/${rid}" --data "$body" \
      | python3 -c 'import json,sys; d=json.load(sys.stdin); assert d.get("success"), d; print("updated", d["result"]["name"], d["result"]["content"])'
  else
    api POST "/zones/${ZONE_ID}/dns_records" --data "$body" \
      | python3 -c 'import json,sys; d=json.load(sys.stdin); assert d.get("success"), d; print("created", d["result"]["name"], d["result"]["content"])'
  fi
}

upsert_a "$ZONE_NAME"
upsert_a "www.${ZONE_NAME}"

# Prefer Full SSL when origin has a cert (Caddy). Ignore failures if token lacks permission.
api PATCH "/zones/${ZONE_ID}/settings/ssl" --data '{"value":"full"}' \
  | python3 -c 'import json,sys; d=json.load(sys.stdin); print("ssl", d.get("success"), (d.get("result") or {}).get("value"), d.get("errors"))' || true

echo "DNS done: ${ZONE_NAME} / www.${ZONE_NAME} → ${STATIC_IP} (proxied)"
