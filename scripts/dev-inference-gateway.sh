#!/usr/bin/env bash
# Dev stub: inference gateway (delta assembly + upstream proxy).
#
# Usage:
#   ./scripts/dev-inference-gateway.sh
#
# Then point zene at the gateway:
#   export ZENE_INFERENCE_GATEWAY_URL=http://127.0.0.1:8790
#   export ZENE_UPSTREAM_URL=https://api.openai.com/v1   # on gateway process (default)
#   export ZENE_API_KEY=sk-...
#
# Local dev uses in-memory session store by default (no Redis required).
# To exercise Redis locally (multi-instance / production parity):
#   export ZENE_SESSION_REDIS_URL=redis://127.0.0.1/
#
# Zene auto-routes LLM traffic to {ZENE_INFERENCE_GATEWAY_URL}/v1 when the env is set.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

export ZENE_GATEWAY_LISTEN="${ZENE_GATEWAY_LISTEN:-127.0.0.1:8790}"
export ZENE_UPSTREAM_URL="${ZENE_UPSTREAM_URL:-https://api.openai.com/v1}"
export RUST_LOG="${RUST_LOG:-zene_inference_gateway=info,tower_http=info}"

exec cargo run -p zene-inference-gateway --locked
