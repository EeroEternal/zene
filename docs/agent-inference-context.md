# Agent ↔ 推理服务：Session 联动与上下文管理

本文档用于持续讨论：Zene（Agent 侧）调用后台模型 / 推理服务时，如何用 session 关联一轮对话，并与推理层联动做上下文管理，从而提升 Agent 效率。

相关已有实现见 [ENGINE.md](./ENGINE.md)（compaction、memory、context water level 等）。

---

## 背景与问题

### 当前机制（截至文档创建时）

- Cloud Console 前台通过 **HTTP** 与 `zene-cloud-api` 通信。
- Worker 拉起 **`zene acp` 子进程**，双方用 **ACP（stdio 上的 JSON-RPC）** 通信。
- `zene` 调外部模型是 **BYOK + 出站 HTTPS**（OpenAI 兼容 / Anthropic），密钥与 `base_url` 等由 Settings 或环境变量 / `~/.zene/config.toml` 注入。
- Zene / ACP / Cloud run **内部有 session / run id**，但出站 chat 请求里 **尚未把 session id 传给推理服务**（`ProxyChatRequest` 的 `extra` / `metadata` 目前为空；Anthropic 路径也只有常规鉴权头）。

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

### （在此追加下一次讨论）
