# Zene Web Agent UI

Static Web Agent UI served by `zene-gateway`.

Phase B keeps this as a zero-build HTML/JS page embedded into the gateway
binary via `include_str!`. Later phases may introduce a bundler under the same
directory without changing the Gateway/ACP contract.

Open via the URL printed by `zene-gateway` (includes `#token=...`).
