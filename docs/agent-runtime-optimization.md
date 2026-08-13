# Agent Runtime 架构优化设计

> 状态：持续演进。Wave 0–12 把控制面/数据面的 **接口边界** 建起来了；Wave 13 起让默认执行路径真正走这些边界。
>
> **进度快照：2026-08-13，基线 `8ff37a6`（PR #78 已合并）。**
> 本文同时记录目标架构、已实现能力和剩余工作。
> Wave 16 当前工作是 JobRunner mock 路径走 `RuntimeClient` 产品类型；Steer/SetMode 与整型 crate 仍未做。
>
> 本文基于当前 zene runtime 实现，描述如何将 `Agent`、`Turn`、`Step`、`Session`、Cloud `Run` 和 ACP transport 拉开，并给出渐进式迁移方案。
>
> **与 Session / Context 优化的关系：** 本文主攻 **控制面**（谁在跑、怎么被控制、状态归谁）；
> [session-as-source-of-truth.md](./session-as-source-of-truth.md) 与
> [context-engine.md](./context-engine.md) 主攻 **数据面**（记什么、模型看见什么）。
> 两条线 **正交且必须合并落地**，不互相取代。合并后的 Wave 顺序见
> [§16](#16-merged-implementation-waves)。

## 1. 背景与目标

当前 zene 已经具备可用的 coding-agent runtime：

- `zene-turn` 抽象了多步 `LLM → Tool → LLM` 循环；
- `zene-core::Agent` 持有 session、context、tools、sandbox、permission、hooks、MCP 等能力；
- `zene-llm` 统一了 OpenAI-compatible / Anthropic 的模型请求；
- `zene-tools`、`zene-sandbox` 和 `zene-permission` 提供了工具、执行环境和权限边界；
- ACP server 将 `Agent` 暴露给编辑器和 Cloud worker；
- Cloud worker 负责 workspace、worker、approval、heartbeat、commit/PR 等 Job 生命周期。

但当前 `Agent` 仍同时是 composition root 和 step orchestrator，开放度还不够：

1. `TurnEngine` 已统一主 Agent 和 Subagent 的循环，ports 也已抽出；Wave 13 起默认路径必须真正把 `prepare_context` 的 `PreparedContext` 交给 `run_model`，而不是在 model 步骤里重新组装上下文。
2. `Provider`、`ChatClient`、`ChatBackend` 仍存在相近但不统一的模型抽象；`zene-model-executor` 已可注入，但请求类型仍是 `zene-llm::ChatRequest`，Turn 还看不到中性 `ModelRequest`。
3. ACP、Cloud event 和 Core `AgentEvent` 仍存在语义转换。Cloud `RuntimeCommand::Approval` 已与 core 同形（`request_id` + `ApprovalDecision`），但 Cloud 仍无 Steer/SetMode 等变体，也未依赖 `zene-runtime`。已分类 Cloud payload 已是产品字段；未识别帧仍为 ACP JSON。
4. Session 可以恢复历史并在安全 model-boundary 上自动恢复未完成 execution；pending tool、approval 和 failure 仍必须 inspection/manual intervention。
5. `RuntimeHandle` 已成为 active turn、prompt queue、cancel 的控制所有者，但 Agent-specific actor 仍在 `zene-core`，ACP 仍保留 transport 层 session bookkeeping。
6. 权限已拆成纯 `evaluate` + 异步 `ApprovalBroker`。ACP 监听 `ApprovalRequested` 再发 `RuntimeCommand::Approval`。Cloud JobRunner 经 `RuntimeClient::send(RuntimeCommand::Approval)` 回复；ACP jsonrpc id 与 option 列表只留在 adapter。Cloud 产品审批类型不再携带 `jsonrpc_id`。存库/API/Console/RuntimeCommand 共用 domain `ApprovalDecision`，并带 `ApprovalKind` / `ApprovalRisk`。已分类 Cloud payload 与审批表 payload 已是产品字段；未识别帧仍为 ACP JSON。API→worker `WorkerCommand.kind` 是 `Prompt` / `Cancel` 枚举。
7. `ToolRegistry` 仍同时承担目录与执行；`RuntimeScope` 尚未成为 Subagent 的正式差异注入面。

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
        │ RuntimeCommand / RuntimeEvent
        ▼
  RuntimeHandle (actor, 仍在 zene-core)
        │ owns Agent + prompt queue + cancel
        ▼
  zene_core::Agent   ← composition root；step 编排正从 Agent 迁出
        │
        ▼
  zene_turn::TurnEngine
        │
        ├── ContextAssemblerPort  → ContextEngine.prepare_step → PreparedContext
        ├── ModelExecutorPort     → ModelExecutor.complete/stream(PreparedContext)
        ├── ToolExecutorPort      → DefaultToolExecutor
        ├── EventSinkPort
        ├── Permission / Sandbox / Hooks / MCP
        └── SessionRecord + SessionStore
```

Cloud 场景额外经过一层进程边界：

```text
Cloud API
   │
   ▼
zene-cloud-worker  (JobRunner)
   │  claim / workspace / heartbeat / approval / cancel
   ▼
zene-cloud-runtime-client  (send RuntimeCommand；已分类事件为产品 payload；未识别帧仍是 ACP JSON)
   │
   ▼
zene acp
   │
   ▼
RuntimeHandle → Agent → TurnEngine
```

诚实约束：ports 已经是默认 Agent / Subagent 路径的入口；`LegacyTurnPorts` 只留给旧 `TurnRuntime` 适配。`Agent` 仍持有全部具体能力，审批和 Cloud 事件语义尚未成为可替换 port。

### 2.2 当前核心职责位置

| 能力 | 当前实现 | 说明 |
| --- | --- | --- |
| Turn 循环 | `crates/turn/src/turn_loop.rs` | 通用多步循环，已经是较好的抽象 |
| Turn 适配 | `crates/core/src/agent_turn.rs` | `AgentTurnPorts` 直接实现 ports；`prepare_context` 产出 `PreparedContext`，`run_model` 消费它 |
| Agent 状态 | `crates/core/src/lib.rs` | session、context、tools、permission 等仍集中在 `Agent`；目标是退回 wiring |
| LLM 请求 | `crates/model-executor` + `crates/llm` | 可注入 `Arc<dyn ModelExecutor>`；请求类型仍是 `ChatRequest` |
| Tool 执行 | `crates/core/src/tool_executor.rs` + `crates/tools` | `DefaultToolExecutor` 已抽出；`ToolRegistry` 仍是目录+执行一体 |
| Tool 并发 | `crates/core/src/tool_scheduler.rs` | 同一模型 step 内进行冲突感知并发 |
| Session | `crates/session` | `SessionRecord` + `SessionStore`；events 优先，messages 为 materialized cache |
| Context | `crates/context` | observe/commit/project；`SessionView` 驱动 event-backed projection |
| Permission | `crates/permission` | `evaluate` 纯判定；`Ask` 走 `ApprovalBroker`。Runtime 挂 waiter，transport 发 `RuntimeCommand::Approval` |
| Subagent | `crates/core/src/subagent.rs` | 直接实现 `TurnEnginePorts`；仍用独立 `ChatBackend` 和内存消息，尚未 `RuntimeScope` |
| Runtime | `crates/runtime` + `crates/core/src/agent_runtime.rs` | 公共 command/event 在 `zene-runtime`；Agent actor 仍在 core |
| ACP | `apps/cli/src/acp/server.rs` | transport adapter；创建/加载 session，并把请求接入 `RuntimeHandle` |
| Cloud Job | `cloud/apps/worker` + `cloud/crates/runtime-client` | Job 经 `RuntimeClient::send` 发 Prompt/Cancel/Approval/Shutdown；API→worker `WorkerCommand` 为 Prompt/Cancel；审批产品面（决策/kind/risk）从 DB 到 Console 到 RuntimeCommand 共用 domain 类型；ACP `optionId` 只在 adapter 内映射 |

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

**对外** 这是 TurnEngine 看到的 port；**对内** 实现应对齐投影三段式
（见 [context-engine.md](./context-engine.md)）：

```text
observe  — 只读 SessionView，估算 token / water，提出 recommended actions
commit   — 唯一允许写 Conversation SoT 的入口（如 CompactionApplied、memory flush）
project  — 纯函数（或只读 cache）→ PreparedContext { messages, metadata, explain }
```

纪律：

- `prepare` / `project` **不得** 隐式物理删除历史当唯一真相；
- compaction 经 `commit` 追加事实后，再 `project`；
- `PreparedContext` 宜带可选 `ProjectionExplain`（debug / ACP `_meta` / Console）。

兼容期可继续暴露 `ContextEngine::prepare_step` 门面，内部切换为三段式。

它负责：

- system prompt 和 workspace context；
- memory 注入；
- token estimation；
- proactive compaction（commit 侧）；
- provider overflow 后的 compaction/retry；
- external session 和 context metadata；
- 每步装饰（todos / bg / reminder）与 epoch 规则。

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
- `RuntimeOwnedBroker`（actor 持有 waiter，transport 发 `RuntimeCommand::Approval`）；
- 测试用 `AutoApprovalBroker`。

ACP 不再注入专用 broker：它监听 `ApprovalRequested`，把 `session/request_permission` 的结果转成 `RuntimeCommand::Approval`。Cloud JobRunner 用同一套 `ApprovalDecision` 回复；ACP JSON 只留在 RuntimeClient adapter。

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
  ├── events / mutations   ← conversation SoT（目标）
  ├── messages cache       ← materialized，可重建
  ├── metadata / todos / mode
  └── context metadata（epoch、segment 指针等）

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

`SessionMutation` / `SessionEvent` 应能表达至少：message、tool call/result、
`CompactionApplied`、checkpoint marker、model change、branch/fork/rewind。
完整心智模型见 [session-as-source-of-truth.md](./session-as-source-of-truth.md)；
compaction 不得只靠「物理删除 `messages` 前缀」充当唯一真相。

### 9.2 三类记录，勿挤成一种

| 名称 | 职责 | 例子 |
| --- | --- | --- |
| **Conversation SoT**（`SessionMutation` / events） | 对话与上下文事实 | message、compaction、fork |
| **Execution record** | 运行进度与恢复 | step 边界、tool 完成、pending approval |
| **RuntimeEvent** | 对外实时流 | text delta、tool call UI、usage |

- **ID 空间统一**（`SessionId` / `TurnId` / `StepId` / `ToolCallId`）
- **不必** 合成一个万能 enum 打天下
- RuntimeEvent 可由前两者 **投影** 而出（带 `sequence`）

### 9.3 从 session resume 到 execution resume

当前 `session/load` 和 `session/resume` 主要恢复已经持久化的历史，不能从正在执行的 step 恢复。未来应在以下边界保存 **execution** checkpoint：

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
- message mutations（或指向 Conversation SoT 的 offset）；
- completed tool call ids；
- pending approval id；
- retry count；
- context epoch；
- model request hash。

工具执行需要 `execution_id`、`tool_call_id` 和幂等键，避免进程崩溃恢复时重复执行写操作。

**CompactionApplied**（内容投影边界）与 **execution checkpoint**（崩溃恢复边界）生命周期不同，
可共享 `context_epoch` / `step_id`，但不要用一个文件糊两职。

第一阶段不要求完整 Event Sourcing，采用下面的最小方案即可：

```text
session snapshot
+ append-only conversation events（双写起步）
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
7. 不改变现有 Console IA 或 UI 视觉规范；
8. 不为「Runtime 干净」而继续把 `messages[]` 当唯一可毁历史；
9. 不照搬外部 harness 的「砍 MCP / 砍 permission」产品哲学
   （见 [pi-agent-harness-lessons.md](./pi-agent-harness-lessons.md)）。

## 14. 验收标准

### 14.1 Runtime / 控制面

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

### 14.2 Session / Context 数据面（与姊妹文档共用）

- Conversation SoT 双写后，旧 session 仍可 load；
- compact 产生可查询的 `CompactionApplied`（或等价 mutation），并可解释；
- `project(events)` 与现网 materialized `messages` 有金丝雀等价测试（flag 前）；
- fork / rewind 后两边上下文可解释、互不污染；
- `PreparedContext` / `ProjectionExplain` 可经 RuntimeEvent 或 debug 通道观测；
- Tool batch 的 block / error / terminate / cancel 语义在 `ToolExecutor` 层可测。

场景表与 Phase 细节见
[context-engine.md](./context-engine.md)、
[session-as-source-of-truth.md](./session-as-source-of-truth.md)。

## 15. 当前进度与最终判断

### 15.1 已完成的基础能力

截至 2026-08-13，以下能力已经合并到 `main`，并在当前分支上让默认 Turn 路径开始真正消费 ports：

| 能力 | 实现位置 | 状态 |
| --- | --- | --- |
| 统一 `SessionId` / `TurnId` / `StepId` / `ToolCallId` | `crates/turn` | 已完成 |
| `RuntimeEvent` envelope、sequence 和 scope | `crates/turn`, `crates/core` | 已完成 |
| 旧 `AgentEvent` → `RuntimeEvent` 兼容适配 | `crates/core/src/events.rs` | 已完成 |
| `SessionEvent` 双写基础设施 | `crates/session` | 已完成（过渡态） |
| Context `observe → commit → project` | `crates/context/src/engine.rs` | 已完成（仍以 materialized messages 为输入） |
| `ProjectionExplain` + RuntimeEvent / ACP projection update | `crates/context`, `crates/core`, `apps/cli/src/acp` | 已完成基础观测链路 |
| 独立 `DefaultToolExecutor` | `crates/core/src/tool_executor.rs` | 已完成 |
| `ToolBatchOutcome::Terminate` | `crates/turn`, `crates/core` | 已完成 |
| `TurnEngine` ports | `crates/turn` | 已完成；默认路径必须消费 `PreparedContext` |
| `RuntimeHandle` / `RuntimeCommand` actor | `crates/runtime`, `crates/core/src/agent_runtime.rs` | 协议在 runtime crate；Agent actor 仍位于 core |
| ACP 通过 Runtime control plane 工作 | `apps/cli/src/acp` | 已完成 |
| Execution checkpoint、幂等 key、恢复快照 | `crates/session`, `crates/core` | 已完成（评估能力） |
| ACP recovery metadata | `apps/cli/src/acp/server.rs` | 已完成，默认不自动 resume |
| RecoveryPlan 安全门控、rewind execution boundary | `crates/session`, `crates/core`, `apps/cli/src/acp` | 已完成（禁止不安全自动恢复） |
| 主 Agent 与 Subagent 共用 `TurnEngine` | `crates/core/src/subagent.rs` | 已完成 |
| 可注入 `SessionStore` / 默认 `FileSessionStore` | `crates/session`, `crates/core` | 已完成 |

已合并的主要 PR：

- PR #54：Tool executor termination contract
- PR #55：TurnEngine capability ports
- PR #56：Runtime control plane
- PR #57：Execution recovery checkpoints and ACP recovery metadata
- PR #58：Subagent TurnEngine unification and SessionStore injection
- PR #60：TurnEngine default path consumes PreparedContext

### 15.2 当前判断（2026-08-13）

Wave 0–12 的价值是把 **所有权和语义** 分开：Runtime 控制面、Conversation SoT、Context 投影、ModelExecutor、Cloud RuntimeClient 都有了明确边界。这是正确的地基，但还不是开放、解耦、可替换的 Agent Infra。

真正的缺口不在 crate 数量，而在默认执行路径和产品面仍耦合在具体对象上：

1. **Turn ports 已是真入口（Wave 13，PR #60）**：`AgentTurnPorts.prepare_context` 产出 `PreparedContext`，`run_model` 只消费它；Subagent 直接实现 `TurnEnginePorts`。
2. **`Agent` 仍是 God Object**：它同时持有 model、tools、sandbox、permission、hooks、MCP、todos、plan mode。下一步是把它变成 wiring，而不是继续实现 step。
3. **模型抽象仍偏 provider**：`ModelExecutor` 吃 `ChatRequest`；Subagent 另有 `ChatBackend`。目标是 Turn 只看见 `PreparedContext` → `ModelRequest`。
4. **审批 waiter 已打通（Wave 15）**：`PermissionGate::evaluate` 只做 allow/deny/ask；`Ask` 交给 `ApprovalBroker`。Runtime actor 持有 oneshot waiter，`RuntimeCommand::Approval` 唤醒 in-flight tool。ACP 只做事件适配。
5. **Cloud 审批 command 已中性化（Wave 16）**：JobRunner 用 `send(RuntimeCommand::Approval { request_id, decision })` 回复审批。ACP jsonrpc id 与 `optionId` 只留在 adapter。Cloud 产品审批类型不再携带 `jsonrpc_id`。API→worker `WorkerCommand.kind` 是 Prompt/Cancel 枚举。存库/API/Console/RuntimeCommand 共用 domain `ApprovalDecision` / `ApprovalKind` / `ApprovalRisk`。Cloud 仍不是 `zene-runtime::RuntimeCommand`（无 Steer/SetMode，不引入该 crate）。
6. **Cloud 事件产品面已接到 Console（Wave 16）**：domain `CloudEventKind` 是 RuntimeClient 分类结果；已分类 payload 存产品字段。Console 时间线按 `event_type` 渲染 `text_delta` / `thought_delta` / `user_message` / `tool_call` / `tool_result`（含 legacy `params.update` 回放）。plan/usage/projection 等不进时间线。未识别帧仍为 `acp`。
7. **JobRunner 只看见 RuntimeClient 产品类型（Wave 16）**：mock 与真实 ACP 共用 event pump；`RuntimeNotification.event_type` 是 `CloudEventKind`；`RuntimeRequest::Approval` 携带 `ApprovalKind`。ACP `MockMsg` / `optionId` 只留在 adapter。
8. **不要再为干净拆 crate**：在审批 waiter 和事件语义统一之前，把 actor 搬到新 crate 只会搬耦合。

Conversation event 与 materialized `messages` cache 的双轨可以保留，直到 legacy session 可证明无损迁移。不要为了架构干净上完整 Event Sourcing。

### 15.3 完成标准

只有在以下条件满足后，才可称为本设计的主要目标完成：

- Local CLI、ACP、Cloud 使用同一套 runtime command/event 语义；
- 主 Agent 和 Subagent 使用同一个 `TurnEngine`；
- Model、Context、Tool、Approval、Session 都可注入 fake 实现并独立测试；
- `Agent` 不负责 ACP 格式转换，Cloud JobRunner 不解析 ACP 内部事件；
- 一个 runtime 只有一个状态所有者；
- Conversation facts 可从 append-only events 重建，`messages` 降为 materialized cache；
- compaction 追加事实并保留可解释历史，不以物理删除消息作为唯一真相；
- context projection 能从 active event path 生成，并可说明本步输入；
- execution recovery 具备明确的 resume / inspect / manual 策略和工具幂等保护；
- fork、rewind、compact、resume 的行为有跨层集成测试。

## 16. Merged implementation waves

本节是 **Runtime（本文 §12）** 与 **Session/Context 投影** 的合并排期，避免两拨人互拆。
细节仍以各专项文档为准；这里只定 **顺序与依赖**。

```text
Wave 0   契约对齐（文档 / 术语）
         Runtime · Turn · Step · SessionMutation · PreparedContext · RuntimeEvent

Wave 1   统一身份与事件信封                    ← 本文 Phase 1
         SessionId / TurnId / StepId / ToolCallId
         RuntimeEvent { sequence, ids, kind }
         现有 AgentEvent 包装为 RuntimeEvent（行为不变）

Wave 2   Conversation SoT 双写                 ← session-as-source-of-truth
         SessionEvent：至少 Message + CompactionApplied
         与 Record / RuntimeEvent 共享 ID；messages 仍为 cache
         读路径暂不切换

Wave 3   ContextAssembler 对齐投影             ← context-engine
         prepare_step 内 observe → commit → project
         对外可仍叫 prepare_step / ContextEngine
         对 Turn 只暴露 prepare / handle_overflow

Wave 4   ToolExecutor + terminate 契约         ← 本文 Phase 3 + tool 协议
         移出 Agent::run_tools；顺序 / block / terminate 可测

Wave 5   TurnEngine 只依赖 ports               ← 本文 Phase 4
         SessionView + PreparedContext 已相对稳定后再做

Wave 6   RuntimeHandle 控制面                  ← 本文 Phase 6
         ACP / Cloud 只发 Command、订 Event
         steer / cancel / approval 单所有者

Wave 7   Execution checkpoint / 幂等           ← 本文 Phase 7
         建立在 Wave 1 ID + Wave 2/4 工具完成事实上

Wave 8   Subagent = RuntimeScope               ← 本文 Phase 5（可与 5–6 交叉）

Wave 9   Conversation facts 完整化
         tool call/result、permission、model change、branch/fork/rewind
         execution record 与 conversation event 的关联

Wave 10  Event-backed Context projection
         active event path → SessionView → observe/commit/project
         messages 降级为可重建 materialized cache

Wave 11  ModelExecutor 与 Runtime crate 边界
         抽离 ChatClient/stream/overflow；RuntimeHandle 移出 core

Wave 12  Execution resume 与 Cloud RuntimeClient
         safe resume、approval/tool inspection、JobRunner/RuntimeClient 解耦

Wave 13  默认路径真正走 ports
         AgentTurnPorts.prepare_context 产出 PreparedContext
         run_model 只消费该上下文
         Subagent 直接实现 TurnEnginePorts
         LegacyTurnPorts 不再是默认入口

Wave 14  RuntimeScope 与能力注入
         Subagent/Explore/Coder 通过 scope+policy 配同一套 ports
         ToolCatalog 与 ToolExecutor 语义拆分
         Agent 退回 composition root / 兼容 facade

Wave 15  ApprovalBroker
         PermissionService（纯判定）与异步 ApprovalBroker 拆开
         Runtime-owned waiter：Approval command 唤醒 in-flight tool

Wave 16  统一 transport command/event  ← 当前工作
         Cloud 审批 command 使用 ApprovalDecision
         Cloud event_type 使用中性 kind
         Console 按 event_type 分发时间线
         Cloud 时间线 payload 使用产品字段
         Cloud RuntimeCommand::Approval 与 core 同形
         Cloud 控制事件 payload 使用产品字段
         Cloud usage/state payload 使用产品字段
         Cloud projection/plan payload 使用产品字段
         Cloud 审批表 payload 使用产品字段
         API→worker WorkerCommand kind 使用 Prompt/Cancel 枚举
         Cloud 存库/API 审批决策使用 ApprovalDecision 枚举
         Cloud 产品审批类型不再携带 jsonrpc_id
         Cloud 审批产品面（决策/kind/risk）从 Console 到 RuntimeCommand 收口
         Console 时间线按 event_type 消费产品字段（含 user_message）
         JobRunner mock 路径走 RuntimeClient 产品类型（本轮）
         Cloud payload 不再以 ACP JSON 为产品语义
         本地与 Cloud 共用同一套 RuntimeCommand / RuntimeEvent
```

### Wave 0–8 完成状态

| Wave | 当前状态 | 说明 |
| --- | --- | --- |
| Wave 0 | 已完成 | 术语、ID、Turn/Step/Session/RuntimeEvent 契约已对齐 |
| Wave 1 | 已完成 | 统一 ID、RuntimeEvent envelope、兼容适配 |
| Wave 2 | 已完成（兼容过渡态） | SessionEvent 双写、事件优先 projection、cache drift 诊断和 legacy fallback 已落地 |
| Wave 3 | 已完成（兼容过渡态） | observe/commit/project 已拆分，Context 模型路径使用 strict event-backed projection |
| Wave 4 | 已完成 | DefaultToolExecutor、scheduler、terminate 语义已抽取 |
| Wave 5 | 已完成（默认路径收口） | TurnEngine ports 已抽出；Agent/Subagent 默认走 direct ports，`LegacyTurnPorts` 仅兼容旧 `TurnRuntime` |
| Wave 6 | 已完成（Agent actor 仍在 core） | RuntimeHandle、RuntimeCommandRouter 和 ACP control plane 已落地 |
| Wave 7 | 已完成（保守恢复策略） | checkpoint、幂等 fence、fsync append、resume-claim recovery visibility 和安全 model-boundary resume 已落地 |
| Wave 8 | 已完成 | Subagent 复用 TurnEngine，SessionStore 可注入 |
| Wave 9 | 已完成（legacy fallback 保留） | Conversation facts、tool/permission/model/branch/rewind 事件、execution links、active-path projection 和等价测试已落地 |
| Wave 10 | 已完成（当前设计范围） | strict projection、compaction event-backed planning、ProjectionExplain、cache drift/fallback provenance 已落地 |
| Wave 11 | 已完成第一阶段 | ModelExecutor、ContextModel、usage boundary、runtime protocol 和 lifecycle publisher 已落地；Agent-specific actor 尚在 core |
| Wave 12 | 已完成第一阶段 | safe resume、Cloud RuntimeClient、neutral runtime notifications、fenced command lease/ack、atomic state/event writes、outbox replay 和真实 replacement 测试已落地 |
| Wave 13 | 已完成 | 默认 Agent/Subagent 路径消费 `PreparedContext`；PR #60 |
| Wave 14 | 未开始 | RuntimeScope、ToolCatalog 拆分、Agent 退回 wiring |
| Wave 15 | 已完成 | `evaluate` + `ApprovalBroker` + runtime-owned waiter；PR #61 / #62 |
| Wave 16 | 进行中 | JobRunner mock/real 共用 RuntimeClient；Steer/SetMode 与整型 crate 仍待统一 |

### 当前收口状态与剩余边界

本轮已完成仓库内可以安全验证的主要优化，并保持旧协议兼容。当前剩余项分为三类：

- **本轮已完成的审批/事件收口**：runtime-owned waiter 与 Cloud `RuntimeCommand::Approval`；已分类 Cloud payload 与审批表 payload 已是产品字段；API→worker `WorkerCommand` 为 Prompt/Cancel；Cloud 审批产品面（决策/kind/risk）从 DB 到 Console 到 RuntimeCommand 共用 domain 类型；产品审批类型不再携带 `jsonrpc_id`；domain `CloudEventKind` 与 Console 时间线共用分类结果；JobRunner mock 与真实 ACP 都只看见 `RuntimeClient` 产品类型；未识别帧仍为 ACP JSON。
- **可继续做但需要协议的产品面解耦**：本地/Cloud 统一 `RuntimeCommand`/`RuntimeEvent`、`RuntimeScope`。这些改动跨 transport，不应通过局部兼容代码伪装完成。
- **需要部署基础设施决策**：本地 EventOutbox 不能单独提供跨 VM durability。跨 VM replacement 必须使用共享 POSIX 持久卷，或实现 DB/object-backed spool。

本轮已落地的 durability 收口包括：

1. **Conversation / Context SoT**
   - compaction planning、system-prefix memory injection 和 context report 使用 event-backed projection；materialized `messages` 继续作为兼容 cache。
   - legacy compaction/rewind/incomplete event log 保留显式 fallback reason，strict model-facing paths 拒绝不完整 fallback。
   - fork、rewind、compaction、reload、cache drift 和 system-prefix 更新均有回归测试。

2. **Execution recovery / tool safety**
   - execution record append 使用 per-path serialization 和 `sync_all`；重复 checkpoint/link append 幂等。
   - safe-resume claim 在没有后续 terminal checkpoint 时保持 recovery-visible，并分类为 manual intervention，而不是静默 Clean。
   - `ToolStarted` 必须在工具执行前成功持久化，`ToolCompleted` 在执行后持久化失败会使 turn 失败；UI event sink 仍保持 best effort。
   - worker shutdown、cancellation、runtime interruption 不再被当作 Cloud run Completed。

3. **Cloud control / replay durability**
   - approval create/decide 使用数据库并发权威，避免重复 approval/event；无 toolCallId 的 approval identity 使用确定性 key。
   - follow-up command 使用 attempt-fenced lease/ack，只有 runtime prompt 成功后才 ACK；过期 `waiting_for_user` attempt 可重新排队，approval hold 保持人工处理边界。
   - run state mutation 与对应 run event 在同一 DB transaction；event cursor migration 可从部分 DDL 状态安全重试。
   - RuntimeClient 对 worker 暴露 neutral notification/approval 类型，ACP method/path 只留在 adapter 内部。
   - Runtime lifecycle 统一更新 state 和 ordered terminal event；Cloud outbox/API/SSE/Last-Event-ID/replacement 测试全部通过。
   - worker title mutation 已接入 attempt fence 和确定性 event key；stale worker 不能覆盖 replacement worker 的 title。push/PR 的完整 provider-side fencing 仍需 Git provider 幂等协议。

4. **仍明确未自动完成的项目**
   - pending tool / approval 的任意副作用自动 replay：继续采用 inspection/manual intervention，避免重复写操作。
   - `Agent` 从 step orchestrator 完全退回 composition root。
   - Subagent 通过 `RuntimeScope` 复用 `DefaultToolExecutor` / ModelExecutor，而不是并行 `ChatBackend`。
   - 本地与 Cloud 共用同一套 `RuntimeCommand` / `RuntimeEvent`（Cloud 已有 Prompt/Cancel/Approval/Shutdown；API→worker 已是 Prompt/Cancel；审批产品面已收口；仍缺 Steer/SetMode，且不依赖 `zene-runtime`）。未识别 Cloud 帧仍为 ACP JSON。
   - Agent-specific actor 从 `zene-core` 移入独立 runtime implementation crate（应在 ports/审批稳定之后）。
   - 跨 VM outbox 的共享持久化实现；当前部署文档要求共享 POSIX volume 或后续 DB/object spool。

以上未完成项不是本轮测试覆盖的小修复；在没有明确协议、存储和迁移策略前，不应声称已经完成。

### 已完成的 Wave 9–12 细节

1. **Wave 9：完整 Conversation Event Log（兼容过渡态，已完成当前切片）**
   - 已完成：扩展 `SessionEvent` 覆盖 tool call/result、permission、mode/model change、branch/rewind，并加入 monotonic sequence；
   - 已完成：turn/step/tool/terminal checkpoint 路径写入 `execution_link`，将 execution idempotency key 关联到 Conversation Event 的 `id` / `sequence`，且旧 record JSONL 保持兼容；
   - 已完成：fork/rewind lineage、active-path projection、cache-free projection 和事件重建等价测试；
   - 继续保留旧 session load、messages materialized cache 和无法无损迁移数据的显式 fallback。

2. **Wave 10：Event-backed Context Projection（当前设计范围已完成）**
   - 已完成：`ContextSession::view` / `SessionView::from_events`，新 compaction/rewind 事件携带 projection snapshot；
   - 已完成：Context 的 observe/project 默认基于 event-backed view，`ProjectionExplain` 暴露 source event count、cache fallback 和 fallback reason；rewind target boundary 已用于 active path 过滤；
   - 已完成：fallback reason、active branch ID、active path start sequence、active event count 和结构化 tool/injected/retained-turn provenance 通过 RuntimeEvent 和 ACP `projection_update` 暴露；
   - 已完成：fork parent lineage metadata，以及 nested/sibling fork projection regression tests；
   - 已完成：compaction 后 SessionRecord 序列化 reload 与 event-backed projection 等价测试；
   - 已完成：完整事件日志优先于 materialized cache；cache drift 通过 `cacheDriftDetected` 诊断，不再覆盖 event-backed projection；空的新 session 不再误报 fallback；
   - 已完成：严格 event-backed `try_view` 入口、legacy fallback reason 测试，以及 tool/injected/retained-turn explain；后续仅在迁移数据可证明无损时继续收缩兼容代码。

3. **Wave 11：ModelExecutor 与 runtime crate（P1）**
   - 已完成第一切片：core 内部 `model_executor` seam 负责 stream tool-call delta 累积、ID 规范化和消息组装；
   - 已完成请求边界切片：`ModelExecutor` / `ChatClientExecutor` 接管 stream 与 non-stream 模型请求，并提供 fake executor 测试；
   - 已完成 stream assembly 切片：`StreamAccumulator` 负责文本、tool-call delta 和 usage 累积，Agent 只负责事件转发与终端输出；
   - 已完成 overflow retry 状态切片：`OverflowRetryState` 集中管理 truncate → summarize 的重试边界，`Agent::recover_overflow` 只编排 ContextEngine recovery 与 step 刷新，具体 compaction 仍由 ContextEngine 执行；
   - 已完成 usage 累积切片：`UsageAccumulator` 负责跨 step 的 turn usage 累积，兼容保留 `Agent::turn_usage()`；
   - 已完成 usage snapshot 与 Context water 计算切片：`UsageSnapshot` 封装 usage 事件只读字段，`ContextWaterLevel::usage_update` 统一 provider usage 与估算值的持久化取值；`ContextUsageUpdate` 作为 ContextEngine 到 runtime 的稳定返回值；
   - 已完成 usage 模块边界切片：`UsageAccumulator` / `UsageSnapshot` 从 `model_executor` 移入独立 usage 模块，模型执行器不再承担 token accounting；
   - 已完成请求 executor 共享持有关系：Agent 通过 `Arc<dyn ModelExecutor>` 复用 `ChatClientExecutor`，模型配置切换时同步替换 client/executor；
   - 已完成 Context 模型边界切片：`zene-context::ContextModel` 只暴露 `complete(ChatRequest)`，compaction、memory flush、prefire factory 和 `ContextDeps` 不再要求具体 `ChatClient`；
   - 已完成 Agent client 持有关系切片：Agent 仅保存 `Arc<dyn ContextModel>` 与 `Arc<dyn ModelExecutor>`，具体 `ChatClient` 只在 builder / model adapter 内部持有；
   - 已完成 Context water/usage 写回由 ContextEngine 统一计算，并对 event-backed projection 执行 strict 校验；
   - 已完成公共 runtime command/state/response、`RuntimeControl` 和 recovery info 迁移到独立 `zene-runtime` crate；`zene-runtime::RuntimeCommandRouter` 统一拥有 command channel、reply、event broadcast 与 state watch；`zene-core::RuntimeHandle` 继续保留 Agent-specific actor、recovery 和兼容 facade；
   - 已完成独立 `zene-model-executor` 的唯一实现收口，删除 core 内未使用的历史重复实现；
   - 已完成 runtime protocol、公共 control contract、Runtime lifecycle publisher 与 Agent actor wiring 的依赖隔离；Agent-specific actor 仍保留在 core，待 generic driver contract 稳定后再迁移。

4. **Wave 12：Execution resume 与 Cloud transport（P1）**
   - 已完成安全门控第一切片：`RecoveryPlan`、rewind execution boundary 和 ACP recovery metadata；
   - 已完成显式 safe model-boundary resume 第一切片；
   - 已完成安全启动恢复：ACP `session/resume` 自动触发 safe model-boundary resume；pending tool / approval 仍强制 inspection，禁止不安全 replay；
   - 已完成 Cloud JobRunner → RuntimeClient 第一层分离、attempt/generation fencing、ACP child failure propagation、按 attempt 持久化的 session resume、稳定 ACP event identity、run-level dedupe、canonical seq/cursor replay、Last-Event-ID、worker event retry、event pump failure propagation/flush barrier，以及本地 crash-safe durable event outbox；新增 API/SSE/replacement-worker 闭环测试，覆盖首条事件投递、worker replacement 后 source-event 重放去重、平台 status event 造成的 seq 间隙，以及 `Last-Event-ID` 后续只回放未消费事件；outbox 事件先经文件 `sync_all` 和目录同步后原子落盘，使用固定长度稳定 key 和原子 hard-link 占位，成功 POST 后删除，新 attempt 启动时按 source event ID 重放，并清理崩溃遗留临时文件；同时具备 10,000 事件/128 MiB 背压上限。EventOutbox 已内聚为 worker 内部模块，并使用跨进程 advisory lock 将容量检查与原子占位合并为一个临界区，避免 replacement worker 并发 enqueue 突破背压限制。RuntimeClient 还将 ACP reverse request 归一化为 `Permission` / `Unsupported` runtime request，worker 不再解析 ACP method/parameter 路径。
   - 已增加 provider cursor 之后混合 platform/runtime event 的 canonical seq replay 测试，覆盖 cursor 重连后继续接收无 cursor platform event；EventOutbox 已迁移到独立 worker 模块。
   - 已增加 worker restart/ACK 集成切片：第一 worker 持久化未发送事件后退出，replacement worker 重开同一 outbox，通过 HTTP 重放事件并在成功 ACK 后确认 outbox 清空；DB smoke 同时覆盖跨 attempt 的 run-level source-event 去重。
   - 已覆盖 outbox retryable HTTP failure（503 → retry → 200）和 non-retryable HTTP failure（400 保留事件）路径，并在 Cloud deploy 文档中明确本地 outbox 的跨 VM 限制、共享持久卷要求和清理策略。
   - 同一 durable filesystem 的真实跨 worker reconnect/replay、crash recovery 和 DB/API/SSE 集成测试已完成；跨 VM outbox 仍需共享 POSIX volume 或 DB/object-backed spool 的部署决策。

5. **Wave 13：默认路径真正走 ports（PR #60，已完成）**
   - `PreparedContext` 携带 messages、tools、metadata、estimate_tokens；TurnEngine 把 assembler 输出传给 `run_model`。
   - `AgentTurnPorts.prepare_context` 调用 ContextEngine 并记录 step checkpoint；`run_model` 只调用 `invoke_model`，不再忽略上下文。
   - Subagent 直接实现 `TurnEnginePorts`，`ChatBackend` 收到的 messages/tools 来自 `PreparedContext`。
   - `AgentBuilder.model_executor` 允许注入 fake executor，便于不经过真实 provider 测试 runtime。
   - `LegacyTurnPorts` / `run_turn_loop` 保留给旧 `TurnRuntime` 实现。

6. **Wave 15：ApprovalBroker（本地/ACP 已完成）**
   - `PermissionGate::evaluate` 纯 allow/deny/ask；`Ask` 才进入 broker。
   - `resolve_permission` 在 await 期间不持有 permission mutex。
   - `AutoApprovalBroker` / `TerminalApprovalBroker` 可注入；runtime 默认挂 `RuntimeOwnedBroker`。
   - ACP 监听 `ApprovalRequested`，`session/request_permission` 结束后发 `RuntimeCommand::Approval`。
   - Cloud JobRunner 用 `ApprovalDecision` 回复审批；ACP JSON 只在 RuntimeClient adapter 内构造。

7. **Wave 16：统一 command/event（进行中）**
   - JobRunner 经 `RuntimeClient::send` 发送 Prompt/Cancel/Approval/Shutdown；`Approval` 携带 `request_id` + `ApprovalDecision`。
   - ACP jsonrpc id 与 `optionId` 只留在 RuntimeClient adapter。审批表 payload 存产品字段，不再存 ACP params。Cloud 产品审批类型不再携带 `jsonrpc_id`；DB 列保留但不读写。
   - RuntimeClient 把 ACP `sessionUpdate` 分类为 domain `CloudEventKind`；已分类 payload 存产品字段。
   - Console 按 `event_type` 分发时间线：`text_delta` / `thought_delta` / `user_message` / `tool_call` / `tool_result` 直接渲染产品字段，legacy `params.update` 仍可回放。plan/usage/projection/session_started/approval_requested 不进时间线。
   - API→worker `WorkerCommand.kind` 是 `Prompt` / `Cancel` 枚举，wire JSON 仍为 `"prompt"` / `"cancel"`。不包含 Approval/Shutdown/Steer。
   - Cloud 存库/API/Console/RuntimeCommand 共用 domain `ApprovalDecision`；`ApprovalKind` / `ApprovalRisk` 同样是产品枚举。ACP `optionId` 仍只在 adapter 内映射到 `PermissionDecision`。
   - JobRunner mock 与真实 ACP 共用 event pump；只发送 `RuntimeCommand`，只消费 `RuntimeEvent` / `RuntimeRequest`。`RuntimeNotification.event_type` 是 `CloudEventKind`；审批请求携带 `ApprovalKind`。ACP `MockMsg` 与 `optionId` 只留在 adapter。
   - 未做：Cloud 补齐 Steer/SetMode 并与 `zene-runtime` 合成一套类型。

8. **持续质量门槛**
   - 每个 wave 保持 `cargo test --workspace --locked`；
   - 不破坏旧 ACP `AgentEvent` / RuntimeEvent 协议；
   - 不把 Cloud HTTP、Git、ACP JSON 格式引入 `zene-turn`；
   - 每个跨层迁移必须包含 legacy compatibility test 和 failure-path test。

**对应本文原 Phase 编号：**

| 本文 Phase | Merged Wave |
| --- | --- |
| Phase 1 ID/事件 | Wave 1 |
| Phase 2 ModelExecutor | 可嵌在 Wave 4–5 之前或并行（不挡 Wave 2–3） |
| Phase 3 ToolExecutor | Wave 4 |
| Phase 4 TurnEngine ports | Wave 5 |
| Phase 5 Subagent | Wave 8 |
| Phase 6 Runtime actor | Wave 6 |
| Phase 7 checkpoint | Wave 7 |
| （数据面）SoT / 投影 | Wave 2–3 |

**若只能做一件事：**

| 选择 | Wave | 理由 |
| --- | --- | --- |
| 当前最大杠杆 | **Wave 16 剩余 command** | JobRunner 已只看见 RuntimeClient 产品类型；Steer/SetMode 与整型 crate 仍分叉 |
| 结构清理 | Wave 14 | Agent 退回 wiring，应在审批/事件稳定之后 |

推荐组合：**JobRunner 已与 ACP 解耦**；下一步不要先补无调用方的 Steer/SetMode，也不要先搬 runtime crate。Wave 14 把 `Agent` 收成 wiring 仍应在 command 稳定之后。

**不要一上来做** actor 全量重写、完整 Event Sourcing、或再抽一层没有调用方的 crate。

**PR 切片建议：** 一 PR 只动一层；Core 继续当 composition root，接口稳后再搬 crate（与 §11 一致）。

## 17. 术语表（与姊妹文档共用）

| 术语 | 含义 |
| --- | --- |
| **AgentRuntime** | 长期存在的 session 执行器；命令入口、状态所有者 |
| **RuntimeHandle / RuntimeCommand** | 外部唯一控制 API |
| **RuntimeEvent** | 统一实时事件信封（多 sink 映射） |
| **TurnEngine** | 单次 turn 的 LLM↔tool 状态机 |
| **ContextAssembler** | 准备 `PreparedContext` 的 port（内含 observe/commit/project） |
| **PreparedContext** | 本步给模型的 messages + metadata + 可选 explain |
| **Conversation SoT** | 对话事实（SessionMutation / events） |
| **Execution record** | 运行进度（step/tool/approval） |
| **SessionView** | 只读会话视图，供 project / UI |
| **Cloud Run / Job** | 产品任务生命周期 ≠ Turn ≠ ACP session |
| **ACP** | Transport adapter，不是 core 执行抽象 |

## 相关文档

- [session-as-source-of-truth.md](./session-as-source-of-truth.md) — Session 事实 vs Context 投影
- [context-engine.md](./context-engine.md) — ContextEngine（投影、prefix cache、epoch/delta）
- [pi-agent-harness-lessons.md](./pi-agent-harness-lessons.md) — Pi Harness 对照
- [agent-components.md](./agent-components.md) — 可组装组件栈
- [ENGINE.md](./ENGINE.md) — turn / compaction 行为
- [decoupling-plan.md](./decoupling-plan.md) — crate 拆分历程

## 讨论记录

### 2026-08-11 — 并入 Session/Context 线

- 自 closed PR #50 恢复本文；标明控制面 vs 数据面正交
- §6.2 ContextAssembler 对齐 observe/commit/project
- §9 区分 Conversation SoT / Execution record / RuntimeEvent；compact vs execution checkpoint
- 新增 §16 Merged waves、§17 术语表；验收并入 fork/compact/explain

### 2026-08-13 — 默认路径必须走真实 ports

- 判断：Wave 0–12 完成的是边界，不是可插拔 runtime。`AgentTurnPorts.prepare_context` 曾是空实现，`run_model` 回调 `Agent::run_step` 内部重新组装上下文。
- 下一阶段优先级改为 Wave 13 → 15 → 16；暂缓再拆 crate。
- 本轮落地：`PreparedContext` 携带 messages/tools/metadata；Agent/Subagent 默认路径消费它；`AgentBuilder` 可注入 `ModelExecutor`。
- 明确非目标：完整 Event Sourcing、pending tool 自动 replay、把 God Object 一次性拆完。

### 2026-08-13 — ApprovalBroker

- 策略 `evaluate` 与异步 `ApprovalBroker` 拆开；`resolve_permission` 在 await 期间不持有 permission mutex。
- ACP 注入 `AcpApprovalBroker`，去掉 `spawn` + `std::thread` 同步等待。
- 未做：`RuntimeCommand::Approval` 唤醒 tool、`RuntimeEventKind::ApprovalRequested`、Cloud 共用同一 broker。

### 2026-08-13 — runtime-owned approval waiter

- `ApprovalWaiters` 挂在 runtime actor 上；`RuntimeOwnedBroker` 发 `ApprovalRequested` 并等待 oneshot。
- `RuntimeCommand::Approval` 在 active turn 中 `resolve` waiter；Cancel/Shutdown 会 `cancel_all`。
- ACP 不再注入 `AcpApprovalBroker`：事件循环看到 `ApprovalRequested` 后发 `session/request_permission`，再 `runtime.approve`。
- CLI 未开 waiter 时仍走旧同步 prompter。Cloud 审批回复已用中性 `ApprovalDecision`；`event_type` 已分类，Console 按 kind 分发；时间线 payload 已去 ACP JSON。
- 未做：pending tool 自动 replay、把 Agent actor 搬出 core、完整 Event Sourcing。

### 2026-08-13 — Cloud ApprovalDecision

- `zene-cloud-runtime-client::ApprovalDecision` 与 core 同为 AllowOnce / AllowSession / Deny。
- `RuntimeCommand::RespondApproval` 和 `RuntimeClient::respond_approval` 不再接受 ACP result JSON。
- ACP `optionId` 只在 adapter / `PermissionDecision::to_result` 内构造。
- 未做：Cloud 与 `zene-runtime::RuntimeCommand` 合成同一类型。

### 2026-08-13 — Cloud event_type

- RuntimeClient 按 ACP `sessionUpdate` / method 分类 `event_type`（`text_delta`、`tool_call` 等）。
- 未识别的帧仍为 `acp`。`payload` 保持原始 JSON，Console 回放不迁移。
- MockAgent 事件同样经 `RuntimeNotification::from_acp` 分类。

### 2026-08-13 — Console event_type 分发

- Console 时间线按 `event_type` 分发（`text_delta` → 助手文本，`tool_call` / `tool_result` 等）。
- legacy `acp` / `runtime` 仍读 `payload.params.update.sessionUpdate`。不迁移已存 payload。

### 2026-08-13 — Cloud 时间线 payload

- RuntimeClient 把 `text_delta` / `thought_delta` / `tool_call` / `tool_result` 存成产品字段（`text`、`toolCallId` 等），不再存 jsonrpc 信封。
- Console 优先读产品 payload；legacy `params.update` 仍可回放。不迁移已存记录。

### 2026-08-13 — Cloud RuntimeCommand::Approval

- Cloud `RuntimeCommand` 为 Prompt / Cancel / Approval / Shutdown。`Approval` 与 core 同为 `request_id` + `ApprovalDecision`。
- JobRunner 只 `send` 这些命令。ACP jsonrpc id 留在 adapter 的 pending map；不支持的 reverse request 在 adapter 内拒绝。
- 未把 `zene-runtime` 引入 Cloud。未做：Steer/SetMode。

### 2026-08-13 — Cloud 控制事件 payload

- `approval_requested` 存 `requestId` 与 toolCall 产品字段，不再存 jsonrpc 信封。
- `session_started` 存 `sessionId`（resume 带 `resumed: true`）。
- JobRunner 审批回复仍走 `RuntimeRequest` / `send(Approval)`，不读该 payload。不迁移已存记录。

### 2026-08-13 — Cloud usage/state payload

- `user_message` 存 `text`；`state_changed` 把 ACP `modeId` 收成 `state`；`usage_update` 把 `used`/`size` 与 `_meta` token 字段提到顶层。
- 不迁移已存记录。

### 2026-08-13 — Cloud projection/plan payload

- `projection_ready` 把 `_meta` 解释字段提到顶层；`plan` 存 `entries`；`available_commands` 存 `availableCommands`。
- 未识别帧仍为 `acp` 并保留原始 JSON。不迁移已存记录。

### 2026-08-13 — Cloud 审批表 payload

- `RuntimeRequest::Approval.context` 与 MockAgent 审批行改存产品字段（`requestId`、toolCall 字段），不再存 ACP params / `options.optionId`。
- Console 仍展示 payload JSON，无需改布局。不迁移已存审批行。
- 未做：Steer/SetMode、把 `zene-runtime` 引入 Cloud。

### 2026-08-13 — Cloud WorkerCommand kind

- API→worker `WorkerCommand.kind` 从字符串改为 `Prompt` / `Cancel` 枚举；serde `snake_case`，wire JSON 仍为 `"prompt"` / `"cancel"`。
- 不把 Approval/Shutdown/Steer 放进 `WorkerCommandKind`（那些不是 API→worker 命令）。
- 不把 `zene-runtime` 引入 Cloud。

### 2026-08-13 — Cloud 存库/API 审批决策

- Cloud domain `ApprovalDecision` 覆盖存库、Decide API 与 worker 创建审批；wire JSON 仍为 `allow-once` / `allow-always` / `reject-once`，并接受 `allow` / `deny` 别名。
- Console 仍发送这些字符串，不改布局。不把 `zene-runtime` 引入 Cloud，也不合并 runtime-client 的 `ApprovalDecision`。
- 不补 Steer/SetMode。

### 2026-08-13 — Cloud 产品审批去掉 jsonrpc_id

- `CreateApprovalRequest` / `ApprovalRequest` 不再携带 ACP `jsonrpc_id`。worker 创建审批不再传入该字段。
- 数据库列保留但不读写；不迁移、不删列。不改 Console 布局。
- 不补 Steer/SetMode。不把 `zene-runtime` 引入 Cloud。

### 2026-08-13 — Cloud 审批产品面

- domain `ApprovalDecision` 成为存库、Decide API、Console 与 `RuntimeCommand::Approval` 的同一类型；runtime-client 不再另定义一份。ACP `optionId` 经 `to_permission_decision` 留在 adapter。
- `ApprovalKind`（`permission` / `tool`）与 `ApprovalRisk`（`low` / `medium` / `high`）同样是产品枚举。
- Console 用同一套 wire 字符串类型，审批卡片布局不变。
- 不补 Steer/SetMode。不把 `zene-runtime` 引入 Cloud。

