# Documentation Layout & Lifecycle

This document describes the structure and lifecycle of the `docs/` tree.

## Directory Structure

- `docs/architecture.md`: High-level system architecture, crate responsibilities, and component boundaries.
- `docs/ENGINE.md`: Detailed agent loop, compaction algorithms, and token estimation notes.
- `docs/context-engine.md`: Semantic context projection, prefix cache, and KV cache layout.
- `docs/session-as-source-of-truth.md`: Architectural model separating durable session facts from ephemeral context projections.
- `docs/ai/agents/`: Engineering hard rules, commit conventions, loop charters, and agent governance.

## Document Lifecycle Discipline

1. **Zero UI in Core**: Zene core does not contain frontend or admin UI specifications; all console UI specifications live in `zene-cloud`.
2. **No Phantom Capabilities**: Never document skeleton-only or hypothetical features as ready.
3. **Deterministic Verification**: Architecture claims, configuration keys, and code snippets inside documentation must reflect real crate implementations.
