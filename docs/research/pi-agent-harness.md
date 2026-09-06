# Pi Agent Harness 对 Zene 的启发

研究范围：Pi（earendil-works/pi）v0.84.1 及配套文档 / 源码结构。  
**只做架构对照与产品取舍，不要求照搬实现或文件格式。**

相关 Zene 文档：

- [session-as-source-of-truth.md](../session-as-source-of-truth.md) — Session 事实 vs Context 投影
- [context-engine.md](../context-engine.md) — ContextEngine（含索引 Select 契约）
- [agent-components.md](../agent-components.md) — 可组装组件栈
- [ENGINE.md](../ENGINE.md) — turn / compaction / ACP 行为

外部参考：

- [Pi repository](https://github.com/earendil-works/pi)
- [v0.84.1 release](https://github.com/earendil-works/pi/releases/tag/v0.84.1)
- [Coding agent README](https://github.com/earendil-works/pi/blob/v0.84.1/packages/coding-agent/README.md)
- [Agent core](https://github.com/earendil-works/pi/blob/v0.84.1/packages/agent/README.md)
- [Session format](https://github.com/earendil-works/pi/blob/v0.84.1/packages/coding-agent/docs/session-format.md)
- [Compaction](https://github.com/earendil-works/pi/blob/v0.84.1/packages/coding-agent/docs/compaction.md)
- [Extensions](https://github.com/earendil-works/pi/blob/v0.84.1/packages/coding-agent/docs/extensions.md)
- [Design rationale](https://mariozechner.at/posts/2025-11-30-pi-coding-agent/)

---

## 1. 一句话结论

Pi 最值得学的不是某个具体工具或 TUI 技巧，而是 **Agent Harness 边界**：

> **核心只负责稳定的 agent loop、消息状态、工具调用和事件流；工作流能力通过 extensions、skills、packages 和外部 runtime 组合。**

对 Zene 的五个主启发：

1. 把 **Agent 运行时** 与 **产品工作流** 分得更清楚  
2. 把 **Session 设计成可回放、可分支、可后处理的事件树**（事实源）  
3. 把 **上下文投影、压缩、工具执行、中止** 做成明确稳定契约  
4. 让非核心能力具备 **可安装、可组合、可版本化** 的扩展机制  
5. 提升 **可观察性**：用户和上层 UI 应能知道模型到底看到了什么  

Zene 在组件拆分、Context Engine、Turn Runtime、Sandbox、Permission、ACP 上已经更「平台化」；Pi 提供的是清晰的 **最小核心 + 可扩展外围** 参考模型。

---

## 2. Pi 是什么

TypeScript monorepo，主要包：

| 包 | 作用 |
|----|------|
| `pi-ai` | 多 Provider LLM API、流式、tool calling、模型发现 |
| `pi-agent-core` | 通用 Agent Loop、状态、事件流 |
| `pi-coding-agent` | 编码 CLI、工具、Session、Compaction、TUI |
| `pi-tui` | 终端 UI / 差分渲染 |
| `pi-client` / `pi-protocol` | 远程 session / 进程集成 |
| `pi-telemetry` | 厂商无关 telemetry contract |
| `pi-session-backend-*` | 可替换 session 持久化 |

运行模式：Interactive TUI、Print/JSON、RPC、SDK 嵌入。

定位：**可重组的 agent harness**，不是功能最全的 IDE Agent。

---

## 3. v0.84.1 里值得单独记的点

该版本不是大重构，但体现工程取向。

### 3.1 `pi auth check`

运行前检查 provider / model 凭证是否就绪，可输出解析后的 credential。  
比「第一次 LLM 请求才失败」更友好。

**Zene 可吸收为 readiness：**

- provider / model
- sandbox / permission
- MCP server
- tool availability
- workspace trust

适合 Cloud session 创建前的结构化诊断。

### 3.2 Tool `terminate` 语义

Extension `tool_call` 可 block，并带 `terminate`：在整批 tool result 都要求 terminate 时，跳过自动 follow-up LLM 调用；**混合批次仍继续**。

**应显式区分：**

| 语义 | 含义 |
|------|------|
| `block` | 当前工具不执行 |
| `is_error` | 向模型返回工具错误 |
| `terminate` | loop 不再自动发起下一次 LLM |
| `cancel` | 外部中断整次运行 |
| `retry` | 失败后可重试 |

「工具被拒绝」≠「整个 Agent 结束」。

### 3.3 `Agent.reset()` idle guard

运行中 reset 应拒绝，避免清掉 transcript / runtime state。  
推广：model switch、tool registry 变更、extension reload、fork 等也应有生命周期约束（idle-only / next-turn / cancel-then-apply）。

---

## 4. 架构要点与 Zene 对照

### 4.1 消息流分层

Pi：

```text
AgentMessage[]
  → transformContext()     // prune / inject
  → convertToLlm()         // 过滤 UI-only，转 Provider 格式
  → Message[] → LLM
```

`AgentMessage` 可比 LLM 消息更丰富；转换边界必须清晰。

**Zene：** 已有 `Message`、`ContextEngine`、`ContextSession`、compaction、memory、metadata。  
建议固定三层：

```text
Canonical Session Events
        ↓
Agent Runtime Context
        ↓
Provider Request Context
```

展开与落地：[session-as-source-of-truth.md](../session-as-source-of-truth.md)、[context-engine.md](../context-engine.md)。

### 4.2 生命周期事件

Pi 层次大致：

```text
agent_start
  turn_start
    message_* / tool_execution_*
  turn_end
agent_end
```

外加 `beforeToolCall` / `afterToolCall`、`shouldStopAfterTurn`、`prepareNextTurn`、steering / follow-up。

**Zene：** 已有 `AgentEvent`、step、ACP `session/update` 等。  
可进一步固定：

| 层次 | 关注点 |
|------|--------|
| Agent Run | 一次完整运行 |
| Turn | 一次用户输入对应的多步循环 |
| Step | 一次 LLM + 其工具 |
| Message | 消息生命周期 |
| Tool Execution | 工具生命周期 |

便于 UI、回放、telemetry、多 host（ACP / Cloud / CLI）共享语义。

### 4.3 Steering vs Follow-up

| 类型 | 时机 | 用途 |
|------|------|------|
| Steering | 当前工具批完成后、下次 LLM 前注入 | 纠偏、加约束 |
| Follow-up | 当前 agent 跑完再处理 | 收尾任务、下一阶段工作 |

**Zene：** 已有 `Agent::steer()` / `SteerBuffer`（见 [ENGINE.md](../ENGINE.md)）。  
建议在协议与文档里显式保留两种队列语义，避免 Cloud「执行中继续输入」行为含糊。

### 4.4 并行工具 + 稳定 transcript 顺序

Pi：preflight 顺序校验 → 可并行执行 → 完成事件可按完成序 → **持久化 tool result 按 assistant 源顺序**。

**Zene：** `ToolScheduler` 已做 conflict-aware 并行，并保持返回序。  
需统一对外语义：event completion order vs transcript order vs model-visible order vs cancel/retry。

### 4.5 Session = JSONL 事件树

Pi Session 不是裸 `messages[]`，而是带 `id` / `parentId` 的 entry 树：

- message、model_change、thinking_level_change  
- compaction、branch_summary  
- custom entry（不进 LLM）、custom_message（进 LLM）  
- label、session_info  

支持 in-place branch、fork、tree 导航、回放、后处理。  
Compaction / branch summary **追加 entry**，完整历史仍在文件中；context 由 `buildContextEntries` / `buildSessionContext` **投影**。

**这是对 Zene 价值最大的一点。**  
Zene 已有 `SessionRecord`、checkpoint、compaction segment、fork/rewind、record writer、ACP replay —— 多个投影/旁路并存，但尚未把「完整事件树」明确升为单一事实源。  
心智模型与落地原则见 [session-as-source-of-truth.md](../session-as-source-of-truth.md)。

### 4.6 Compaction + Branch summarization

Pi 两套机制：

1. **Context compaction** — token 阈值或 `/compact`；结构化 summary + file 追踪  
2. **Branch summarization** — `/tree` 换分支时总结离开的路径  

Summary 结构（Goal / Constraints / Progress / Decisions / Next / Critical Context + read/modified files）稳定、可机器读。

**Zene：** 压缩算法更强（truncate/slice/summarize、overflow、prefire、memory、water level 等，见 [ENGINE.md](../ENGINE.md)）。  
应借的是抽象，不是换算法：

- compaction 是 **session 事件**，不是销毁历史  
- branch summary ≠ 普通 compaction  
- summary **schema 稳定且可解释**  

投影侧路线：[context-engine.md](../context-engine.md)。

### 4.7 Extensions / Skills / Packages

Pi 哲学（适合个人 CLI，**不能原样当 Zene 产品默认**）：

- No MCP / No 内置 sub-agent / plan / todo / permission popup / background bash  
- 能力放 extension、skill、package、外部 sandbox、tmux  

Extension API 很宽：tools、commands、shortcuts、providers、renderers、UI、compaction/session/tool hooks。  
Skills 跟 Agent Skills 风格（`SKILL.md` + name/description，按需加载）。  
Packages 可打包 extensions/skills/prompts/themes，npm/git，带 trust 与 pin。

**Zene 不应删 MCP、subagent、plan、todo、permission。**  
应做的是：

> 保留产品能力，但让它们通过清晰的 trait / capability / trust / 版本边界接入，而不是焊死在 core 上帝对象里。

方向与 [agent-components.md](../agent-components.md) 的 composition root 一致。可演进：

| 层次 | 适合内容 |
|------|----------|
| Skill package | prompt、流程、参考文档（`SKILL.md`） |
| Tool package | 工具定义与执行 |
| Runtime extension | turn / context / session hooks |
| MCP package | 外部服务 |
| UI/ACP package | Console 命令与交互 |

先定 **manifest + trust model**，不必立刻做 npm 式包管理器。

### 4.8 Context engineering / 可观察性

Pi 强调：精确控制每次进入模型的内容；session 可检视；token/cache/context 可见。

**Zene：** Context Engine 能力领先；下一杠杆是 **前缀稳定 / prefix cache** 与可解释投影。见 [context-engine.md](../context-engine.md)。

### 4.9 集成表面

Pi：CLI / JSON / RPC / SDK 共 Agent Core。  
Zene：ACP + Cloud worker + Console 更适合远程产品。  
原则相同：**协议可不同，turn / message / tool / permission / context / usage 语义不应分叉。**

---

## 5. 对照表

| 领域 | Pi | Zene | 判断 |
|------|----|------|------|
| LLM | `pi-ai` 多 Provider | `zene-llm` | 方向一致 |
| Loop | `pi-agent-core` | `zene-turn` + `zene-core` | Zene 在拆分 |
| Context | transform + compaction | 独立 `zene-context`，算法更强 | 可观察性 / 投影契约可增强 |
| Tools | hooks + 可并行 | Registry + Permission + conflict scheduler | Zene 更强 |
| Session | JSONL event tree | SessionRecord + checkpoint + segments | **统一事件树最值得学** |
| Extensions | 宽 TS API | traits / hooks / MCP / builder | Zene 更安全，分发较弱 |
| Skills | Agent Skills | Workspace / context 外移中 | 可吸收 `SKILL.md` |
| Permissions | 默认不内置 | Permission + Sandbox 核心能力 | **不要照搬 Pi** |
| MCP | 明确不内置 | 一等集成 | **不要照搬** |
| Sub-agent / Plan / Todo | 不内置 | 已有 | 保持，做成可组合 capability |
| Integration | CLI/RPC/SDK | ACP/Cloud/Console | Zene 更适合产品 |
| TUI | 强项 | 重点在 Console | 局部可参考，非主线 |

---

## 6. 建议优先级（给 Zene）

### P0

0. **统一 ID + RuntimeEvent 信封**（与控制面共用地基）  
   细节与 Wave 顺序：[agent-runtime-optimization.md §16](../archive/agent-runtime-optimization.md#16-merged-implementation-waves)。

1. **统一 Session Event Model**  
   逻辑事件：message、tool、permission、model change、compaction、branch/fork/rewind、checkpoint、custom state…  
   投影：LLM / UI / replay / analytics / export。  
   细节：[session-as-source-of-truth.md](../session-as-source-of-truth.md)。

2. **明确 Context Projection 契约**  
   `observe` / `commit` / `project`；compaction 追加事件；注入物分类。  
   对外 port 名对齐 `ContextAssembler`。  
   细节：[context-engine.md](../context-engine.md)。

3. **固定 Tool Batch 终止协议**（落在 `ToolExecutor`，非 Agent 私货）  
   block / error / terminate / cancel / retry 及批量、顺序、ACP 表达。

### P1

4. **RuntimeHandle 控制面**（cancel/steer/approval 单一状态所有者）  
5. **Skill / Capability Manifest + trust**（可先 `SKILL.md`，后包管理）  
6. **Readiness check**（auth 只是子集）  
7. **Agent 生命周期状态契约**（reset / model / tools / compact / fork 何时允许）  
8. **ProjectionExplain + `/context` 可视化**

### P2

9. 统一 RPC/ACP/SDK/Cloud 事件语义（一种 RuntimeEvent，多种 sink）  
10. Branch summary、summary 内 file-ops 累积  
11. Capability 分发（git/npm 类）—— 先 manifest，后安装器  
12. Execution checkpoint / tool 幂等恢复  

### 6.1 与 AgentRuntime 文档的关系

Pi 启发的 **数据面**（Session 树、投影、可观察）与
[agent-runtime-optimization.md](../archive/agent-runtime-optimization.md) 的 **控制面**
（Runtime actor、TurnEngine ports、Cloud/ACP 拆分）正交。
**合并落地以该文 §16 Waves 为准**，避免「只拆 Runtime 仍毁 messages」或「只双写事件仍多处 queue」。

---

## 7. 明确不照搬

| Pi 选择 | 原因 | Zene 态度 |
|---------|------|-----------|
| No MCP | 个人 CLI 哲学 | 保持 MCP 一等公民 |
| No 内置 permission | 交给容器/用户 | 保持 Permission + Sandbox |
| No sub-agent / plan / todo / bg bash | 极简核心 | 保留产品能力，边界 trait 化 |
| 默认用户全权限 | 本地信任模型 | Cloud 默认隔离与审批 |
| 「功能少」当极简 | 误解 | 极简 = **核心契约稳 + 外围可替换**，不是砍功能 |

正确吸收方式：

```text
丰富的产品能力
+ 稳定的 core contract
+ 可替换的 composition root
```

---

## 8. 四句收束

1. **Agent Core 应稳定，工作流应可替换** — 与 `AgentBuilder` / crate 拆分同向。  
2. **Session 是事实来源，Context 只是投影** — 下一阶段最高杠杆架构议题。  
3. **可观察性本身是 Agent 能力** — 模型看到什么、为何 tool/block/compact，都应可解释。  
4. **扩展要有接口、生命周期、权限、版本与 trust** — 不必先做商店，先做模型。

---

## 讨论记录

### 2026-08-11 — 初稿

- 基于 Pi v0.84.1 release、coding-agent / agent-core 文档与源码树，对照 Zene crates 与 ENGINE/context 文档  
- 抽出 P0–P2 与「不照搬」清单；Session/Context 细节外链至专项文档

### 2026-08-11 — 对齐 AgentRuntime

- P0 增加 RuntimeEvent/ID；Tool 协议归属 ToolExecutor；P1 增加 RuntimeHandle
- 指向 [agent-runtime-optimization.md](../archive/agent-runtime-optimization.md) Merged Waves
