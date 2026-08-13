# ContextEngine

Zene 的语义上下文引擎（`crates/context`）。它从 Session 事实算出**这一次**发给模型的视图，不把「模型碰巧看到的 messages」当成会话历史。

相关实现：`zene-context`、`zene-llm` 协议字段、`zene-core` 组装。心智模型见 [session-as-source-of-truth.md](./session-as-source-of-truth.md)；推理协议见 [agent-inference-context.md](./agent-inference-context.md)；compaction 算法细节见 [ENGINE.md](./ENGINE.md)；控制面见 [agent-runtime-optimization.md](./agent-runtime-optimization.md)。

**不在本文范围**：新的 compress 算法、改成 Pi JSONL、把 permission / MCP / Turn 塞进 ContextEngine。

**进度（2026-08-13）**：crate 边界、`observe → commit → project`、event-backed projection、前缀三区、Plan/overflow 去改写、prefix-adjacent 注入拖尾已在实现里。剩余是 legacy fallback 清理，以及 Console 对 `prefixCache` 的展示。

---

## 1. 职责

```
Agent / runtime     turn、tools、permission、steer、审批
ContextEngine       estimate → compact → memory → assemble → epoch → 前缀布局
推理层              会话亲和、KV / prompt cache、cached_tokens 回传
```

```
zene-session     持久化：events、兼容 messages cache、checkpoints、todos
zene-context     语义上下文：estimate、compact、memory、prefire、epoch、assemble、layout
zene-llm         ChatRequest + ContextMetadata、TokenUsage.cached_tokens
zene-core        composition root
```

依赖：`core → context → {session, llm}`，`llm` 不依赖 `context`。

四层：

| 层 | 名称 | 回答什么 | 谁拥有 |
|----|------|----------|--------|
| L0 | Session Events | 发生过什么 | `zene-session` |
| L1 | Active Branch | 当前叶到根 | session `view` / `try_view` |
| L2 | Context Plan | 如何投影（cut、summary、注入、三区） | `zene-context` |
| L3 | Provider Request | 最终 `messages[]` + metadata | `ContextEngine` → `zene-llm` |

同一份 Session 还可投影出 UI transcript、replay、export；那些不是 ContextEngine 的职责。

---

## 2. API

Runtime 主要调：

| 方法 | 用途 |
|------|------|
| `prepare_step(deps, tools)` | 门面：observe → commit → project |
| `record_step_usage` | water + `cached_tokens` + session 占用 |
| `handle_overflow` | 当前 turn steps-first truncate，不够再完整 compact |
| `compact_forced` | `/compact` |
| `set_step_tail_decorations` | 无 hooks 时的尾巴注入 |
| `on_system_prefix_changed` | 真正改冻结 system 时 `epoch++` |
| `metadata` / `water` | 出站 metadata、`/context` |

`prepare_step` 不再在同一次调用里偷偷既改历史又当唯一真相。三段式：

```text
observe   只读 SessionView，估算 token / water，决定是否 compact
commit    唯一写 Conversation SoT 的入口（CompactionApplied、memory flush、checkpoint）
project   事件路径 → messages；尾巴注入 reminder；full|delta 组装；ProjectionExplain
```

Compact **追加** `CompactionApplied`，不把旧事件从事实日志物理删掉。`SessionRecord.messages` 只是兼容缓存；cache drift 不覆盖 event-backed projection。

`ContextHooks::step_tail_decorations` 是 todos / plan / 后台任务的注入源，写进出站尾巴，不写进 SoT。

---

## 3. 核心类型（现状）

```rust
pub struct StepContext {
    pub messages: Vec<Message>,
    pub metadata: ContextMetadata,
    pub estimate_tokens: u32,
}

pub struct ContextMetadata {
    pub session_id: String,
    pub context_epoch: u64,
    pub prefix_hash: Option<String>,
    pub delivery: ContextDelivery, // full | delta
    pub tail_start: Option<usize>,
}

pub struct ProjectionExplain { /* path / fallback / injected / delivery / prefix_cache */ }
pub struct PrefixCacheExplain {
    pub prefix_end: usize,
    pub body_end: usize,
    pub tail_decoration_count: usize,
    pub prefix_fingerprint: Option<String>,
    pub break_kind: String, // none | compact | system_resize | injected_resize | body_mutate | unknown
    pub cached_tokens: Option<u64>,
    pub unchanged_reprocessed_est: Option<u64>,
}
```

`ChatRequest.context` 与 `TokenUsage.cached_tokens` 在 `zene-llm`。Provider 透传 `X-Zene-Session-Id` / `X-Zene-Context-Epoch`。ACP `projection_update._meta.prefixCache` 带上 zone 与 `breakKind`。

---

## 4. 前缀稳定与 Prefix Cache

厂商 prefix cache 只认：从 prompt 左侧起，连续多少 token 与上一请求 **字节级相同**。`session_id` / `epoch` / `prefix_hash` 是给网关的信号，不能替代字节前缀。

一次 DeepSeek-V4-Pro 诊断（11 次 LLM call，窗口 56.2k）：注入块 `<agent_documents_index>` 只有 698 token，却因 3 次 resize 让约 52k 未变更 token 被重算。位置比体积更贵。

```text
可变内容的位置 ≫ 可变内容的大小
意外打断 ≫ 一次合法 compact
epoch 正确 ≠ 前缀字节稳定
```

### 布局契约

```text
[冻结 system 基座] [pinned / compaction 边界] [只追加的对话 + 工具] [本步装饰]
        ← 稳定前缀：变了才 epoch++ →           ← 只往尾部涨 →         ← 只放尾巴 →
```

实现：`crates/context/src/layout.rs`。`project()` 先把紧贴 pinned 前缀的 reminder 拖到尾巴，再按 hooks 换成当前装饰。历史中间残留的旧 reminder 保持不动（字节冻结）。

索引 / RAG 只允许：开工写入冻结 system（定长或不再改），或当本步 tail。禁止做成 msg[1] 那种变长块（`InjectionZone::BodyInsert`）。

Compact 是 **允许的一次打断**（`epoch++`）。要消灭的是同一会话里 system / 注入块 / 旧 tool 被反复 resize。

### 已落地（Phase P–R）

- 三区：`split_layout` / `PrefixCacheExplain`
- Plan reminder、todos、后台任务走 tail，进出 Plan 不改 system、不 bump epoch
- Overflow 先 `apply_steps_truncate_pass`（当前 user 之后）；不够再完整 compact
- Compact 快照不再持久化 volatile `<system-reminder>`
- Memory 开工写入 system 一次；本步可见的更新走 tail
- Workspace / skills 只在 session start 编进 system
- `break_kind` + `cached_tokens` 进入 explain / ACP（`cached_tokens` 为上一轮 provider 回传）
- Phase S：`InjectionZone`（FrozenPrefix / TailDecorations）；`project()` 把紧贴 pinned 前缀的 reminder 拖到尾巴；debug 断言拒绝 msg[1] 注入块

### 还没做完

| 项 | 说明 |
|----|------|
| `cached_tokens` 当次闭环 | 现在是上一轮回填；UsageUpdate 已有当次值，Console 条形图仍非目标 |
| 旧 compact reminder 在 body 中间 | 保持冻结，不回写；新 compact 不再写入 |
| legacy session fallback | 仅清理可无损迁移的兼容代码 |

Water / auto-compact 仍看窗口占用 `max(usage, estimate)`，不因 cache 命中率推迟 compact。

---

## 5. 出站：epoch、delta、gateway

已实现行为（原 Phase 0–5）：

- compact 或真正的 system 基座变更 → `epoch++` 并 `PublishPrefix`
- `ZENE_CONTEXT_DELIVERY=full|delta`（配 `ZENE_INFERENCE_GATEWAY_URL` 时默认 delta）
- `pinned_boundary` = `stable_system_boundary`（system + compaction summary）
- 大 tool 输出可句柄化（`ZENE_TOOL_OUTPUT_HANDLES`）
- Cloud Worker 注入 `ZENE_RUN_ID`；Run 结束 `close_session`
- 网关：`apps/inference-gateway`，可选 Redis session store

推理收益三档仍见 [agent-inference-context.md](./agent-inference-context.md)。**A 档**（full messages + 稳定前缀）是 prefix cache 的大头；前缀抖动时 B/C 档也救不了。

---

## 6. 数据流

```mermaid
sequenceDiagram
    participant RT as Agent (runtime)
    participant CE as ContextEngine
    participant SE as SessionRecord
    participant LLM as zene-llm

    RT->>CE: prepare_step(deps, tools)
    CE->>SE: observe / commit compact
    CE-->>RT: StepContext + ProjectionExplain
    RT->>LLM: ChatRequest + ContextMetadata
    LLM-->>RT: response + cached_tokens
    RT->>CE: record_step_usage(usage)
    CE->>SE: update_context_usage
```

TurnEngine 只依赖 `ContextAssembler::prepare` / `handle_overflow`；三段式是引擎内部实现。

---

## 7. 剩余工作

- 仅清理可无损迁移的 legacy session fallback
- Console 按产品需求展示 `prefixCache`（非第一版必做彩图）
- Agent-specific runtime wiring 的进一步 crate 化（控制面，见 runtime 文档）

---

## 相关文档

- [session-as-source-of-truth.md](./session-as-source-of-truth.md) — Session 事实 vs Context 投影
- [agent-inference-context.md](./agent-inference-context.md) — 与推理层的 session / cache / 续算
- [ENGINE.md](./ENGINE.md) — turn、compaction 算法、memory、sandbox
- [agent-components.md](./agent-components.md) — 可组装组件栈
- [agent-runtime-optimization.md](./agent-runtime-optimization.md) — 控制面；本文不替代
- [pi-agent-harness-lessons.md](./pi-agent-harness-lessons.md) — Pi 对照

曾拆成 `context-engine-projection.md` 与 `context-engine-prefix-cache.md`，已并入本文。

---

## 讨论记录

### 2026-08-13 — 前缀稳定 + 文档合并

- DeepSeek 诊断：msg[1] 注入块 resize 导致约 52k 未变更 token 重算。
- PR #70：三区布局、Plan/overflow 去改写、`prefixCache` 观测。
- Phase S：`InjectionZone`、`project()` 拖走 prefix-adjacent reminder、debug 断言。

### 2026-08-11 — 投影化与 Runtime 对齐

- SoT / 投影四层、`observe|commit|project`、compaction 事件化、ProjectionExplain。
- TurnEngine 只依赖 assembler port；三段式为对内实现。

### 2026-08-10–11 — crate 与网关

- 抽出 `zene-context`；delta / tool handle / `pinned_boundary` / Cloud publish。
- inference-gateway 经 unigateway-sdk session 演进（版本细节见 git 历史）。
