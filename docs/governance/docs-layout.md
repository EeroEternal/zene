# Documentation Layout & Lifecycle

This document describes the structure and lifecycle of the `docs/` tree.

## Directory Structure

### Core Specifications
- `docs/architecture.md`: 5-tier system architecture, crate responsibilities, and runtime boundaries.
- `docs/ENGINE.md`: Core agent loop, compaction algorithms, and token estimation notes.
- `docs/context-engine.md`: Semantic context projection, 3-zone layout, prefix caching, and governance.
- `docs/session-as-source-of-truth.md`: Architectural model separating durable session facts from ephemeral projections.
- `docs/agent-components.md`: Composable agent foundation components, dependency ordering, and composition rules.
- `docs/agent-inference-context.md`: Inference-layer session linkage, prefix publishing, and delta assembly.
- `docs/agent-notes-design.md`: Agent Notes 3-layer storage, discovery, and active invariants lifecycle.
- `docs/ROADMAP.md`: Project vision, completed foundations, active milestones, and future directions.

### Specialized Subdirectories
- `docs/governance/`: Engineering hard rules, commit conventions, loop charters, and agent governance.
- `docs/research/`: Architectural comparisons, benchmarks, and external harness research (e.g. Pi, DeepSeek).
- `docs/archive/`: Historical implementation wave logs and closed-out design iterations.

## Document Lifecycle Discipline

1. **Zero UI in Core**: Zene core does not contain frontend or admin UI specifications; all console UI specifications live in `zene-cloud`.
2. **No Phantom Capabilities**: Never document skeleton-only or hypothetical features as ready.
3. **Deterministic Verification**: Architecture claims, configuration keys, and code snippets inside documentation must reflect real crate implementations.
