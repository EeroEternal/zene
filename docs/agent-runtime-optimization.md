# Agent Runtime 架构优化设计

> 状态：持续演进（Wave 0–8 基础能力已落地；Wave 2/3/7 仍处于过渡态；Wave 9+ 进行中）
>
> **进度快照：2026-08-12，基线 `origin/main` = `38265cb`（PR #58 已合并）。**
> 本文同时记录目标架构、已实现能力和剩余工作；“已建立边界”不等于“最终架构已完成”。
>
> 本文基于当前 zene runtime 实现，描述如何将 `Agent`、`Turn`、`Step`、`Session`、Cloud `Run` 和 ACP transport 拉开，并给出渐进式迁移方案。
>
> **与 Session / Context 优化的关系：** 本文主攻 **控制面**（谁在跑、怎么被控制、状态归谁）；
> [session-as-source-of-truth.md](./session-as-source-of-truth.md) 与
> [context-engine-projection.md](./context-engine-projection.md) 主攻 **数据面**（记什么、模型看见什么）。
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

但当前 `Agent` 仍是一个较大的具体对象，且不同层次之间仍存在职责重叠：

1. `TurnEngine` 已统一主 Agent 和 Subagent 的循环，但 `Agent` 仍承担默认能力组装和兼容 facade 职责；
2. `Provider`、`ChatClient`、`ChatBackend` 仍存在相近但不统一的模型抽象，ModelExecutor 尚未独立落地；
3. ACP、Cloud event 和 Core `AgentEvent` 仍存在语义转换，RuntimeEvent 适配已建立但消费方尚未完全统一；
4. Session 可以恢复历史和评估未完成 execution，但不能自动恢复一个正在执行的 turn；
5. `RuntimeHandle` 已成为 active turn、prompt queue、cancel 的控制所有者，但 ACP 仍保留 transport 层请求/响应和 session bookkeeping；
6. Cloud `Run`、ACP session、Agent runtime 的跨进程生命周期边界仍需进一步显式化。

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
  RuntimeHandle (actor)
        │ owns Agent + prompt queue + cancel
        ▼
  zene_core::Agent
        │
        ▼
  zene_turn::TurnEngine
        │
        ├── ContextEngine (observe/commit/project)
        ├── ChatClient (ModelExecutor boundary pending)
        ├── DefaultToolExecutor
        ├── PermissionGate
        ├── Sandbox
        ├── Hooks / MCP
        └── SessionRecord + SessionStore
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
| Tool 执行 | `crates/core/src/tool_executor.rs` + `crates/tools` | `Agent` 负责组装 `DefaultToolExecutor`；权限、hook、plan、scheduler 和结果规范化已移出主要编排路径 |
| Tool 并发 | `crates/core/src/tool_scheduler.rs` | 同一模型 step 内进行冲突感知并发 |
| Session | `crates/session` | `SessionRecord` + `SessionStore`；`SessionEvent` 与 messages cache 双写，SoT 仍在迁移 |
| Context | `crates/context` | token 估算、compaction、overflow retry、observe/commit/project |
| Permission | `crates/permission` | 策略判断和 prompter |
| Subagent | `crates/core/src/subagent.rs` | 通过 `SubagentTurnRuntime` 复用 `zene-turn::TurnEngine`，保持 ephemeral scope |
| Runtime | `crates/core/src/runtime.rs` | `RuntimeHandle` actor、command、event、state、prompt queue 和 cancellation |
| ACP | `apps/cli/src/acp/server.rs` | transport adapter；创建/加载 session，并把请求接入 `RuntimeHandle` |
| Cloud Job | `cloud/apps/worker/src/main.rs` | claim、workspace、ACP 子进程、Cloud 状态和 Git 生命周期；RuntimeClient 分层仍待完成 |

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
（见 [context-engine-projection.md](./context-engine-projection.md)）：

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
[context-engine-projection.md](./context-engine-projection.md)、
[session-as-source-of-truth.md](./session-as-source-of-truth.md)。

## 15. 当前进度与最终判断

### 15.1 已完成的基础能力

截至 2026-08-12，以下能力已经合并到 `main`：

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
| `TurnEngine` ports | `crates/turn` | 已完成 |
| `RuntimeHandle` / `RuntimeCommand` actor | `crates/core/src/runtime.rs` | 已完成（仍位于 core） |
| ACP 通过 Runtime control plane 工作 | `apps/cli/src/acp` | 已完成 |
| Execution checkpoint、幂等 key、恢复快照 | `crates/session`, `crates/core` | 已完成（评估能力） |
| ACP recovery metadata | `apps/cli/src/acp/server.rs` | 已完成，默认不自动 resume |
| 主 Agent 与 Subagent 共用 `TurnEngine` | `crates/core/src/subagent.rs` | 已完成 |
| 可注入 `SessionStore` / 默认 `FileSessionStore` | `crates/session`, `crates/core` | 已完成 |

已合并的主要 PR：

- PR #54：Tool executor termination contract
- PR #55：TurnEngine capability ports
- PR #56：Runtime control plane
- PR #57：Execution recovery checkpoints and ACP recovery metadata
- PR #58：Subagent TurnEngine unification and SessionStore injection

### 15.2 当前仍未完成的目标

**Wave 9 第一切片已完成（当前工作区，尚未提交）：** `SessionEvent` 已覆盖 tool call/result、permission decision、mode/model change、branch fork、rewind；事件带 session-local monotonic sequence，并兼容旧事件加载；主 Agent tool executor 已将关键事件双写到 Conversation facts，同时继续写入旧 execution record / RuntimeEvent。

当前实现已经完成了控制面的大部分地基，但还不是最终的 `AgentRuntime + 可投影 Session` 架构：

1. **Conversation SoT 仍是过渡态**：`SessionEvent` 已覆盖 message、system prefix、compaction、tool call/result、permission、model change、branch/fork/rewind 等事实，但旧 session 仍需 materialized fallback。
2. **Context 事件投影已进入过渡实现**：`observe / commit / project` 已拆分，`SessionView` 已选择 active branch path 并驱动 Context 只读 projection；仍需继续移除旧 cache fallback，并补充更完整的 projection lineage / injected-item explain。
3. **ModelExecutor 尚未独立实现**：`Agent` 仍直接持有 `ChatClient` 并承担 context preparation、stream assembly、overflow retry 和 usage 处理；`TurnEngine` 的 port 已存在，但主 Agent 仍通过兼容适配层接入。
4. **Recovery 仍是只读评估**：`RecoveryDisposition` 能区分 safe resume、tool inspection 和 manual intervention，但还不会自动恢复未完成 turn、pending approval 或 crash 后的 tool step。
5. **Cloud JobRunner / RuntimeClient 尚未完全解耦**：目标中的 Cloud Job → RuntimeClient → ACP client 分层仍需落地，Cloud 不应解析 ACP 的底层 session/update 语义。
6. **Runtime 尚未拆成独立 crate**：`RuntimeHandle` 当前在 `zene-core`，`Agent` 仍是默认 wiring、运行状态和兼容 facade 的大型 composition root。
7. **投影可观测性仍可扩展**：`ProjectionExplain` 已通过 RuntimeEvent 和 ACP `projection_update` 暴露基础字段；`/context`、tool truncation/handle、注入项、保留 turn 等更细粒度信息仍需补齐。

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

Wave 3   ContextAssembler 对齐投影             ← context-engine-projection
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
```

### Wave 0–8 完成状态

| Wave | 当前状态 | 说明 |
| --- | --- | --- |
| Wave 0 | 已完成 | 术语、ID、Turn/Step/Session/RuntimeEvent 契约已对齐 |
| Wave 1 | 已完成 | 统一 ID、RuntimeEvent envelope、兼容适配 |
| Wave 2 | 基础完成 | SessionEvent 双写已存在；完整 Conversation SoT 未完成 |
| Wave 3 | 基础完成 | observe/commit/project 已存在；仍从 messages 投影 |
| Wave 4 | 已完成 | DefaultToolExecutor、scheduler、terminate 语义已抽取 |
| Wave 5 | 已完成 | TurnEngine ports 已抽取，Legacy adapter 保留兼容 |
| Wave 6 | 已完成 | RuntimeHandle actor 和 ACP control plane 已落地 |
| Wave 7 | 基础完成 | checkpoint、幂等和恢复 disposition 已落地；尚无自动 resume |
| Wave 8 | 已完成 | Subagent 复用 TurnEngine，SessionStore 可注入 |
| Wave 9 | 进行中（第一切片已完成） | Conversation events 已覆盖工具、权限、模型/模式、turn/step、checkpoint、branch/rewind；仍需补齐所有 execution 关联和重建金丝雀 |
| Wave 10 | 进行中（第一切片已完成） | `SessionView` 已从 active events 投影消息，Context 默认消费 projection 并提供 explain；仍需完全移除旧 cache fallback、补 active branch path |

### 接下来要完成的内容

建议按以下顺序推进，避免同时重写控制面和数据面：

1. **Wave 9：完整 Conversation Event Log（P0，进行中）**
   - 已完成：扩展 `SessionEvent` 覆盖 tool call/result、permission、mode/model change、branch/rewind，并加入 monotonic sequence；
   - 继续完成：补齐 turn/step/checkpoint marker、所有 fork/rewind 写入路径，并将 conversation/execution IDs 统一到持久化事件；
   - 保持旧 session 可 load，继续双写 messages cache；
   - 增加事件重建与 materialized messages 的金丝雀等价测试。

2. **Wave 10：Event-backed Context Projection（P0，进行中）**
   - 已完成：`ContextSession::view` / `SessionView::from_events`，新 compaction/rewind 事件携带 projection snapshot；
   - 已完成：Context 的 observe/project 默认基于 event-backed view，`ProjectionExplain` 暴露 source event count 与 cache fallback；
   - 继续完成：active branch path、旧 cache fallback 的迁移清理、`/context` 或 debug RuntimeEvent 接入，以及跨层重建等价测试。

3. **Wave 11：ModelExecutor 与 runtime crate（P1）**
   - 抽离 ChatClient、stream assembly、overflow retry、usage 更新；
   - 为 fake model executor 增加独立测试；
   - 将 RuntimeHandle/command/event 逐步移入独立 runtime crate；
   - 保留 `zene-core::Agent` 作为默认 wiring 和兼容 facade。

4. **Wave 12：Execution resume 与 Cloud transport（P1）**
   - 实现 safe turn resume；
   - 对 pending tool / approval 强制 inspection，禁止不安全 replay；
   - 将 Cloud JobRunner 与 RuntimeClient 分层，Cloud 只消费统一 runtime events；
   - 增加 worker restart、ACP reconnect 和 crash recovery 集成测试。

5. **持续质量门槛**
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
| 最小公共地基 | **Wave 1** | 控制面与 SoT 都依赖稳定 ID |
| 数据面最大杠杆 | **Wave 2** | 没有事件双写，投影与 fork 长期假 |
| 结构清理 | Wave 3 | 依赖 Wave 2 更干净；可先内部拆仍读 messages |

推荐组合：**Wave 1 → Wave 2 → Wave 3**，再并行 ToolExecutor / ModelExecutor，然后 TurnEngine ports 与 RuntimeHandle。

**不要一上来做** Wave 6 actor 全量重写或 Wave 7 完整崩溃恢复；也不要先堆新 compress phase。

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
- [context-engine-projection.md](./context-engine-projection.md) — Context 投影化路线
- [context-engine.md](./context-engine.md) — 已实现 ContextEngine
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

### 2026-08-12 — 更新 Wave 0–8 实现状态

- PR #54–#58 已合并到 `main`；Wave 0–8 的主要控制面基础能力已完成
- 明确 Wave 2/3/7 仍是过渡态：Session facts 尚未完整事件化，Context 尚未从 events 投影，recovery 尚未自动 resume
- 记录 Wave 9–12：完整 Conversation Event Log、event-backed Context、ModelExecutor/runtime crate、execution resume 与 Cloud RuntimeClient
- 将 `SessionStore` 注入、Subagent TurnEngine 统一、RuntimeHandle 和 ACP recovery metadata 加入实际实现清单

