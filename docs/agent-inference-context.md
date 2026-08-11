# Agent ↔ 推理服务：Session 联动与上下文管理

本文档用于持续讨论：Zene（Agent 侧）调用后台模型 / 推理服务时，如何用 session 关联一轮对话，并与推理层联动做上下文管理，从而提升 Agent 效率。

相关已有实现见 [ENGINE.md](./ENGINE.md)（compaction、memory、context water level 等）。上下文解耦设计见 [context-engine.md](./context-engine.md)。

---

## 背景与问题

### 当前机制（截至文档创建时）

- Cloud Console 前台通过 **HTTP** 与 `zene-cloud-api` 通信。
- Worker 拉起 **`zene acp` 子进程**，双方用 **ACP（stdio 上的 JSON-RPC）** 通信。
- `zene` 调外部模型是 **BYOK + 出站 HTTPS**（OpenAI 兼容 / Anthropic），密钥与 `base_url` 等由 Settings 或环境变量 / `~/.zene/config.toml` 注入。
- Zene / ACP / Cloud run **内部有 session / run id**；**P1.5 客户端**（`X-Zene-*` header、epoch、compact 后 publish）见实现 PR #42；自研 **推理引擎 session 续算** 仍为本文档「推理引擎备注」中的后续阶段。

### 为什么需要 Session ID

同一轮 Agent 对话里会有多次模型调用（多 turn、tool 后续写、重试、compaction 旁路 summarize 等）。若推理层能拿到稳定的 session id，就可以：

- 把这些调用在服务侧串成一条会话；
- 做会话级限流、审计、路由亲和；
- 为 **KV / prompt cache**、会话级观测提供关联键；
- 支撑后续「Agent ↔ 推理层」的上下文联动（见下）。

**注意**：Session ID 本身不管理上下文；它只是关联键。效率来自少传、传对、前缀稳定，以及两边约定的状态协议。

---

## 职责划分（建议）

| 侧 | 职责 |
|----|------|
| Agent（Zene） | 语义状态权威：完整可恢复 transcript、compaction、memory、pinned 事实、何时 compact |
| 推理服务 | 执行与缓存：会话亲和、KV/prompt cache、token 用量回传、（可选）大附件存储 |
| Session 协议 | 用 id + epoch 等字段把两边对齐，避免各维护一份互相漂移的「完整历史」 |

原则：**不要两边各管一份完整对话历史**；Agent 管语义，推理层管执行缓存。

---

## 联动方案（讨论中的点子）

### 1. `session_id` + `context_epoch`（或 `prefix_hash`）

每次出站请求携带：

- `session_id`：ACP session / Cloud `run_id`（或二者映射后的稳定 id）
- `context_epoch` 或 `prefix_hash`：表示当前「可缓存前缀」的版本

规则建议：

- system / memory 注入 / compaction 摘要替换等导致前缀语义变化时，`epoch++`（或 hash 变更）；
- epoch 未变 → 网关可安全复用 KV / prompt cache；
- epoch 变了 → 主动作废旧前缀缓存，避免命中过期上下文。

### 2. Delta / 续写（若网关支持）

- Agent 不必每次重放全文，只传 **delta**（新 user 消息、新 tool result）+ 必要的 pinned 块；
- 完整可恢复 transcript 仍留在 Zene（session 持久化）；
- 推理层按 `session_id` 维护软状态；Agent compact 时通过 epoch 通知失效并下发新前缀。

### 3. 用量闭环

- 推理层回传 `prompt_tokens` / `cached_tokens`（及可选 cache hit 标记）；
- 喂给现有 `ContextWaterLevel` / compaction 触发，比纯本地启发式更准；
- 便于评估联动收益（cache 命中率、prefill 成本）。

### 4. 大工具输出卸载

- 超长 tool result 先落对象存储或会话附件；
- 请求里只传句柄 + 短摘要；
- 两边约定哪些字段是 `pinned`（用户约束、当前计划、关键文件结论），compaction 与网关淘汰都不得丢。

### 5. 与现有 Zene 能力对齐

已有（见 ENGINE.md），联动时应复用而非重做：

- 分阶段 compaction（截断 tool result → slice → LLM summarize）
- two-pass / prefire
- `.zene/memory` 抽取与 `<memory-context>` 注入（有利于稳定前缀）
- 会话持久化与 rewind / fork

联动增量主要是：**出站带 id/epoch、吃回 cache/usage、compact 时失效缓存**。

---

## 建议的最小落地路径

1. **透传 id**：`zene-llm` 出站增加可配置 header/body 字段（如 `X-Session-Id` / `session_id`），值来自 ACP session 或 Cloud run。
2. **加 epoch**：compaction 或 memory 前缀变更时递增，一并传出。
3. **观测**：记录 `cached_tokens` / cache hit，对照 compaction 前后成本。
4. **再谈 delta API**：确认网关能力后再做续写协议，避免过早绑定。

具体字段名与网关约定待定，需与推理服务接口对齐。

---

## 推理引擎备注（Session 续算与业界趋势）

本节记录 **Inference 层** 的讨论结论：与网关协议（Warm tier）的关系、何时需要引擎改造、以及 vLLM / SGLang 等方向。**Compact 始终在 Agent（Zene）执行**，引擎只响应 `epoch++` 做 KV 失效与续算，不做 summarize。

### 三档收益模型

| 档位 | 谁控什么 | Prompt cache | Session 续算（KV 复用） |
|------|----------|--------------|-------------------------|
| **A. 仅 Agent** | Agent 发 **full messages**，prefix 稳定 | ✅ 可拿到大部分收益 | ❌ 无（inference 无状态） |
| **B. Agent + 自研网关** | 网关 Warm：prefix + tail 组装、delta 上行 | ✅ 与 A 类似（拼出相同 prefix） | ❌ inference 仍黑盒 |
| **C. Agent + 网关 + 自研引擎** | 引擎存 `(session_id, epoch)` 的 KV | ✅ | ✅ 上限最高 |

**Session ID 不直接激活 prompt cache。** Cache 只认「拼出来的 prompt 前缀是否字节级相同」。Session ID 的价值在 B/C 档：**关联网关/引擎状态**、delta 组装、compact 后 `epoch++` 对齐、按 run 观测。

**不做网关、全在 Agent**：只要 stable prefix + 每次 full messages，**prompt cache 大头照样有**；少的是 delta 省带宽、网关侧会话态与运维视图。

### Prompt cache vs Session 续算（举例）

固定 `system+memory` ≈ 2000 token，后面是对话 tail。

- **第 2 轮**：prompt = prefix + 第 1 轮对话。若 prefix 与上次一致，inference 可能对前 ~2500 token 报 `cached_tokens`，**无需 session id**。
- **有网关 + session id**：Agent 只发 delta；网关拼成与上面 **相同的 full prompt** 再调 inference → cache 行为与 Agent 直发 full **几乎一样**；省的是 Agent→网关 流量与拼包一致性。
- **有自研引擎续算**：引擎记住上一轮 KV，只对 **新增 token** prefill → 比 prompt cache 再省一截（尤其 tail 很长时）。

**Compact 后**：Agent 本地 summarize → `epoch++` → `publish` 新 prefix。Prompt cache **必然 miss 一轮**（前缀变了）；引擎侧须 **invalidate(session_id, old_epoch)**，再对新 prefix full prefill 建立新 epoch 的 KV。

### 引擎支持 Session 续算：需要的工作

1. **会话状态**：按 `(session_id, epoch)` 存 KV blocks / paged cache；Run 结束 `DELETE`；epoch 变更整份作废。
2. **API**：除标准 chat 外，支持 `session_id` + `epoch` + **delta**（或 `continue_from_seq_len`）；命中则 load KV → prefill delta → append → decode；miss 则 full prefill 并建档。
3. **流水线**：attention 层接上已有 KV，而非每请求从 0 forward（或接引擎已有 prefix/session 扩展）。
4. **调度亲和**：同 `session_id` 尽量固定 GPU/worker；多副本需分布式 KV 池（见 vLLM Mooncake）或 sticky routing。
5. **资源**：单 session 长度上限、idle TTL、OOM evict；与网关 `DELETE session`、`epoch` invalidate 联动。
6. **分工**：Compact / memory **只在 Agent**；引擎只听 epoch 失效，不替 Agent 做语义压缩。
7. **验证**：续算输出与 full 重算一致；epoch 变后不可误用旧 KV。

网关职责：拼 delta、调引擎 session API、校验 epoch。Agent 职责：compact、epoch、`publish`（已实现方向见 PR #42）。

### 业界趋势：SGLang

- **RadixAttention / RadixCache**（基础）：radix tree 复用 prefix KV，多轮 / 多请求共享前缀；Agent 场景的主战场之一。
- **Session radix cache**（`--enable-session-radix-cache`，PR 如 [#27058](https://github.com/sgl-project/sglang/pull/27058)）：请求带 `session_id`，KV 在 radix 上 **打 tag**；`close_session` 扫描释放该 session 的链；KV 仍为 **可 evict 的普通 radix 节点**（非永久 pin）。
- **UnifiedRadixCache + session 引用**（PR 如 [#29173](https://github.com/sgl-project/sglang/pull/29173)）：面向 agent 多轮，session 引用计数、关闭 session 释放引用、unreferenced-first eviction。
- **StreamingSession**（较早路径）：turn 间 KV 留在 slot，`match_prefix` 续上；与并发 evict 的平衡仍在演进（RFC [#29099](https://github.com/sgl-project/sglang/issues/29099) 讨论 session 级抢占）。

**务实结论**：若短期要较高收益且能换引擎，**SGLang + session radix + 上层 epoch 失效** 往往比从零自研 KV 管理更快。

### 业界趋势：vLLM

- **已有**：Automatic Prefix Caching（同前缀少 prefill），对应本文档 **A/B 档** 的主体收益。
- **Agent 向 RFC / 集成（演进中，非开箱即用续算 API）**：
  - [#37003](https://github.com/vllm-project/vllm/issues/37003) **RetentionDirective**：tool 暂停期间提高 session KV 保留优先级，减轻 LRU 误 evict（长 pause + 大 context 收益更明显）。
  - [#37168](https://github.com/vllm-project/vllm/issues/37168) **`POST /release_kv_cache`**、session 感知引用计数、双区调度（Aging/Fresh）——与 Agent **`epoch++` 主动失效** 高度相关。
  - [#48501](https://github.com/vllm-project/vllm/issues/48501) **`session_id` + `continuation_id`**：session 作用域 + 内容链位置，供控制面做 KV orchestration。
  - [agentic-api Session Cache Manager](https://github.com/vllm-project/agentic-api/issues/18)：编排层维护 session↔KV 映射。
  - **Mooncake Store**（[vLLM 博客 2026](https://vllm.ai/blog/2026-05-06-mooncake-store)）：跨节点分布式 KV 池，缓解「session 被路由到其他实例 → cache miss」。

**务实结论**：vLLM 路线适合 **prefix cache + Mooncake + 等 RFC 落地（release_kv_cache / retention）**；完整 session 续算仍在 RFC/生态拼装阶段，工程要求 **高于** 仅用 prompt cache。

### 与 Zene 协议（session_id + epoch）的对接

| Zene 事件 | 网关 | 自研引擎（若启用续算） |
|-----------|------|------------------------|
| 常态 step | delta 组装 → full prompt 调 inference | load `(session_id, epoch)` KV，prefill delta |
| compact 完成 | `epoch++`，`publish`，清 tail | `invalidate(session_id, old_epoch)` 或 `release_kv_cache` |
| Run 结束 | `DELETE /v1/zene/sessions/{id}` | 释放 session KV / `close_session` |

`epoch` = 「第几版 canonical prefix」；`session_id` = 「哪条 Run/对话」。引擎侧建议与 **`(session_id, epoch)`** 联合寻址，避免 compact 后误续旧 KV。

### 开放问题（引擎侧，追加）

- Zene Cloud 首选引擎：SGLang session radix vs vLLM prefix cache + Mooncake？
- `release_kv_cache` / SGLang `close_session` 由 Worker 在 Run 结束时调，还是网关代理？
- 续算 API 是否标准化为 OpenAI 扩展字段，还是内网 gRPC？

---

## 开放问题

- Session 主键用 ACP `sessionId`、Cloud `run_id`，还是独立生成并双向映射？
- 字段放在 HTTP header 还是 JSON body（`metadata` / `extra`）？不同厂商网关兼容性如何？
- Compaction 旁路 summarize、subagent 调用是否共用父 session，还是子 session + parent 链接？
- 推理层是否持久化会话状态，还是仅进程内 / TTL 缓存？
- 与 prompt cache 计费模型如何对齐（什么样的前缀变更必须 bump epoch）？

---

## 讨论记录

按时间追加。结论变更时同步改上方正文，并在条目里留一句指针即可。

### 2026-08-09 — 初稿（对话整理）

参与：用户 ↔ Agent（本仓库 Cloud Agent）

要点：

1. 澄清架构：前台 HTTP → Cloud API；Worker ↔ `zene acp` 为 ACP stdio；模型为 BYOK HTTPS。
2. 现状：内部有 session，**未**传给模型服务。
3. 诉求：带 session id，便于推理层把一轮对话的多次调用关联起来，核心是上下文管理、提升 Agent 效率。
4. 建议分层：Agent 做语义上下文（compaction/memory/稳定前缀），推理层做会话亲和与 cache；id 是关联键。
5. 联动加强版：`session_id` + `context_epoch`/`prefix_hash`、delta 续写、usage/cache 回传闭环、大 tool 输出卸载 + pinned 约定。
6. 约定：后续讨论继续写在本文档「讨论记录」中。

### 2026-08-09 — Session ID、prompt cache、引擎续算

参与：用户 ↔ Agent

要点：

1. **Session ID 不直接等于 prompt cache**；cache 认 prefix 字节一致。ID 用于网关/引擎状态与 delta 组装，间接利于 prefix 稳定。
2. **仅 Agent、不做网关**：stable prefix + full messages 仍可拿到 prompt cache 大部分收益。
3. **网关 + 自研引擎**：可在引擎做 **KV 续算**（只算 delta），上限高于 prompt cache；需 session store、API、亲和调度、epoch 失效。
4. **Compact 在 Agent**；`epoch++` 后引擎须 invalidate 旧 KV。
5. **SGLang**：RadixCache、session radix cache、UnifiedRadixCache 等，Agent 向产品化较快。
6. **vLLM**：prefix caching 已成熟；Agent 向 RFC（RetentionDirective、release_kv_cache、Mooncake、session_id/continuation_id）演进中。
7. 详见上文「推理引擎备注（Session 续算与业界趋势）」。

### （在此追加下一次讨论）
