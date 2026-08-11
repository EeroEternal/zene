# Context Engine 优化：从「压缩器」到「投影引擎」

> **目标定位**：Context Engine 不再「管理并改写历史」，只「根据策略从 Session 事实算出本次请求视图」。

本文是 [session-as-source-of-truth.md](./session-as-source-of-truth.md) 在 Context 侧的落地规划，衔接 [context-engine.md](./context-engine.md)（现状与 API）、[agent-inference-context.md](./agent-inference-context.md)（推理协议）、[ENGINE.md](./ENGINE.md)（turn / compaction 行为）。Pi 侧整体对照见 [pi-agent-harness-lessons.md](./pi-agent-harness-lessons.md)。

**不在本文范围**：再堆一种全新 compress 算法；为对齐 Pi 而改成 JSONL 文件格式；把 permission / MCP 塞进 ContextEngine。

---

## 1. 现状判断

Zene Context Engine **算法已经很强**：

- truncate → slice-keep → LLM summarize
- overflow recovery、input ladder
- prefire two-pass、memory flush
- tool output spill / handles
- full | delta assemble、`context_epoch`、gateway publish

短板不在压缩策略本身，而在 **职责边界**：

```text
今天（简化）

SessionRecord.messages  ←──  既是历史，又像「当前上下文」
        │
        ▼
ContextEngine.prepare_step
  (estimate / compact / assemble 时可能改写 session.messages)
        │
        ▼
StepContext.messages  →  LLM
```

`prepare_step` 把 **只读观测、写事实、投影出站** 缠在同一次调用里；compact 容易变成对唯一 `messages` 数组的原地变异，而不是「追加压缩事件 + 重新投影」。

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

今天 L0/L1 弱、L2/L3 强，且 L3 经常回写 L0。  
优化顺序：**先稳 L0 → 再让 L2 只读 L0 → 最后 L3 纯函数化**。

---

## 3. 分阶段路线

### Phase A — Session 事实模型（Context 的前置依赖）

没有稳的事实源，Context Engine 只能继续改 `Vec<Message>`。

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
// 今天
session.messages() -> &[Message]   // 已经是「半投影」

// 目标
session.events() -> &[SessionEvent]
session.active_path() -> &[SessionEvent]   // leaf → root
// materialize 可由 session helper 或 context 提供
```

**原则：**

- compact **追加** `CompactionApplied { summary, replaces_range | first_kept, segment_ref, tokens_before, … }`
- **不要**把旧 message 从事实日志物理删掉（可进 cold segment，但事件里保留指针）
- `SessionRecord.messages` 过渡期可保留为 **active projection 缓存**，代码与文档须标明：它不是 Source of Truth

**验收：**

- rewind / fork 后能重建与当时一致的 LLM 上下文
- compact 后 UI 仍能打开「压缩前段落」或 segment

相关心智模型：[session-as-source-of-truth.md](./session-as-source-of-truth.md)。

---

### Phase B — 拆分 `prepare_step`：observe / commit / project

今天近似流水线：

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
    injected: Vec<&'static str>, // "memory", "todos", "bg_tasks", …
    truncated_tools: usize,
    delivery: DeliveryExplain,   // Full | Delta { tail_start }
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

### Phase C — Compaction = 投影规则，不是改写唯一历史

| 现行为（简化） | 目标行为 |
|----------------|----------|
| 改 `session.messages`：删前缀、插 summary | 追加 `CompactionApplied`；投影时用 summary 折叠 range |
| checkpoint / segment 旁路保存 | segment 作为 cold storage，事件带 `segment_ref` |
| `CompactionEntry` 与 messages 双轨 | 事件为主键；`compactions[]` 可作索引 / 兼容 |

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
- `ProjectionExplain.injected` 标明本步装饰，供 UI / ACP 展示「模型额外看到了什么」

与现有 `pinned_boundary` / `PublishPrefix` / delta `tail_start` 一致：把「什么算 pinned prefix」写进投影契约，而不是散落在 assemble 细节里。见 [context-engine.md](./context-engine.md) Phase 3–5。

---

### Phase E — 可解释与可观测

引擎已能压 token；下一步是 **让人与系统看懂投影**。

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

**Console / ACP `/context`（或等价调试通道）建议展示：**

- estimate vs provider `prompt_tokens` vs water level
- 最近一次 compact 原因、`tokens_before` / `tokens_after`
- 保留了哪些 turn、折叠了哪段
- 本步是否注入 memory / todos / bg
- 多少 tool 结果被 truncate / handle 化
- full 还是 delta、`tail_start`、`context_epoch`

没有 explain，Context Engine 是黑盒优化器；有了 explain，才是可治理的投影层。

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
| `ContextSession` | 从「messages 读写」扩到「events + 可选 cache」 |

**`ContextSession` 演进示意：**

```rust
trait ContextSession {
    fn session_id(&self) -> &str;

    // 过渡期保留
    fn messages(&self) -> &[Message];
    fn messages_mut(&mut self) -> &mut Vec<Message>; // 逐步收敛

    // 目标
    fn append_event(&mut self, ev: SessionEvent);
    fn active_events(&self) -> &[SessionEvent];

    fn record_compaction_event(
        &mut self,
        reason: &str,
        compacted_count: usize,
        summary: Option<String>,
        tokens_before: Option<u32>,
        tokens_after: Option<u32>,
    );
    fn persist_checkpoint(&mut self, reason: &str) -> anyhow::Result<()>;
}
```

具体字段以 `crates/context` / `crates/session` 实现为准，本文约束 **语义** 而非一次 API 冻结。

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
| **P0** | Session 事件化 + compact 追加而非物理删 | 否则 fork/rewind/可解释全部不稳 |
| **P0** | `observe` / `commit` / `project` 拆分 | 可测、可缓存、可对 UI 暴露 |
| **P1** | `ProjectionExplain` + `/context` 可视化 | 产品与调试立刻受益 |
| **P1** | 注入物分类 + epoch 规则 | 稳住 delta / prompt cache |
| **P2** | branch summary、file-ops 累积进 summary details | 长会话增强，不阻塞主线 |
| **P2** | `messages` 降级为 materialized cache | 兼容旧 API，内部切 SoT |
| **P3** | UI / replay 与 LLM 共享同一 active path 查询 | 统一 ACP / Cloud / CLI |

**暂不必优先：**

- 再引入全新 compress phase
- 为像 Pi 而改存储格式
- 把 permission / MCP 逻辑迁入 `zene-context`

---

## 7. 迁移与风险控制

1. **双写期**  
   compact 仍更新 `SessionRecord.messages`（兼容），同时 append 事件 / 完善 `segment_ref`。

2. **project 金丝雀**  
   用事件投影出的 messages 与现网 materialized `messages` 做 diff 测试（维护等价规则表）。

3. **`prepare_step` 保持门面**  
   外部调用点先不动，内部改为 observe → commit → project。

4. **先解释、后切换 SoT**  
   先上 `ProjectionExplain`，再切「以事件回放为准」。

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
| [pi-agent-harness-lessons.md](./pi-agent-harness-lessons.md) | Pi Harness 对照与启发总览 |

本文 **不替代** `context-engine.md` 的实现说明，只定义 **下一阶段语义优化方向**。

---

## 9. 一句话

> **Context Engine 的优化方向：从「会改历史的压缩器」，变成「只读 Session 事实、可提交压缩事件、可纯函数投影、可解释每一步模型看见了什么」的投影引擎。**

算法层已具备优势；下一杠杆在 **事实模型 + 读写分离 + 可解释投影**，而不是继续堆 compression phase。

---

## 讨论记录

### 2026-08-11 — 初稿

- 自 Pi Session Tree 与 Zene 现状对照引出：SoT / 投影四层、`observe|commit|project`、compaction 事件化、注入与 epoch、ProjectionExplain
- 明确不照搬 Pi 格式；优先双写与 `prepare_step` 门面兼容
