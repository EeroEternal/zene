# Context 架构现状评估与优化计划

本文档描述 Zene 当前上下文架构的代码事实、实现边界与优化顺序。它用于约束后续实现，不把设计目标写成已经具备的能力。

相关文档：

- [ContextEngine](./context-engine.md)
- [Session as Source of Truth](./session-as-source-of-truth.md)
- [Agent 与推理服务的 Session 联动](./agent-inference-context.md)
- [Context 优化计划](./context-optimization-plan.md)

## 结论

Zene 当前最有辨识度的架构，是 Agent 持有语义状态，ContextEngine 将状态投影成前缀缓存友好的 Provider 请求，推理网关通过显式版本协议保存 canonical prefix 并拼装 delta。这个分工已经具备可运行的 A/B 两档实现，但还没有形成推理引擎 KV 续算。

代码已经验证了以下主干：

- Session event 投影优先于 `SessionRecord.messages` 兼容缓存。
- `prepare_step` 按 `observe → commit → project` 执行。
- Provider 请求按 Frozen Prefix、append-oriented Body、Tail Decorations 组织。
- `ContextMetadata` 携带 session、epoch、prefix hash、delivery 和 tail start。
- inference-gateway 能 publish prefix，并把 delta 组装成完整请求。
- compact 后 Agent 提升 epoch 并发布新 prefix。
- window water 使用真实 usage 与本地 estimate 的较大值，不以 cache 命中率替代容量判断。

这些能力说明 Zene 已经实现“语义状态与模型视图分离”和“缓存导向的投影协议”。仓库本身不能证明该设计已经领先所有常见 coding agent；竞争差异需要跨产品实现与运行指标支持。

## 当前职责边界

| 层 | 当前职责 | 实现状态 |
|----|----------|----------|
| `zene-session` | 保存 events、兼容 messages cache、checkpoint、todos，并生成 active view | 主路径可用，仍有 legacy fallback 与双写 |
| `zene-context` | estimate、compact、memory、三区投影、epoch、full/delta、explain | 已实现 |
| `zene-llm` | 将 `ContextMetadata` 放入 OpenAI-compatible 请求 | 已实现；不是所有 provider 路径都具备相同协议 |
| inference-gateway | 保存 canonical prefix、校验版本、拼装 delta、代理上游 | 已实现；属于消息级 prefix 缓存层（设计文档称 Warm tier） |
| 推理引擎 | Provider prompt cache 或未来按 session 续 KV | Provider cache 可用；自研 KV 续算未实现 |
| Cloud API / Worker / ACP | 控制面、run 调度、stdio JSON-RPC、Agent 进程环境 | 已实现；Cloud API 不代理 LLM 推理 |

## 已实现的关键不变量

### Session 与投影分离

`SessionRecord.events` 是新集成的投影来源，`messages` 是 materialized compatibility cache。严格路径通过 `try_view()` 拒绝不可恢复的 legacy event log；宽松 `view()` 仍可在 legacy 场景回退到缓存。

事件日志按单调 `sequence` 追加。active view 使用 fork 与 rewind 的 sequence 区间过滤，不是带 `parent_id` 的同一 Session 内事件树。fork 会创建新的 Session，复制已有 events，再追加 `BranchForked`。

`CompactionApplied` 不物理删除旧 events，但其 `messages_after` 会替换后续模型投影中的已累积 messages。因此需要区分：

- raw event audit：保留 compact 前事件；
- active conversation projection：应用 branch、rewind 和 compaction 快照；
- model context projection：继续执行截断、尾部装饰和 full/delta 交付。

### 缓存导向的三区布局

当前投影布局为：

```text
[Frozen Prefix] [Append-oriented Body] [Tail Decorations]
```

Frozen Prefix 包含稳定 system 与 pinned compaction summary。对话和工具结果进入 Body。plan、todos 与运行中后台任务通过 hooks 进入 Tail Decorations。`InjectionZone` 不提供 BodyInsert，紧贴 prefix 的 reminder 会被移动到尾部。

这个布局直接服务于 Provider prefix cache：session id、epoch 和 hash 只能描述版本，不能替代请求左侧字节稳定性。

Memory 当前在 Agent 构建时并入 system prefix，memory flush 在 compact 路径更新持久化内容。当前 hooks 没有“每步 memory 更新进入 tail”的实现。

### full/delta 协议

OpenAI-compatible 路径定义以下元数据：

| 字段 | 当前语义 |
|------|----------|
| `session_id` | 关联同一 run 的模型调用 |
| `context_epoch` | canonical prefix 版本 |
| `prefix_hash` | 已发布 prefix 的指纹 |
| `delivery` | `full` 或 `delta` |
| `tail_start` | delta 对应的消息切分位置 |

启用 inference-gateway 时，delta 需通过 `ZENE_CONTEXT_DELIVERY=delta` 显式开启（v0.1.14 起，网关 URL 单独存在不再默认启用 delta——能力协商未落地前，不能假设网关具备按 session 重建全量 prompt 的能力）。Agent 先向 `/v1/zene/sessions/{id}/publish` 发送 canonical prefix（payload 含 epoch、messages、pinned_boundary、anchor_boundaries），再由网关的 session middleware 组装 `prefix || tail`。没有新增 tail 或无法形成有效 delta 时，Agent 会回退到 full。当前 publish 不等待可反馈给 ContextEngine 的确认，故障闭环属于下文明确列出的缺口。

网关保存 prefix 的消息副本，但不拥有 compaction、memory、branch 或 transcript 语义。它不是第二个 Session SoT。

### 能力档位

| 档位 | 当前状态 | 主要收益 |
|------|----------|----------|
| A：Agent full | 已实现且是无网关时默认路径 | 稳定字节前缀带来的 Provider prompt cache |
| B：Agent + gateway | 已实现，需部署配置启用 | delta 省带宽、统一拼装、epoch/hash 校验 |
| C：Agent + gateway + KV-aware engine | 未实现 | 只 prefill 新 token，并管理 session KV 生命周期 |

Session ID 本身不会打开 prompt cache。A/B 两档的缓存收益仍取决于组装后的完整 prompt 是否保持相同字节前缀。

## 当前缺口

### Session SoT 尚未完全收口

`events` 在类型注释中仍被称为 future SoT。`messages`、todos、compactions 和部分 checkpoint 状态仍存在双写。legacy compaction 或 rewind 缺少 snapshot 时，宽松投影仍依赖 materialized cache。

“commit 是唯一 Conversation SoT 写入口”只适用于 `prepare_step` 内的上下文提交语义。普通 message、turn 与 tool 事件由 Agent 循环的其他入口追加。

Todos 目前保存在 `SessionRecord.todos`，并通过 tail hooks 投影，不属于 conversation event log。这会限制统一 replay 与事件审计。

### Transcript 与模型视图仍容易混淆

raw events 保留 compact 前事实，但默认 `SessionView.messages` 会应用 compaction snapshot。调用方若把 `SessionView.messages` 当作完整可审计 transcript，会丢失 compact 前的默认展示。

现有 API 需要显式区分审计 transcript、active conversation 和 model context，避免调用方依赖隐含投影语义。

### Gateway 交付状态缺少强一致 fallback

ContextEngine 会在 emit publish 前更新 `initial_publish_done`、`pending_publish` 和 `gateway_prefix_len`。Agent 侧 `publish_prefix` 当前只记录 HTTP 失败并返回，不把失败反馈给 ContextEngine。publish 超时、冲突或不可用时，Agent 仍可能认为 prefix 已发布。

协议需要保证：只有 publish 得到确认后才能发送依赖该版本的 delta；否则必须发送 full，直到同一 epoch 的 publish 成功。

### Cache explain 尚未当步闭环

`ProjectionExplain.cached_tokens` 使用前一次 Provider 调用的 usage，`unchanged_reprocessed_est` 使用 frozen prefix 字符数除以四的估算。它们适合诊断趋势，不是 Provider 对当前投影的精确归因。v0.1.14 起 `PrefixCacheExplain` 并列记录网关账本命中（`gateway_hit_tokens`）与 provider 实际命中（`cached_tokens`），但两数仍是上一轮回填，当步归因缺口不变。

`PrefixCacheBreakKind::BodyMutate` 已定义，但当前分类路径没有产生该值。Console 也尚未展示 typed prefix-cache 数据。

文档中的 DeepSeek resize 数字是诊断记录，仓库没有原始 trace 或可运行 benchmark。该数字可以说明风险，不应作为当前实现效果的验证结果。

### 协议覆盖和测试仍有限

现有 E2E 覆盖 publish prefix 后 delta 请求在上游组装成 `prefix + tail`。仍缺少 compact 后 epoch 提升与 republish、stale epoch、hash mismatch、publish 不可用、Redis 多实例和 SmartGate 全链路验证。

当前 session 元数据协议主要位于 OpenAI-compatible 路径。Provider 能力差异需要显式建模，不能假设所有 provider 都接受同一套 header 与 gateway 行为。

`on_system_prefix_changed` 已提供 epoch 提升入口，但 runtime 中的 system/memory 变更尚未全部统一经过该入口。动态 prefix 修改需要与 epoch 和 republish 建立同一不变量。

## 优化原则

- Agent 始终是唯一语义权威；网关和推理引擎只保存可丢弃的执行缓存。
- 审计 transcript、active conversation 和 model context 使用不同类型或入口。
- delta 是可恢复优化，不能成为正确性前提。
- epoch 只在 canonical prefix 语义改变时提升。
- 可变内容进入尾部；稳定内容进入 prefix 后不得在 step 间隐式变化。
- Select 继续以工具结果进入 Body，不把 RAG 或 repo index 塞入 ContextEngine prefix。
- compact、memory 与 pinned facts 不下放到网关或推理引擎。
- C 档只有在 A/B 指标证明 prefill 仍是主要瓶颈时才进入实现。

## 优化计划

### 必须先完成：收口 Session 事实模型

目标是让所有新 Session 都能仅凭 events 确定性重建，不依赖 `messages`、todos 或 compactions 的独立真相。

实施内容：

- 定义并测试 conversation event schema 的完整不变量。
- 将 `messages` 降为可重建缓存；禁止业务逻辑直接把它作为事实源。
- 为 todos 增加事件，或明确将其定义为独立 runtime state，并禁止文档把它称为 conversation event。
- 为可无损 legacy 数据提供迁移；无法无损迁移的数据保留显式 fallback reason。
- 为 message、tool、compact、rewind、fork、checkpoint 增加从 events 重建的属性测试。

完成标准：

- 新建 Session 的 `try_view()` 永不使用 materialized fallback。
- 人为改坏 `messages` cache 后，严格投影结果不变且能报告 drift。
- fork、rewind 和 compact 组合回放具有确定结果。
- 所有非 event 状态在类型和文档中被明确标为 derived cache 或独立 runtime state。

### 必须先完成：拆分三种 View

目标是消除“SessionView 等于完整 transcript”或“SessionView 等于模型 prompt”的歧义。

实施内容：

- 提供 raw audit transcript、active conversation 与 model context 的独立 API。
- raw audit API 暴露 compact 前事件和 compaction segment 关联。
- active conversation API 只处理 branch、rewind、compaction 的语义投影。
- model context 继续由 ContextEngine 负责 token cut、三区布局和 delivery。
- UI、replay、export 只依赖 transcript/active API，不依赖 ContextEngine。

完成标准：

- API 名称和返回类型能阻止三种 view 被互换使用。
- compact 后 UI/export 仍可访问 compact 前审计事实。
- Provider 请求只能由 model context API 产生。

### 必须先完成：让 delta 具备失败闭环

目标是任何 publish 或 session store 故障都只降低性能，不影响请求正确性。

实施内容：

- 让 `publish_prefix` 返回结构化结果，不再吞掉 HTTP 错误。
- 仅在 publish ack 后更新 `gateway_prefix_len` 与 publish 状态。
- 对 timeout、409、404、413、503 定义 retry、republish 或 full fallback 行为。
- delta 请求遭遇 epoch/hash/tail mismatch 时，在同一语义请求上执行有界 full retry。
- 增加 gateway capability/health 状态，禁止仅靠环境变量假定 session middleware 可用。

完成标准：

- publish 失败后上游仍能收到完整且顺序正确的 messages。
- stale epoch 或 hash mismatch 不会拼接错误 prefix。
- gateway 恢复后可重新 publish，并安全返回 delta。
- 故障测试能证明 delta 不是正确性依赖。

### 随后完成：闭环 Prefix Cache 观测

目标是把每次投影、请求和 Provider usage 关联为同一个可查询事实。

实施内容：

- 为 projection/request 增加稳定 step id，将当次 `cached_tokens` 回填到对应投影。
- 同时记录 frozen prefix token estimate、Provider prompt tokens、cached tokens 和 delivery bytes。
- 完成 `body_mutate` 分类，区分 compact、system resize、injected resize 与未知 break。
- 将 typed `prefixCache` 数据沿 ACP、Cloud domain 和 Console 展示。
- 将 DeepSeek 诊断方法固化为可重复 trace 分析，避免只保留结论数字。

完成标准：

- Console 中每次 LLM call 都能看到对应 epoch、hash、break kind、prompt tokens 与 cached tokens。
- explain 不再把前一次调用的 usage 显示成当前调用结果。
- 能从 trace 判断 cache miss 来自合法 compact、system resize、body mutation 或 Provider 未命中。

### 随后完成：补齐协议 E2E 矩阵

目标是用跨 crate 测试锁住 Agent 与 gateway 的版本契约。

测试矩阵至少包括：

- initial publish → delta → upstream full；
- compact → epoch++ → republish → 新 epoch delta；
- system prefix change → epoch++ → republish；
- stale epoch、hash mismatch、tail start mismatch；
- publish timeout、store unavailable、prefix too large；
- full fallback 与恢复后的 delta；
- memory store 与 Redis store；
- generic OpenAI-compatible 与 SmartGate metadata 映射；
- Run 结束 delete session。

完成标准：

- 每一种协议错误都有确定的状态码、Agent 行为和观测事件。
- 网关重启或 session eviction 不会导致错误 prompt。
- E2E 校验上游收到的最终 messages，而不只校验中间 metadata。

### 条件触发：评估 KV-aware engine

C 档不是 A/B 的正确性补丁。只有运行指标显示长 tail prefill 在稳定 prefix cache 后仍占主要成本，才进入引擎选型。

进入条件：

- A/B 已具备可靠的当步 cache 观测。
- 已量化 Agent→gateway 带宽、Provider cached token、prefill latency 与 session 长度分布。
- 已验证现有 Provider prefix cache 无法满足目标负载。

若进入实现，协议至少需要：

- `(session_id, epoch)` 级 KV 生命周期；
- delta continuation 与 full recompute 等价性验证；
- session affinity、TTL、eviction 和 OOM 策略；
- compact 后旧 epoch 主动失效；
- Run 结束释放；
- miss 或 eviction 后安全 full prefill。

## 推荐实施顺序

```text
Session SoT 收口
  → 三种 View 拆分
  → delta 失败闭环
  → 当步 cache 观测
  → 协议 E2E 矩阵
  → 基于指标决定是否实现 KV-aware engine
```

前三项解决语义正确性和故障安全，之后的观测与测试用于证明优化收益。KV 续算保持为条件能力，避免在消息投影和网关协议尚未完全闭环时引入第三份状态。
