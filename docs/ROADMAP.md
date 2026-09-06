# Zene Roadmap

Zene is an open-source coding-agent framework and CLI toolchain. Its mission is to provide high-reliability execution, prefix-cache-friendly context management, and clean crate boundaries for local developers, autonomous worker environments, and external IDEs via ACP (Agent Client Protocol).

---

## 1. Milestone Status

```
[Phase 0: Tool Foundation] ──► [Phase 1: Context & Session Engine] ──► [Phase 2: Decoupled Architecture & ACP]
           ✅                                 ✅                                          ✅
                                                                                           │
                                                                                           ▼
                                                           [Phase 3: Robustness & Governance] (Active)
                                                                                           │
                                                                                           ▼
                                                           [Phase 4: Advanced Coordination & Streaming] (Planned)
```

---

## 2. Completed Milestones

### Phase 0 — Tool Foundation & Execution Reliability (✅ Complete)
- **Edit / Write Safety**: Precise line ending normalization (LF/CRLF), unique-match assertions, and atomic writes.
- **Sandboxed Execution**: Isolated shell runs with timeout enforcement, permission boundaries, and configurable profiles.
- **Output Sanitization**: Automated folding of voluminous test success logs, preserving failed assertions and panic traces.
- **MCP Integration**: Stdio and HTTP MCP client implementations with doctor probes.

### Phase 1 — Context & Session Engine (✅ Complete)
- **Session as Source of Truth**: Durable event-sourced conversation journal decoupled from ephemeral model projections.
- **Three-Zone Context Layout**:
  - `[Frozen System Prefix]`: Model base instructions + invariant axioms.
  - `[Append-Oriented Body]`: Sequential conversation history and tool executions.
  - `[Tail Decorations]`: Transient reminders, todos, and status hints injected only at the prompt tail.
- **Compaction & Prefix Caching**:
  - Compaction snapshotting with debouncing;
  - Stable system prefix preserving KV-cache across turns;
  - Delta and Full delivery protocol with `apps/inference-gateway`.

### Phase 2 — Decoupled Architecture & ACP (✅ Complete)
- **17 Crates Separation**: Strict boundaries between runtime (`zene-runtime`), actor (`zene-agent-runtime`), context (`zene-context`), tools (`zene-tools`), and execution (`zene-turn`).
- **ACP Standard Protocol**: Pure `zene acp` server interface supporting tool calls, progress streaming, mode switching, and permissions over stdio JSON-RPC.
- **Zero UI in Core**: Complete extraction of web/frontend consoles to `zene-cloud`.

---

## 3. Active Work (Phase 3 — Robustness & Governance)

| Area | Objective | Status |
| :--- | :--- | :--- |
| **Codebase Entropy Reclaim** | Regular audits with `find-simplifications` skill to prune dead APIs and redundant abstractions | Active |
| **Index & Navigation** | Tree-sitter incremental symbol indexing (`zene-index`) and token-budgeted `RepoMap` tool | Active |
| **Context Governance** | Zero CoT leakage in commits/docs (`trim-cot-leakage`) and Agent Notes lifecycle management | Active |
| **Session Lineage** | Explicit lossless migration pathways for legacy session checkpoints | In Progress |

---

## 4. Planned Horizons (Phase 4 — Advanced Coordination & Streaming)

### 4.1 Durable Subagent Sessions
- Transition subagent execution from in-memory ephemeral messages to durable session journals.
- Lineage tracking connecting parent task intentions with subagent execution outcomes.

### 4.2 Streaming Mid-Turn Steering
- Seamless real-time user steering and cancellations injected into active streaming turns without invalidating prefix caches.

### 4.3 Enhanced Model Adaptation
- Deeper integration with provider-specific prompt-caching headers and token accounting.
- Cross-worker durable outbox abstractions for distributed deployment topologies.

---

## 5. Zen Engine Open Harness (Developer Experience & Embeddability)

### 5.1 Top-Level `zene` Facade SDK
- Provide a clean, idiomatic `Agent::builder()` API for embedding Zene directly into any Rust application or service without boilerplate.

### 5.2 Multi-Transport Backend Serving (`zene serve`)
- Complement `zene acp` (stdio JSON-RPC) with a lightweight HTTP/SSE/WebSocket server mode (`zene serve`), allowing external backend microservices in Node.js, Python, or Go to invoke Zene over standard REST APIs (similar to Flue's `POST /agents/:id`).

### 5.3 Standalone Crates Distribution
- Ensure decoupled foundation crates (`zene-sandbox`, `zene-context`, `zene-tools`, `zene-session`) have standalone documentation, zero unnecessary cross-crate dependencies, and clean crates.io publication workflows.
