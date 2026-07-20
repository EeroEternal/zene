# Zene Web Agent UI

Static Web Agent UI served by `zene-gateway`.

This is a zero-build HTML/JS page embedded into the gateway binary via
`include_str!`. Phase C panels include Plan, Todos, background tasks,
terminals, mode switching, and session close.

Open via the URL printed by `zene-gateway` (includes `#token=...`).
