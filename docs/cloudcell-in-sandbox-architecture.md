# In-Sandbox Agent Architecture (Scheme B) — CloudCell & Zene-Cloud Integration

- **Status**: Proposed
- **Date**: 2026-09-11
- **Target Repositories**:
  - `cloudcell` (`/Users/xinference/openhub/cloudcell`)
  - `zene-cloud` (`/Users/xinference/github/zene-cloud`)
  - `zene` (`/Users/xinference/github/zene`)
  - `keel` (`/Users/xinference/github/keel`)

---

## 1. Overview & Motivation

In multi-tenant AI coding agent platforms, code execution presents high security risks (untrusted repository scripts, accidental host resource exhaustion, privilege escalation).

Two architectural options were evaluated:
- **Scheme A (Host Agent + Remote Tool Execution / 细粒度工具外置调用)**:
  `zene acp` runs on the host worker; each bash/edit/terminal tool call dispatches an HTTP/RPC request to a remote `cloudcell` sandbox.
- **Scheme B (In-Sandbox Agent / 整机下沉入沙箱架构 - Selected)**:
  The entire `zene acp` engine, workspace worktree, and `keel` execution gate sink into an isolated `agentcell` jail provisioned by `cloudcell`. `zene-cloud Worker` acts as an orchestrator and transparent ACP protocol proxy.

### Why Scheme B?
1. **Zero Host Escape Risk**: The agent context loop, arbitrary bash scripts, tool plugins, and local worktrees all live inside Linux kernel namespaces (user, pid, mount, net) with Landlock and Seccomp restrictions.
2. **Zero Tool Latency**: Filesystem operations, git checkouts, and incremental compiler checks run locally inside the sandbox with NVMe/tmpfs speed, eliminating high-frequency network roundtrips.
3. **Defense in Depth**: `keel` runs in-process as an agent tool policy gate, while `agentcell` provides kernel-level sandboxing.

---

## 2. System Architecture

```
                    +-------------------------------------------------------------+
                    |                      zene-cloud Control                     |
                    |  (API Server + Web Console + Job Dispatcher + Multi-tenant) |
                    +------------------------------+------------------------------+
                                                   |
                                                   | HTTP / REST (Job Claim)
                                                   v
                    +-------------------------------------------------------------+
                    |                      zene-cloud Worker                      |
                    |  - Run lifecycle & lease coordinator                        |
                    |  - Requests sandbox from Cloudcell REST API                 |
                    |  - Bridges ACP JSON-RPC over WebSocket/duplex tunnel        |
                    +---------------+-----------------------------+---------------+
                                    |                             |
             1. POST /api/v1/sandboxes                            | 2. Duplex Stdio Stream
             (Mem/CPU/Egress/Snapshot)                            |    (WebSocket / Tunnel)
                                    v                             v
+---------------------------------------------------------------------------------------------------------+
|                                        cloudcell (Daemon / Gateway)                                     |
|  - REST Control Plane (Sandboxes CRUD)                                                                  |
|  - WebSocket Stdio Stream Bridge (/api/v1/sandboxes/{id}/stream or /acp)                                |
|  - Manages `sand serve` instances over AgentCell exec protocol v2                                       |
+---------------------------------------------------+-----------------------------------------------------+
                                                    |
                                                    | UNIX Domain Socket (AgentCell Exec v2 Protocol)
                                                    v
+ - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - +
:  agentcell Jail (Linux userns + mount + pid + cgroup + landlock + seccomp + eBPF tracepoints)           :
:                                                                                                         :
:   [ In-Sandbox Process Tree ]                                                                           :
:                                                                                                         :
:   zene acp (Session Loop / Context Engine)                                                              :
:        |                                                                                                :
:        |-- Tool Execution (Bash / File Edit / Compiler / Cargo)                                         :
:        v                                                                                                :
:   keel (Tool Gate / Execution Policy / SandboxProfile: in-cell)                                         :
:        |                                                                                                :
:        +--> Local Subprocess (zero network lag / zero host privilege)                                   :
:                                                                                                         :
+ - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - +
```

---

## 3. Protocol & Component Specifications

### 3.1 `cloudcell` Duplex Stream Endpoint
- **Path**: `GET /api/v1/sandboxes/{id}/stream` (WebSocket) or `POST /api/v1/sandboxes/{id}/acp`
- **Authentication**: `Authorization: Bearer <token>` or query param `?token=<token>` for WebSockets.
- **Behavior**:
  1. Authenticates owner and verifies sandbox is in `running` state.
  2. Connects to `sand serve` UNIX socket, sends `encode_argv(&cmd)`.
  3. Binds WebSocket full-duplex stream to stdin/stdout of the jailed process.
  4. On client close / abort, sends `SHUT_WR` and gracefully terminates the session.

### 3.2 `zene-cloud` Worker Adapter
- Implement `CloudcellRuntimeClient` implementing `RuntimeClient`.
- In `execute_run`:
  1. Calls CloudCell `POST /api/v1/sandboxes` with run resource constraints (`mem_bytes`, `cpu`, `egress`).
  2. Connects via WebSocket to `/api/v1/sandboxes/{id}/stream`.
  3. Pipes ACP events through existing `AcpBridge` logic into the Worker's `run_runtime_session`.
  4. On completion or cancellation, calls `DELETE /api/v1/sandboxes/{id}`.

### 3.3 `zene` & `keel` In-Sandbox Constraints
- Avoid nested namespace collision: `zene` running inside an already unprivileged cell sets `ZENE_SANDBOX_PROFILE=in-cell` (or `workspace`).
- `keel` continues to enforce semantic path checking, secret redaction, and dangerous command blocklists without attempting second-tier kernel unshare operations.
