# Agent Runtime 架构优化设计

> 状态：Proposal
>
> 本文基于当前 zene runtime 实现，描述如何将 `Agent`、`Turn`、`Step`、`Session`、Cloud `Run` 和 ACP transport 拉开，并给出渐进式迁移方案。

## 1. 背景与目标

当前 zene 已经具备可用的 coding-agent runtime：

- `zene-turn` 抽象了多步 `LLM → Tool → LLM` 循环；
- `zene-core::Agent` 持有 session、context、tools、sandbox、permission、hooks、MCP 等能力；
- `zene-llm` 统一了 OpenAI-compatible / Anthropic 的模型请求；
- `zene-tools`、`zene-sandbox` 和 `zene-permission` 提供了工具、执行环境和权限边界；
- ACP server 将 `Agent` 暴露给编辑器和 Cloud worker；
- Cloud worker 负责 workspace、worker、approval、heartbeat、commit/PR 等 Job 生命周期。

但当前 `Agent` 仍是一个较大的具体对象，且不同层次之间存在职责重叠：

1. 主 Agent 和 Subagent 使用两套执行循环；
2. `Provider`、`ChatClient`、`ChatBackend` 存在相近但不统一的模型抽象；
3. ACP、Cloud event 和 Core `AgentEvent` 之间有多次语义转换；
4. Session 可以恢复历史，但不能恢复一个正在执行的 turn；
5. ACP server 和 `Agent` 分别维护 prompt queue、active turn、cancel 等控制状态；
6. Cloud `Run`、ACP session、Agent turn 的生命周期边界不够显式。

本文目标不是立刻重写 runtime，而是建立一个可以渐进落地的目标架构：

- 让 `AgentRuntime` 成为可控制的 session runtime；
- 让 `TurnEngine` 成为独立的执行状态机；
- 让 Model、Context、Tools、Permission、Persistence 成为可替换能力；
- 让 ACP 成为 transport，而不是 runtime 语义的承载者；
- 让 Cloud Job 与 Agent Runtime 解耦；
- 让主 Agent 和 Subagent 共享执行语义。

## 2. 现状基线

### 2.1 当前执行链路

```text
Local CLI / ACP Server
        │
        ▼
  zene_core::Agent
        │
        ▼
zene_turn::run_turn_loop
        │
        ├── ContextEngine
        ├── ChatClient
        ├── ToolRegistry
        ├── PermissionGate
        ├── Sandbox
        ├── Hooks / MCP
        └── SessionRecord
```

Cloud 场景额外经过一层进程边界：

```text
Cloud API
   │
   ▼
zene-cloud-worker
   │  claim / workspace / heartbeat / approval / cancel
   ▼
AcpBridge
   │  NDJSON / JSON-RPC
   ▼
zene acp
   │
   ▼
zene_core::Agent
```

### 2.2 当前核心职责位置

| 能力 | 当前实现 | 说明 |
| --- | --- | --- |
| Turn 循环 | `crates/turn/src/turn_loop.rs` | 通用多步循环，已经是较好的抽象 |
| Turn 适配 | `crates/core/src/agent_turn.rs` | 将 `Agent` 接入 `TurnRuntime` |
| Agent 状态 | `crates/core/src/lib.rs` | session、context、tools、permission 等集中在 `Agent` |
| LLM 请求 | `crates/core/src/lib.rs` + `crates/llm` | `Agent` 仍直接依赖具体 `ChatClient` |
| Tool 执行 | `crates/core/src/lib.rs` + `crates/tools` | 权限、hook、plan、scheduler、结果写回集中在 `Agent::run_tools` |
| Tool 并发 | `crates/core/src/tool_scheduler.rs` | 同一模型 step 内进行冲突感知并发 |
| Session | `crates/session` | 保存消息、元数据和记录 |
| Context | `crates/context` | token 估算、compaction、overflow retry |
| Permission | `crates/permission` | 策略判断和 prompter |
| Subagent | `crates/core/src/subagent.rs` | 自己维护一套简化循环 |
| ACP | `apps/cli/src/acp/server.rs` | 将 ACP request 映射到 `Agent` |
| Cloud Job | `cloud/apps/worker/src/main.rs` | claim、workspace、ACP 子进程、Cloud 状态和 Git 生命周期 |

## 3. 核心概念边界

必须明确区分以下概念。

### 3.1 Session

Session 是长期存在的对话状态，包含：

- system prompt；
- user / assistant / tool messages；
- session metadata；
- todos 和 mode；
- context 相关 metadata。

Session 不负责：

- 调用模型；
- 执行工具；
- 等待权限；
- 管理 Cloud worker 或 Git。

### 3.2 Turn

Turn 是一次用户输入到一次最终响应的执行过程：

```text
user prompt
  → model step
  → tool batch
  → model step
  → final answer
```

Turn 负责：

- `TurnId`；
- max steps；
- cancellation；
- steer；
- tool continuation；
- final / incomplete / cancelled outcome。

### 3.3 Step

Step 是一次模型调用及其后续的一批工具调用：

```text
prepare context
  → invoke model once
  → receive assistant message
  → optionally execute tool batch
```

一个 turn 可以包含多个 step，但一个 step 只应包含一次模型调用。

### 3.4 Runtime

Runtime 是一个长期存在、可接受控制命令的 Agent session 执行器。它负责：

- 持有 session 的内存状态；
- 启动和结束 turn；
- 接受 prompt、steer、cancel、approval 等控制；
- 调度 `TurnEngine`；
- 发布统一 runtime events；
- 管理 runtime 级状态。

### 3.5 Cloud Run / Job

Cloud Run 是 Cloud 产品层任务，不等于 Agent turn，也不等于 ACP session。一个 Cloud Run 可以包含多个 follow-up turns：

```text
Cloud Run
  ├── initial Turn
  ├── follow-up Turn
  └── follow-up Turn
```

Cloud Run 额外负责：

- queue claim；
- workspace clone；
- worker heartbeat；
- Cloud approval；
- commit、push、PR；
- worker 和进程生命周期。

### 3.6 ACP

ACP 是 Runtime 的一种 transport：

```text
ACP request → RuntimeCommand
RuntimeEvent → ACP notification
```

ACP 不应成为 Core runtime 的执行抽象，也不应让 Core 依赖 Cloud API。

## 4. 目标架构

```text
Cloud JobRunner ───────┐
                       │ RuntimeClient / ACP
Local CLI ─────────────┤
                       ▼
                 AgentRuntime
                       │
                       ▼
                   TurnEngine
                       │
       ┌───────────────┼────────────────┐
       ▼               ▼                ▼
ContextAssembler  ModelExecutor   ToolExecutor
       │               │                │
       └───────────────┼────────────────┘
                       │
          ApprovalBroker / SessionStore / EventSink
```

建议的依赖方向：

```text
Product / transport
        ↓
Runtime orchestration
        ↓
Turn engine
        ↓
Runtime ports
        ↓
Concrete capabilities
```

依赖方向反过来时应特别谨慎。例如 `zene-core` 不应依赖 Cloud HTTP client，`TurnEngine` 不应依赖 ACP JSON 格式。

## 5. AgentRuntime 设计

### 5.1 对外暴露 Handle，不暴露可变 Agent

调用方不应直接持有 `Arc<AsyncMutex<Agent>>` 并操作 Agent 内部状态，而应使用 command/event handle：

```rust
pub struct RuntimeHandle {
    commands: mpsc::Sender<RuntimeCommand>,
    // 实际实现可使用 broadcast、mpsc 或专用 EventStream。
}

pub enum RuntimeCommand {
    Prompt { text: String },
    Steer { text: String },
    Cancel,
    Approval {
        request_id: String,
        decision: ApprovalDecision,
    },
    SetMode { mode_id: String },
    Shutdown,
}
```

目标是让 runtime 成为单一状态所有者：

```text
Runtime actor
  ├── session
  ├── active turn
  ├── steer queue
  ├── pending approval
  ├── todos / mode
  └── runtime state
```

外部只能发送命令和订阅事件，不能直接修改这些字段。这样可以消除 ACP server、Agent、Cloud worker 各自维护控制状态的问题。

### 5.2 Runtime 状态

建议引入显式状态模型：

```rust
pub enum ExecutionState {
    Idle,
    Running { turn_id: TurnId, step: u32 },
    AwaitingApproval { request_id: String },
    AwaitingUser,
    Completed,
    Failed,
    Cancelled,
}
```

`Cloud Run` 状态与 `ExecutionState` 不应强行一一对应。比如：

```text
Cloud Run = WaitingForUser
Runtime   = Idle
```

或：

```text
Cloud Run = Running
Runtime   = AwaitingApproval
```

Cloud 只应将 runtime 状态映射为产品需要展示的 Job 状态。

## 6. Runtime Ports

不要创建一个包含所有职责的巨大 `AgentRuntime` trait。应拆成少量稳定的 capability interfaces，由 runtime 进行编排。

### 6.1 ModelExecutor

统一 `Provider`、`ChatClient`、`ChatBackend` 面向 runtime 的模型抽象：

```rust
#[async_trait]
pub trait ModelExecutor: Send + Sync {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse>;

    async fn stream(
        &self,
        request: ModelRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ModelEvent>> + Send>>>;
}
```

职责包括：

- provider 路由；
- OpenAI / Anthropic 适配；
- retry；
- usage 归一化；
- context overflow 分类；
- stream event 标准化；
- inference gateway metadata。

`TurnEngine` 不应知道 provider-specific request、header 或 retry 细节。现有 `ChatClient` 可以先作为 `DefaultModelExecutor` 的内部实现。

### 6.2 ContextAssembler

现有 `ContextEngine` 已经是很好的基础，但应进一步缩小 Agent 对其内部机制的依赖：

```rust
#[async_trait]
pub trait ContextAssembler: Send + Sync {
    async fn prepare(
        &self,
        session: &SessionView,
        tools: &[ToolDefinition],
    ) -> Result<PreparedContext>;

    async fn handle_overflow(
        &self,
        input: OverflowInput,
    ) -> Result<OverflowOutcome>;
}
```

它负责：

- system prompt 和 workspace context；
- memory 注入；
- token estimation；
- proactive compaction；
- provider overflow 后的 compaction/retry；
- external session 和 context metadata。

### 6.3 ToolCatalog 与 ToolExecutor

当前 `ToolRegistry` 既提供定义，又负责执行。建议在 runtime 语义上拆成两个端口。

```rust
pub trait ToolCatalog: Send + Sync {
    fn definitions(&self, scope: &ToolScope) -> Vec<ToolDefinition>;
}

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute_batch(
        &self,
        calls: &[ToolCall],
        ctx: ToolExecutionContext,
    ) -> Result<Vec<ToolResult>>;
}
```

`DefaultToolExecutor` 内部负责：

- JSON/schema 参数校验；
- pre/post hooks；
- permission 前置判断；
- plan mode 限制；
- conflict-aware scheduler；
- tool 执行；
- output bounding；
- tool result 归一化；
- tool message 写回所需的结果。

`TurnEngine` 只需要把 tool calls 交给 executor，并消费结果，不需要知道 `Read`、`Bash`、`Edit` 如何分类。

### 6.4 PermissionService 与 ApprovalBroker

将本地策略判断和外部用户交互拆开：

```text
PermissionService
  ├── allow
  ├── deny
  └── ask
          └── ApprovalBroker
```

```rust
#[async_trait]
pub trait ApprovalBroker: Send + Sync {
    async fn request(
        &self,
        request: ApprovalRequest,
    ) -> Result<ApprovalDecision>;
}
```

实现可以包括：

- `TerminalApprovalBroker`；
- `TuiApprovalBroker`；
- `AcpApprovalBroker`；
- `CloudApprovalBroker`；
- 测试用 `AutoApprovalBroker`。

Core runtime 不应知道 Cloud approval database、Web UI 或 HTTP API。

### 6.5 EventSink

当前 `AgentEvent` 可以作为基础，但建议增加统一 envelope：

```rust
pub struct RuntimeEvent {
    pub sequence: u64,
    pub session_id: SessionId,
    pub turn_id: Option<TurnId>,
    pub step_id: Option<StepId>,
    pub kind: RuntimeEventKind,
}
```

`RuntimeEventKind` 可覆盖现有事件：

- turn start/end；
- step begin/end；
- text/thought delta；
- tool call/result；
- usage update；
- approval requested/resolved；
- mode/state changed；
- error；
- steer input。

不同产品只实现不同 sink：

```text
LocalEventSink     → CLI / TUI
AcpEventSink       → ACP session/update
CloudEventSink     → Cloud event API
RecordingEventSink → AgentRecordWriter
```

Core 只产生一种 runtime event，不直接构造 ACP update。

## 7. TurnEngine 设计

### 7.1 让 `zene-turn` 成为真正的状态机

当前 `zene-turn::run_turn_loop()` 是正确的拆分起点，但接口仍偏 callback。目标是让 `TurnEngine` 依赖 Session 和 Runtime Ports，而不是依赖具体 `Agent`：

```text
TurnEngine
  ├── ContextAssembler
  ├── ModelExecutor
  ├── ToolExecutor
  ├── EventSink
  └── RuntimePolicy
```

输入：

- `SessionView`；
- `TurnInput`；
- `RuntimePolicy`；
- cancellation/control stream。

输出：

- `TurnOutcome`；
- session mutations；
- usage；
- final text；
- optional checkpoint。

### 7.2 推荐状态流

```text
Idle
  → Preparing
  → CallingModel
  → ExecutingTools
  → AwaitingApproval
  → AwaitingUser
  → Completed / Failed / Cancelled
```

模型调用和工具调用应有清楚的边界：

```text
prepare context
  → model request
  → assistant message
  → tool batch
  → tool results
  → next step
```

### 7.3 Control plane

Runtime 应同时运行当前 engine future 和 control command 接收：

```text
select {
    engine step completes
    control command arrives
}
```

这样可以统一处理：

- `Cancel` 立即取消；
- `Steer` 注入 pending input；
- `Approval` 唤醒 permission wait；
- 新 prompt 进入显式队列；
- 状态变化实时发布。

## 8. Subagent 统一方案

当前主 Agent 和 Subagent 有两套循环：

```text
主 Agent   → zene_turn::run_turn_loop()
Subagent   → subagent.rs 自己维护 loop
```

目标是让二者共享同一个 `TurnEngine`，差异通过 `RuntimeScope` 和 policy 注入：

```rust
pub struct RuntimeScope {
    pub profile: AgentProfile,
    pub depth: u32,
    pub max_depth: u32,
    pub tool_policy: ToolPolicy,
    pub session_policy: SessionPolicy,
}
```

示例：

| Scope | 工具 | 持久化 | AskUser | Plan mode |
| --- | --- | --- | --- | --- |
| Full Agent | 全部配置工具 | durable | enabled | enabled |
| Explore | Read/Grep/Glob | ephemeral | disabled | disabled |
| Coder | Read/Edit/Write/Bash | ephemeral 或 child session | inherited | disabled |

现有 `SubagentRunner` 可以保留为兼容 API，但内部改为：

```text
SubagentRunner
  → RuntimeFactory
    → child RuntimeScope
      → shared TurnEngine
```

这样主 Agent 和 Subagent 将共享：

- max turns；
- cancellation；
- tool execution；
- context overflow；
- event semantics；
- model execution；
- 错误行为。

## 9. Session、Store 与 Checkpoint

### 9.1 分离内存状态和持久化

建议将当前 `SessionRecord` 的领域状态和文件持久化职责分开：

```text
SessionState
  ├── messages
  ├── metadata
  ├── todos
  ├── mode
  └── context metadata

SessionStore
  ├── load
  ├── append mutation
  └── snapshot
```

接口示例：

```rust
#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn load(&self, id: &SessionId) -> Result<SessionState>;
    async fn append(&self, mutation: SessionMutation) -> Result<()>;
    async fn snapshot(&self, state: &SessionState) -> Result<()>;
}
```

第一阶段可以继续使用现有文件 session store，只把接口边界先建立起来。

### 9.2 从 session resume 到 execution resume

当前 `session/load` 和 `session/resume` 主要恢复已经持久化的历史，不能从正在执行的 step 恢复。未来应在以下边界保存 checkpoint：

1. turn started；
2. context prepared；
3. model response received；
4. tool batch started；
5. 每个 tool 完成；
6. awaiting approval；
7. turn completed / failed。

Checkpoint 至少记录：

- session / turn / step id；
- execution state；
- message mutations；
- completed tool call ids；
- pending approval id；
- retry count；
- context epoch；
- model request hash。

工具执行需要 `execution_id`、`tool_call_id` 和幂等键，避免进程崩溃恢复时重复执行写操作。

第一阶段不要求完整 Event Sourcing，采用下面的最小方案即可：

```text
session snapshot
+ append-only execution record
+ tool-call idempotency
```

## 10. Cloud 与 ACP 的拆分

### 10.1 Cloud JobRunner

Cloud worker 应拆成两个概念：

`JobRunner` 只负责：

- claim；
- workspace；
- heartbeat；
- Cloud status；
- follow-up command；
- cancel command；
- commit / push / PR。

`RuntimeClient` 只负责：

- initialize；
- create/resume session；
- prompt；
- cancel；
- 接收 runtime events；
- 响应 approval。

目标调用链：

```text
JobRunner
  → RuntimeClient
    → AcpClient
      → zene acp process
```

`JobRunner` 不应解析 ACP 的 `session/update` 细节；这些应由 `AcpClient` 转换为统一 `RuntimeEvent`。

### 10.2 ACP Server

ACP server 应成为 adapter：

```text
ACP request
  → RuntimeCommand
  → AgentRuntime
  → RuntimeEvent
  → ACP notification
```

这样未来可以同时支持：

- in-process runtime；
- ACP process runtime；
- WebSocket runtime；
- gRPC runtime；
- test fake runtime。

这些 transport 共享相同的 command/event 语义。

## 11. 推荐 crate 结构

最终可演进为：

```text
crates/
├── turn/
│   ├── TurnState
│   ├── TurnEngine
│   └── TurnOutcome
├── runtime/
│   ├── AgentRuntime
│   ├── RuntimeHandle
│   ├── RuntimeCommand
│   ├── RuntimeEvent
│   ├── RuntimeScope
│   └── runtime ports
├── core/
│   ├── default runtime wiring
│   ├── default tools / permissions
│   ├── local session integration
│   └── compatibility Agent facade
├── llm/
│   └── ModelExecutor implementations
├── tools/
│   ├── ToolCatalog
│   ├── ToolExecutor
│   └── built-in tools
├── session/
│   ├── SessionState
│   ├── SessionStore
│   └── checkpoints
└── sandbox/
    └── workspace / terminal / filesystem
```

不建议一开始就拆出全部 crate。可以先在现有 `zene-turn` 和 `zene-core` 内建立边界，待接口稳定后再移动模块。

## 12. 渐进式迁移计划

### Phase 1：统一 ID、状态和事件

不改变执行流程，先增加：

- `SessionId`；
- `TurnId`；
- `StepId`；
- `ToolCallId`；
- `ExecutionState`；
- `RuntimeEvent` envelope；
- sequence number。

将现有 `AgentEvent` 包装为 `RuntimeEvent`，让 recording、ACP 和 Cloud 逐渐消费统一事件。

### Phase 2：抽出 ModelExecutor

从 `Agent::run_llm_step()` 拆分：

- context preparation；
- model invocation；
- streaming assembly；
- overflow retry。

先保留 `ChatClient`，让它成为默认 ModelExecutor 的内部实现；同时为测试提供 fake executor。

### Phase 3：抽出 DefaultToolExecutor

将 `Agent::run_tools()` 的以下逻辑移入独立 executor：

- scheduler；
- hooks；
- permission；
- plan mode；
- output bound；
- result normalization。

`Agent` 只负责组装依赖，不再遍历和执行每个 tool call。

### Phase 4：让 TurnEngine 依赖 Ports

将 `zene-turn` 从接收具体 `Agent` 改为接收：

- Session state/view；
- ModelExecutor；
- ContextAssembler；
- ToolExecutor；
- EventSink；
- Runtime policy。

这是完成执行层解耦的关键阶段。

### Phase 5：Subagent 复用 TurnEngine

逐步删除 `subagent.rs` 中的独立循环，改为通过 `RuntimeScope` 创建 child runtime。保留 `SubagentRunner` 作为兼容 facade。

### Phase 6：引入 Runtime actor / Handle

将 ACP 当前的 `Arc<AsyncMutex<Agent>>` 替换为 `RuntimeHandle`。ACP server 只发送 command、订阅 event，不直接访问 Agent 内部字段。

### Phase 7：增加 checkpoint 和幂等恢复

待 runtime 边界稳定后，再实现：

- approval checkpoint；
- tool-call idempotency；
- step recovery；
- Cloud worker crash recovery。

## 13. 非目标与约束

本设计明确不做以下事情：

1. 不立即重写所有 Agent 代码；
2. 不把所有职责塞进一个巨大的 `AgentRuntime` trait；
3. 不让 Cloud HTTP、approval database、Git PR 逻辑进入 Core；
4. 不让 ACP 成为唯一 runtime 运行方式；
5. 不在第一阶段引入完整 Event Sourcing；
6. 不为了抽象而破坏当前 CLI、ACP、Cloud 行为；
7. 不改变现有 Console IA 或 UI 视觉规范。

## 14. 验收标准

架构迁移完成后，应满足：

- Local CLI、ACP、Cloud 使用同一套 runtime command/event 语义；
- 主 Agent 和 Subagent 使用同一个 TurnEngine；
- Model、Context、Tool、Approval、Session 都可以注入 fake 实现测试；
- `Agent` 不再直接负责 ACP 格式转换；
- Cloud JobRunner 不再解析 Agent 内部 tool/stream 语义；
- 一个 runtime 只有一个状态所有者；
- cancel、steer、approval、follow-up 都进入统一 control plane；
- session resume 与 execution checkpoint 的边界明确；
- 工具恢复具备幂等策略；
- 现有 `cargo test --workspace --locked`、CLI ACP 测试和 Cloud worker 测试保持通过。

## 15. 最终判断

zene 当前已经抽象出了比较可靠的 `Agent Turn`，但还没有完全抽象出独立的 `Agent Runtime`。最值得优先落地的三件事是：

1. **主 Agent 和 Subagent 共用同一个 TurnEngine；**
2. **外部通过 RuntimeHandle / RuntimeCommand 操作，不直接持有 Agent；**
3. **明确拆开 Cloud Job、ACP Transport、Agent Runtime 三个生命周期。**

完成这三步后，zene 才能从“以 `Agent` 为中心的 coding agent”演进为可被 CLI、ACP、Cloud、Subagent 和测试环境共同复用的 Agent Runtime。

