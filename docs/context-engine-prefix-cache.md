# Context Engine：前缀稳定与 Prefix Cache

> **目标**：把「发给模型的字节前缀尽量不变」提升为与 compaction 同级的投影约束。  
> 下一杠杆不是再堆压缩算法，而是停止在稳定前缀和历史中间插入、改写会变长的块。

本文是 [context-engine-projection.md](./context-engine-projection.md) 在 **cache 友好布局** 上的续篇，衔接 [agent-inference-context.md](./agent-inference-context.md)（`session_id` / `epoch` / `cached_tokens`）和 [ENGINE.md](./ENGINE.md)（compaction、memory、system reminder）。控制面仍见 [agent-runtime-optimization.md](./agent-runtime-optimization.md)；本文不改 Turn / Permission / ACP 语义。

**状态**：落地中（2026-08-13）。Phase P 布局契约、Phase Q 已知前缀改写、Phase R 观测字段已开始进入 `zene-context` / ACP `projection_update`。

---

## 1. 问题

Provider prefix cache（DeepSeek / OpenAI 等）只认一件事：

> 从 prompt 左侧起，连续多少 token 与上一请求 **字节级相同**。

`session_id`、`context_epoch`、`prefix_hash` 是给网关 / 引擎的协议信号，**不直接决定** 厂商 cache 是否命中。epoch 没变但 msg[1] 变长，后面整段历史仍会 miss。

一次 DeepSeek-V4-Pro 多轮 Agent 诊断（11 次 LLM call，最终窗口 56.2k）量化了成本：

| 现象 | 数字 |
|------|------|
| 因 system 或注入块 resize 打断 cache | 4 / 11 次调用 |
| 注入块本身 | 698 token（`<agent_documents_index>`） |
| 被打断后「其实没变却重算」的 token | **52.0k** |
| 全会话累计重算 | 121.2k |
| 注入块 resize 次数 | 3（贡献约 51.6k 浪费） |
| system prompt resize | 1 次 |

最大的罪魁不是 context 太大（只用了 1M 窗口的 5%），而是 **可变索引块插在 system 之后、对话历史之前**。位置比体积更贵：698 token 的 resize 作废了后面几十 k 的 assistant + tool。

后面几次调用前缀一旦稳住，命中接近完美（含 identical payload、0 重算）。说明 Agent 场景下 prefix cache **可以** 很好；前提是不要反复打断。

对 Zene 的直接推论：

```text
可变内容的位置 ≫ 可变内容的大小
意外打断 ≫ 一次合法 compact
epoch 正确 ≠ 前缀字节稳定
```

Compaction 是 **允许的一次打断**（`epoch++`，付一次账，换新稳定前缀）。要防的是同一会话里 system / 注入块 / 旧 tool 被反复 resize。

---

## 2. 现状：已经做对的，和还没钉死的

### 已经做对

与这张诊断图对齐的骨架已经在：

- Session 是事实、Context 是投影（[session-as-source-of-truth.md](./session-as-source-of-truth.md)）。
- `observe → commit → project`；`stable_system_boundary` / `pinned_boundary` 标记可缓存前缀。
- Session 开始把 memory 写入 system 的 `<memory-context>`，意图是 KV-friendly 前缀（[ENGINE.md](./ENGINE.md)）。
- Phase D：todos / 后台任务等每步装饰 **尽量不 bump epoch**（[context-engine-projection.md](./context-engine-projection.md)）。
- Compact 后 `<system-reminder>` 挂在 assemble 结果的 **尾巴**（`system + last_user + recent + summary + reminder`）。
- 出站带 `prefix_hash` / `context_epoch`；`record_step_usage` 已能打 `cached_tokens` 日志。
- Tool 超长输出 spill 成 handle，避免把大 payload 反复塞进后续请求。

### 缺口

Phase D 解决的是「哪些注入算事实、哪些不 bump epoch」。厂商 cache 不看 epoch，只看拼出来的 `messages[]` 左侧有没有动。

因此仍可能出现「协议层稳定、字节前缀抖动」：

| 当前行为 | 对应图中的打断 | 说明 |
|----------|----------------|------|
| Plan mode 把 reminder **拼进 system**（`build_effective_system_prompt`） | Call 2 system resized | 进出 Plan 都会改整段前缀 |
| Overflow / phase-1 对 **旧 tool** 原地 truncate | 历史中间改字节 | 与注入块 resize 同类：后面全部 miss |
| 未来若把目录索引 / RAG / documents 做成 msg[1] | Call 3/5/6 | 体积可以很小，位置足以毁掉 cache |
| Memory 若中途回写 system | Call 2 | 开工注入一次是对的；变长后再改 system 就不是 |
| `ProjectionExplain` 无 cache 断点 | 看不见 52k 浪费 | 只有日志里的 `cache_pct`，Console / ACP 无法画打断图 |

`ensure_system_message` 在已有 system 时是幂等的（不覆盖）；真正改前缀的是 `update_system_prefix` 和「改历史中间某条 message」。Workspace / skills 目前在 `AgentBuilder` 开工时编进 system，这是对的，**不要**在 Agent 改文件后再刷新进 system。

---

## 3. 目标布局（硬契约）

`project()` 产出的 LLM messages 必须遵守：

```text
[冻结 system 基座] [pinned / compaction 边界] [只追加的对话 + 工具] [本步装饰]
        ← 稳定前缀：变了才 epoch++ →           ← 只往尾部涨 →         ← 只放尾巴 →
```

含义：

- **冻结 system**：session 开始时写入的 base prompt、workspace 快照、skills 列表、`<memory-context>`。之后默认不再改长度。模式切换、todos、本步提示 **不得** 再拼进这一段。
- **Pinned 边界**：`stable_system_boundary`（system + 已提交的 compaction summary）。越过这条边界插入变长块，视为 bug。
- **只追加的对话体**：user / assistant / tool 只 append。需要丢掉旧细节时走 compaction 事件（合法 `epoch++`），而不是改已发出去的中间消息。
- **本步装饰**：plan reminder、todos、后台任务、去重提示、本步 RAG 片段 → 始终在 **最后一条**（或最后一组）`<system-reminder>`。不 bump epoch。

禁止的形状（图中的 `<agent_documents_index>`）：

```text
system
user/injected: <index 本次 698 tok，下次 900 tok>   ← 打断点
… 后面几十 k 历史全部 miss
```

允许的两种索引 / RAG 放置：

- 开工时写入冻结 system（之后不再变长；必要时定长槽 / 截断到上限）。
- 仅作为 **本步 tail** 装饰（不影响已缓存前缀）。

Compact 后的目标形状与现在 assemble 一致，只是把「reminder 在尾」从实现细节升级成契约：

```text
system（冻结）
compaction_summary（新 pinned，epoch++ 一次）
recent tail（只追加）
<system-reminder>（本步才变）
```

---

## 4. 分阶段落地

不改默认 compact 阈值，不引入新的 compress phase。验收以「意外 cache 打断次数」和 `cached_tokens` 为主，而不是更激进的 summarize。

### Phase P — 布局契约（投影层）

在 `zene-context` 的 `project()` / `assemble_outbound` 把三区写进类型或断言，而不是注释：

- `PrefixZone`：`[..stable_system_boundary]`
- `BodyZone`：边界到最后一个非装饰 message
- `TailDecorations`：plan / todos / bg / dedup / 本步检索

`ProjectionExplain` 增加 zone 边界（index 或 token 估计），供测试断言「装饰从未出现在 PrefixZone / BodyZone 中间」。

**验收**：单测覆盖「todos 变了、plan 开关了，PrefixZone 字节不变」；故意把装饰插到 msg[1] 的路径编不过或测试失败。

### Phase Q — 停掉已知前缀改写

**Plan mode**：`build_effective_system_prompt` 不再把 `PLAN_MODE_REMINDER` 拼进 system。模式变更继续记 Session 事件；投影时把 reminder 放进 `TailDecorations`。进出 Plan 不得 `epoch++`（除非同时发生 compact / 真正的 system 基座变更）。

**Overflow truncate**：优先 `apply_steps_truncate_pass`（只截 **当前 user 之后** 的 tool，属于 tail）。对 compactable **旧前缀** 的原地 truncate 视为 cache-hostile；能靠 steps-first 或一次完整 compact 解决就不要改更早的消息。完整 compact 仍是合法打断。

**Memory**：保持开工写入 system 一次。当日新笔记不回写 system；若本步必须可见，走 tail reminder。下一 session 再把更新后的 memory 编进新的冻结前缀。

**System 快照**：workspace overview / skills / AGENTS.md 只在 session start（及明确的「用户要求刷新上下文」）重编。Agent 自己 Write/Edit 文件不得隐式刷新 system。

**验收**：进出 Plan、更新 todos、steps-first truncate 三条路径上，`prefix_hash`（或 PrefixZone fingerprint）与上一 LLM call 相同；仅 compact / 用户刷新 / 首次 publish 允许变化。

### Phase R — Cache 断点可观测

`cached_tokens` 已从 provider 回来，但只进日志。升到投影 / RuntimeEvent，才能画出与诊断图同类的打断。

建议字段（可放进 `ProjectionExplain` 或并列的 `PrefixCacheExplain`）：

```text
prompt_tokens
cached_tokens
prefix_zone_tokens          // 估计或上次 publish 的 pinned 长度
break_kind                  // none | compact | system_resize | injected_resize | body_mutate | unknown
unchanged_reprocessed_est   // max(0, previous_cached_or_prefix - new_cached) 的保守估计
```

ACP `projection_update` / Cloud 时间线带上 `cached_tokens` 与 `break_kind`。Console 不需要第一版就做满彩图；能在 Run 上看到「这次 miss 是 compact 还是注入块」即可。

**验收**：用夹具模拟「system 变长 vs 只追加 tool」两条请求，explain 能区分 `system_resize` 与 `none`；真实 provider 有 `cached_tokens` 时写入 session/usage，缺字段时 `break_kind=unknown` 且不谎报命中。

### Phase S — 未来注入的护栏

任何新的「给模型看的目录 / 代码索引 / RAG / 附件清单」必须声明 zone：`FrozenPrefix` 或 `TailDecorations`。默认拒绝 `BodyInsert`。文档索引类内容若放进 FrozenPrefix，必须有 **最大长度**（超出则截断或降级为 tool），禁止无界 resize。

这项可以是 `project()` 的 debug assert + 一条 CONTRIBUTING / 本文件的约定，不必先做新存储格式。

---

## 5. 和现有分层的关系

```text
L0 Session Events     事实；compact 追加 CompactionApplied，不删旧事件
L1 Active Branch      当前叶到根
L2 Context Plan       本文件新增：三区布局 + 谁允许改 PrefixZone
L3 Provider Request   字节前缀稳定性在这里兑现；epoch 只在 PrefixZone 变时 ++
```

[agent-inference-context.md](./agent-inference-context.md) 的三档收益仍然成立：

- **A. 仅 Agent**：full messages + 稳定前缀 → 厂商 prefix cache 的大头。**本文优先兑现 A。**
- **B. 网关 delta**：拼出与 A 相同的 full prompt 才有意义；前缀抖动时 delta 也救不了 cache。
- **C. 引擎 KV 续算**：依赖 `(session_id, epoch)`；PrefixZone 乱跳会把续算打成反复 full prefill。

A 档做不稳，B/C 都是空转。

Water level / auto-compact 继续用 `max(usage, estimate)`。Cache 命中变好之后，**计费 token** 会下降，但 `prompt_tokens` 窗口水位不变；不要因为 `cached_tokens` 高就推迟 compact。Compact 触发仍看窗口占用，不看 cache 命中率。

---

## 6. 非目标

- 新的 compaction 阶段或替换 summarize 模型。
- 为对齐某张诊断图而改 Session 存储格式。
- 把 permission / MCP / Turn 控制面迁进 ContextEngine。
- 第一版就做 Console 全彩 cache 条形图（Phase R 先字段，图随后）。
- 假设所有 BYOK 供应商都返回 `cached_tokens`（缺失时降级，不阻塞投影）。

---

## 7. 建议实现落点

| 变更 | 主要位置 |
|------|----------|
| 三区布局 + 断言 | `crates/context`：`project` / `assemble` / `ProjectionExplain` |
| Plan reminder 移出 system | `crates/core/src/plan_mode.rs`；投影注入放 `zene-context` 或 core `context_hooks` |
| Overflow 少改旧 body | `crates/context/src/compaction.rs`（`apply_overflow_truncate_pass` 策略） |
| Memory 不回写 system | `crates/context` memory 注入 + `AgentBuilder` 开工路径 |
| `cached_tokens` 进 explain / 事件 | `ContextEngine::record_step_usage` → RuntimeEvent / ACP `projection_update` |
| 文档约定 | 本文 Phase S；新注入点 code review 对照三区 |

`zene-core` 仍是 composition root。布局契约属于 `zene-context`，避免只在 Agent 里约定、第三方 runtime 绕过。

---

## 8. 一句话

> **Compaction 负责窗口装得下；前缀稳定负责装进去的东西能被 cache 住。**  
> 可变块只许冻结在开工 system 里，或挂在请求尾巴上——不许再放进历史中间。

---

## 相关文档

- [session-as-source-of-truth.md](./session-as-source-of-truth.md) — Session 事实 vs Context 投影
- [context-engine.md](./context-engine.md) — ContextEngine API、epoch、delta、publish
- [context-engine-projection.md](./context-engine-projection.md) — observe/commit/project 与 Phase D 注入规则
- [agent-inference-context.md](./agent-inference-context.md) — 与推理层的 session / cache / 续算
- [ENGINE.md](./ENGINE.md) — turn、compaction、memory、sandbox 行为
- [agent-runtime-optimization.md](./agent-runtime-optimization.md) — 控制面；本文不替代

---

## 讨论记录

### 2026-08-13 — 初稿

参与：用户 ↔ Agent（本仓库 Cloud Agent）

要点：

- 以 DeepSeek-V4-Pro 一次 11 call 的 context 组成 + prefix cache 诊断为动机：注入块 698 tok 导致约 52k 未变更 token 重算。
- 确认现有 SoT / epoch / memory-in-system / tail reminder 方向正确，但 **布局尚未成为硬契约**。
- 优化顺序定为：投影三区 → 拆除 Plan/overflow/memory 的前缀改写 → `cached_tokens` 可观测 → 未来注入护栏。
- 明确 compact 仍是合法的一次 `epoch++`；意外 resize 才是要消灭的 miss。
