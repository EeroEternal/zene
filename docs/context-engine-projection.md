# Context Engine 优化：从「压缩器」到「投影引擎」

> **目标定位**：Context Engine 不再「管理并改写历史」，只「根据策略从 Session 事实算出本次请求视图」。

本文是 [session-as-source-of-truth.md](./session-as-source-of-truth.md) 在 Context 侧的落地规划，衔接 [context-engine.md](./context-engine.md)（现状与 API）、[agent-inference-context.md](./agent-inference-context.md)（推理协议）、[ENGINE.md](./ENGINE.md)（turn / compaction 行为）。控制面（Runtime / Turn / ports）见 [agent-runtime-optimization.md](./agent-runtime-optimization.md)。Pi 侧整体对照见 [pi-agent-harness-lessons.md](./pi-agent-harness-lessons.md)。

**不在本文范围**：再堆一种全新 compress 算法；为对齐 Pi 而改成 JSONL 文件格式；把 permission / MCP 塞进 ContextEngine。

---

## 1. 现状判断

> **实现状态（2026-08-13）**：Wave 9–10 的事件事实与 event-backed projection 已完成当前设计范围；Wave 11–12 的 ModelExecutor、runtime contract、safe resume 与 Cloud RuntimeClient 关键边界也已落地。本文以下的“今天 / 目标”对照保留为迁移背景；新的默认路径已是事件优先，剩余内容集中在 legacy fallback、Agent-specific wiring 和外部生命周期边界。

Zene Context Engine **算法已经很强**：

- truncate → slice-keep → LLM summarize
- overflow recovery、input ladder
- prefire two-pass、memory flush
- tool output spill / handles
- full | delta assemble、`context_epoch`、gateway publish

历史短板曾经在 **职责边界**：

```text
历史基线

SessionRecord.messages  ←──  既是历史，又像「当前上下文」
        │
        ▼
ContextEngine.prepare_step
  (estimate / compact / assemble 时可能改写 session.messages)
        │
        ▼
StepContext.messages  →  LLM
```

当前实现已将事件事实与投影分开：`SessionView` 选择 active event path，Context 默认使用 event-backed view；`prepare_step` 内部按 `observe → commit → project` 组织，`messages` 仅作为兼容缓存。仅旧 compaction/rewind 缺少 snapshot 或事件日志不完整时，才使用带原因的 materialized fallback。

---

## 2. 目标形状

```text
Session Event Log          ← 事实：只追加，可回放（zene-session）
        │
        ├─► ContextEngine.project()  →  LLM Context
        ├─► UiProjector              →  Console / ACP transcript
        ├─► ReplayProjector          →  debug / analytics
        └─► ExportProjector          →  share / dump
```

### 四层边界

| 层 | 名称 | 回答什么 | 谁拥有 |
|----|------|----------|--------|
| L0 | **Session Events** | 发生过什么 | `zene-session` |
| L1 | **Active Branch View** | 当前叶到根路径上的事件 | session 查询 API |
| L2 | **Agent Context Plan** | 如何投影（cut point、summary、注入项） | `zene-context` |
| L3 | **Provider Request** | 最终 `messages[]` + metadata | `ContextEngine` → `zene-llm` |

历史上 L0/L1 弱、L2/L3 强，且 L3 容易回写 L0；当前新路径已完成 L0/L1 事件事实与 active-path projection，L2/L3 通过 `observe → commit → project` 分工。后续优化集中在 legacy fallback、可组合 runtime wiring 与外部生命周期边界。

---

## 3. 分阶段路线

### Phase A — Session 事实模型（已完成当前切片）

Conversation SoT 已由 `SessionEvent` 与 `SessionView` 承担。新路径以事件日志为事实源；旧 cache-only session 可显式、幂等迁移，无法无损恢复的 legacy compaction/rewind 继续保留带原因的兼容 fallback。

**逻辑事件类型（一次可不全上）：**

```text
MessageAdded
ToolCall / ToolResult
PermissionDecision
ModelChanged
CompactionApplied      // 追加，不删旧消息
BranchMoved / Forked / Rewound
CheckpointMarked
CustomStateChanged     // todos 等：可不进模型
SystemPrefixChanged
```

**接口含义：**

```rust
// 兼容 API
session.messages() -> &[Message]   // materialized cache

// 当前事实 / 投影入口
session.events() -> &[SessionEvent]
session.view() -> SessionView         // active event path → messages
session.try_view() -> Result<SessionView, ProjectionFallbackReason>
```

**原则：**

- compact **追加** `CompactionApplied { summary, replaces_range | first_kept, segment_ref, tokens_before, … }`
- **不要**把旧 message 从事实日志物理删掉（可进 cold segment，但事件里保留指针）
- `SessionRecord.messages` 继续作为兼容缓存保留；它不是 Source of Truth，cache drift 不覆盖 event-backed projection

**验收：**

- rewind / fork 后能重建与当时一致的 LLM 上下文
- compact 后 UI 仍能打开「压缩前段落」或 segment

相关心智模型：[session-as-source-of-truth.md](./session-as-source-of-truth.md)。

---

### Phase B — `prepare_step` 的 observe / commit / project（已完成）

历史近似流水线：

```text
assemble → estimate → prefire → steps-first → memory flush → compact → epoch++ → StepContext
```

问题：同一次调用既可能 flush memory、改 messages、又 bump epoch。

**目标三阶段：**

```text
1) observe  (只读)
   - 读 active path / 当前 materialized 视图
   - estimate tokens / water level
   - 判断是否需要 compact / memory / steps-first

2) commit   (写 Session 事实，若需要)
   - append CompactionApplied
   - memory flush 落盘（经 handler）
   - checkpoint / compaction segment（经 handler）
   - epoch 策略见 Phase D

3) project  (纯函数倾向)
   - events（或等价输入）→ LLM messages
   - 注入 system / memory reminder / todo reminder
   - tool-output handle 化
   - full | delta 组装
   - 产出 StepContext + ProjectionExplain
```

**API 演进草案：**

```rust
enum ContextAction {
    None,
    StepsFirstTruncate,
    Compact { reason: CompactReason /* … plan fields */ },
    MemoryFlush,
}

struct ContextObservation {
    estimate: u32,
    water: ContextWaterLevel,
    recommended: Vec<ContextAction>,
}

struct ProjectionExplain {
    /// 可选但对「可解释 context」极重要
    kept_event_ids: Vec<String>,
    summary_event_id: Option<String>,
    injected: Vec<&'static str>, // 当前："compaction_summary", "system_reminder"；后续扩展 memory/todos/bg_tasks
    truncated_tools: usize,
    delivery: DeliveryExplain,   // 当前 ACP：delivery + deliveryTailStart
    compact_reason: Option<String>,
}

impl ContextEngine {
    fn observe(&self, deps: &ContextDeps<'_>, tools: &[ToolDefinition]) -> ContextObservation;

    async fn commit(
        &mut self,
        deps: &mut ContextDeps<'_>,
        actions: &[ContextAction],
    ) -> Result<Vec<ContextEvent>>;

    fn project(
        &self,
        deps: &ContextDeps<'_>,
        tools: &[ToolDefinition],
    ) -> Result<(StepContext, ProjectionExplain)>;

    /// 兼容门面：内部改为 observe → commit → project
    async fn prepare_step(
        &mut self,
        deps: &mut ContextDeps<'_>,
        tools: &[ToolDefinition],
    ) -> Result<StepContext> {
        let obs = self.observe(deps, tools);
        self.commit(deps, &obs.recommended).await?;
        Ok(self.project(deps, tools)?.0)
    }
}
```

**收益：**

- 单测可只测 `project`（给定事实 → messages）
- Cloud / ACP 可先 `observe` 再决定是否自动 compact
- 「算上下文」不再悄悄毁掉可回放信息

**溢出路径同样三段式：**

```text
on_context_overflow
  → observe(reason = overflow)
  → commit(truncate and/or compact)
  → project
  → retry
```

---

### Phase C — Compaction = 投影规则，不是改写唯一历史（已完成当前切片）

| 历史兼容形态 | 当前行为 |
|----------------|----------|
| 旧记录只改 `session.messages`：删前缀、插 summary | 新记录追加 `CompactionApplied`；投影使用 snapshot / summary 折叠 range |
| checkpoint / segment 旁路保存 | segment 作为 cold storage，事件带 `segment_ref` |
| `CompactionEntry` 与 messages 双轨 | 事件为主键；`compactions[]` 作为索引 / 兼容 |

**投影规则（伪代码）：**

```text
function project_llm(path, policy):
  if latest CompactionApplied on path:
     messages = system_prefix(policy)
               + [summary_as_message(compaction)]
               + materialize(path after first_kept)
               + injections(policy)
  else:
     messages = materialize(path) + injections(policy)
  apply tool_output_handles(messages)
  apply delivery full|delta
  return messages
```

**现有强项挂到「规划 / 提交」侧，而不是删掉：**

| 能力 | 新角色 |
|------|--------|
| truncate-only / slice-keep / LLM summarize | `CompactionPlan` → commit 为事件 |
| overflow recovery、input ladder | observe / commit |
| prefire two-pass | observe/commit 加速（缓存 plan/summary，不是真相） |
| cumulative file tracking（可选增强） | 写入 compaction `details`，project 时注入 Critical Context |

**验收：**

- `project(events_before)` 与 `project(events_after_compact)` 的差异可解释
- 关闭 auto-commit、仅 project 时结果确定性可测

算法细节仍以 [ENGINE.md](./ENGINE.md) 的 Compaction (v2) 为准；本文只改 **语义归属**。

---

### Phase D — 注入与 epoch：事实 vs 每步装饰

| 内容 | 建议归属 |
|------|----------|
| 用户 / 助手 / 工具消息 | Session 事实 |
| compaction summary | Session 事实（事件） |
| system prompt 基座 | 配置 + 可版本化 prefix；**变更**记事实 |
| memory 每日笔记 | 外部 store；**注入**是投影 |
| todo / background reminder | runtime 状态；**默认投影注入**，不必每条写成 user message |
| plan mode 提示 | 模式变更是事实；提示文本是投影 |
| tool output 句柄 | spill / artifact 是事实；句柄化是投影 |

**规则：**

```text
会改变「模型长期故事」的 → commit 为事件
只影响「这一步怎么提示」的 → project 时注入
```

**epoch 策略收紧（对齐 gateway delta / cache）：**

- 只有 **稳定 prefix 集合** 变化才 `epoch++`（system 基座、compaction 边界、pinned 区）
- 每步都变的 reminder（todos 计数、bg task）**尽量不 bump epoch**，避免 cache 抖动
- `ProjectionExplain.injected` 标明本步装饰，供 UI / ACP 展示「模型额外看到了什么」；当前已识别 `compaction_summary` 和 `system_reminder`
- RuntimeEvent / ACP `projection_update` 当前暴露 `sourceEventCount`、`activeEventCount`、`cacheDriftDetected`、分支路径、fallback、`injected`、`delivery` 和 `deliveryTailStart`
- 完整事件日志优先于 materialized `messages` cache；cache drift 只进入 explain，不覆盖 event-backed projection；仅 legacy / incomplete event log 触发兼容 fallback

与现有 `pinned_boundary` / `PublishPrefix` / delta `tail_start` 一致：把「什么算 pinned prefix」写进投影契约，而不是散落在 assemble 细节里。见 [context-engine.md](./context-engine.md) Phase 3–5。

---

### Phase E — 可解释与可观测（已完成当前设计范围）

引擎已能压 token，且 projection explain 已通过 RuntimeEvent / ACP 让人与系统看懂投影；后续仅按 Console 产品需求扩展 provenance 类型。

**`StepContext` 扩展（或并行返回）：**

```rust
struct StepContext {
    messages: Vec<Message>,
    metadata: ContextMetadata,
    estimate_tokens: u32,
    // 新增：或由 prepare_step / project 额外返回
    // explain: ProjectionExplain,
}
```

**Console / ACP `/context`（或等价调试通道）展示：**

- estimate vs provider `prompt_tokens` vs water level
- 最近一次 compact 原因、`tokens_before` / `tokens_after`
- 保留了哪些 turn、折叠了哪段
- 本步是否注入 memory / todos / bg
- 多少 tool 结果被 truncate / handle 化
- full 还是 delta、`tail_start`、`context_epoch`

`Agent::context_report()` 已展示上述 projection explain 摘要；ACP `projection_update` 提供结构化明细。没有 explain，Context Engine 是黑盒优化器；有了 explain，才是可治理的投影层。

---

## 4. 现有模块对号入座

| 现有能力 | 新角色 |
|----------|--------|
| `TokenEstimator` / water | **observe** |
| truncate / slice / summarize / input_ladder | **plan + commit**（compaction 事件） |
| prefire | **observe/commit 加速路径** |
| memory flush | **commit 副作用** + **project 注入** |
| `assemble_outbound` full/delta | **project 末端**（纯） |
| tool output handles / spill | spill = artifact 事实；handle = project |
| `ContextEventHandler` | commit 的 IO 出口（保持） |
| `ContextHooks`（todo/bg） | **project 注入源**，避免写进 SoT messages |
| `ContextSession` | 已支持 `events`、active-path `view`、严格 `try_view` 与兼容 cache |

**当前 `ContextSession` 形态：**

```rust
trait ContextSession {
    fn session_id(&self) -> &str;
    fn messages(&self) -> &[Message];             // compatibility cache
    fn messages_mut(&mut self) -> &mut Vec<Message>; // compatibility boundary
    fn events(&self) -> &[SessionEvent];
    fn view(&self) -> SessionView;               // event-backed projection
    fn try_view(&self) -> Result<SessionView, ProjectionFallbackReason>;

    fn commit_compaction_snapshot(/* … */);
    fn record_compaction_event(/* … */);
    fn persist_checkpoint(&mut self, reason: &str) -> anyhow::Result<()>;
}
```

`messages_mut` 仍为兼容边界，但 Context 代码不直接修改它；compaction 通过 session commit 方法追加事实并刷新 cache。具体字段以 `crates/context` / `crates/session` 实现为准，本文约束 **语义** 而非一次 API 冻结。

---

## 5. 给 runtime 的目标调用形态

```rust
// 每一步（理想形态）
let obs = engine.observe(&deps, tools)?;
if !obs.recommended.is_empty() {
    engine.commit(&mut deps, &obs.recommended).await?;
}
let (step, explain) = engine.project(&deps, tools)?;

client
    .chat(ChatRequest {
        messages: step.messages,
        context: Some(step.metadata),
        ..Default::default()
    })
    .await?;

engine.record_step_usage(usage, /* … */)?;

// 调试 / Console
// emit(ContextDebug { explain, water: engine.water() });
```

兼容期继续暴露 `prepare_step`，内部切换为三段式，避免 ACP / Cloud / CLI 同步大爆炸。

---

## 6. 优先级

| 优先级 | 项 | 原因 |
|--------|----|------|
| **已完成** | Session 事件化 + compact 追加而非物理删 | 新路径以事件为事实，旧格式保留显式 fallback |
| **已完成** | `observe` / `commit` / `project` 拆分 | Context projection 默认消费 event-backed view |
| **已完成** | `ProjectionExplain` + RuntimeEvent / ACP 明细 | 已覆盖 path、fallback、injected、tool、retained-turn、delivery provenance |
| **已完成** | 注入物分类 + epoch / delta 规则 | 与 gateway publish / `tail_start` 对齐 |
| **剩余** | legacy fallback 与历史格式清理 | 仅清理可无损迁移的兼容代码 |
| **剩余** | UI / replay 与 LLM 共享 active path 查询 | 按 Console 产品需求继续扩展 |

### 6.1 与 AgentRuntime 合并后的 Wave 映射

全文落地顺序以
[agent-runtime-optimization.md §16](./agent-runtime-optimization.md#16-merged-implementation-waves)
为准。本文 Phase 的当前状态：

| 本文 | Merged Wave | 当前状态 |
|------|-------------|----------|
| Phase A Session 事实 | **Wave 9–10** | 已完成事件事实、active path、显式 migration / fallback |
| Phase B observe/commit/project | **Wave 10** | 已完成；Context 默认使用 event-backed view |
| Phase C compact 事件化 | **Wave 9–10** | 已完成当前切片；旧 snapshot 缺失时保留 fallback |
| Phase D 注入 / epoch | **Wave 10–11** | 已完成当前规则与 RuntimeEvent / ACP 暴露 |
| Phase E explain | **Wave 10** | 已完成当前设计范围；后续按 Console 需求扩展 provenance |

**API 造型：** TurnEngine 只依赖 `ContextAssembler::prepare` /
`handle_overflow`；三段式是 `ContextEngine` **内部** 实现，
不是再暴露第三套全局回调。

**当前不属于剩余主线：**

- 再引入全新 compress phase
- 为像 Pi 而改存储格式
- 把 permission / MCP 逻辑迁入 `zene-context`
- 为已完成的 Runtime protocol / actor contract 再做一次全量重写

剩余主线是 Agent-specific driver wiring 的进一步 crate 化、可无损迁移的 legacy cleanup，以及 Cloud/ACP/runtime 的部署级生命周期与共享 outbox 边界。

---

## 7. 迁移与风险控制

1. **兼容期**  
   新路径优先使用事件事实与 active-path projection；`SessionRecord.messages` 继续保留为兼容 cache。

2. **投影等价性**  
   用事件投影出的 messages 与 materialized `messages` 做 diff 测试；cache drift 只诊断，不覆盖事件投影。

3. **`prepare_step` 保持门面**  
   外部调用点不变，内部使用 observe → commit → project。

4. **显式 fallback**  
   旧 compaction/rewind 缺 snapshot 或事件日志不完整时保留 fallback reason，并由 `try_view` 严格拒绝隐式 fallback。

5. **fork / rewind 作为主验收场景**  
   两类测试通过，才说明投影模型立住。

6. **行为不变优先**  
   与 [context-engine.md](./context-engine.md) Phase 1 相同纪律：拆分与事件化默认 **不改变** 默认 compact 阈值阈值，除非单独 RFC。

---

## 8. 和现有文档的关系

| 文档 | 关系 |
|------|------|
| [session-as-source-of-truth.md](./session-as-source-of-truth.md) | 心智模型：Session 事实 vs Context 投影 |
| [context-engine.md](./context-engine.md) | 已实现 crate 边界、`prepare_step`、gateway / delta |
| [agent-inference-context.md](./agent-inference-context.md) | epoch、delta、与推理层协议 |
| [ENGINE.md](./ENGINE.md) | compaction 阶段、water、steer 等运行时行为 |
| [agent-components.md](./agent-components.md) | 可组装组件栈与发布边界 |
| [agent-runtime-optimization.md](./agent-runtime-optimization.md) | Runtime / Turn / ports；Merged Wave |
| [pi-agent-harness-lessons.md](./pi-agent-harness-lessons.md) | Pi Harness 对照与启发总览 |

本文 **不替代** `context-engine.md` 的实现说明；它记录已完成的事件投影边界、兼容策略，以及下一阶段的语义与组合优化方向。

---

## 9. 一句话

> **Context Engine 当前已是以 Session 事件事实为输入、以 active-path projection 为默认路径、以 `ProjectionExplain` 解释每一步模型视图的投影引擎；剩余优化在兼容清理、组合边界与外部生命周期。**

算法层已具备优势；下一杠杆不是继续堆 compression phase，而是收口 legacy fallback、Agent-specific runtime wiring 与部署级恢复边界。

---

## 讨论记录

### 2026-08-11 — 初稿

- 自 Pi Session Tree 与 Zene 现状对照引出：SoT / 投影四层、`observe|commit|project`、compaction 事件化、注入与 epoch、ProjectionExplain
- 明确不照搬 Pi 格式；优先双写与 `prepare_step` 门面兼容

### 2026-08-11 — 对齐 AgentRuntime

- 优先级表增加 Merged Wave 映射（Wave 1→3 优先）
- `ContextAssembler` 为对外 port，三段式为对内实现
- 交叉引用 [agent-runtime-optimization.md](./agent-runtime-optimization.md)
