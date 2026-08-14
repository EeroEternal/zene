# Agent Runtime — 本线收尾与后续目标

> 状态：**控制面 / 事件产品化主线已收口**（截至 PR #113，基线 `8c56fd2`）。
>
> 本文记录结项结论、刻意不做的边界，以及下一批**可选**产品目标。
> 详细设计与 Wave 历程仍以 [agent-runtime-optimization.md](./agent-runtime-optimization.md) 为准。

## 1. 本线结项（已完成）

本线主攻 **控制面**（谁在跑、怎么被控制、状态归谁）与 **Cloud 事件产品化**，不是 Console chrome 重做。

已落地（摘要）：

- Turn 默认路径走 ports；主 Agent / Subagent 共用 `TurnEngine` 语义。
- Wave 14：`RuntimeScope`（ToolPolicy / SessionPolicy）+ Agent facade（prepare / model / tool_batch / turn_session）。
- Actor 迁入 `zene-agent-runtime`；协议在 `zene-runtime`；**Cloud 不依赖二者**。
- `ModelExecutor` / `ContextModel` 共用中性 `ModelRequest` / `ModelResponse`；provider 仍用 `ChatRequest`。
- Cloud `RuntimeCommand` 与本地共享变体字段对齐（Prompt/Steer/Cancel/Approval/SetMode/Shutdown）。
- Cloud 事件：timeline 类产品字段 + `initialized` / `unsupported_request` / turn·step·error；未知 update 为 residual。
- `session_started` 携带 modes + recovery（inspect-only；mock 同形）。
- **Cloud session mode（`default` / `plan`）以推送为真相源**（`session_started` + `current_mode_update`→`state_changed`）；**明确不做 Cloud GetMode**。
- `zene-core` 不再 re-export runtime protocol / turn `RuntimeEvent` 类型。

本线 **停线**。勿再为 GetMode / reply-shaped 读 mode 开碎片 PR。

## 2. 刻意保持的非目标

下列项 **不是遗漏**，本线明确不做或继续维持边界：

| 项 | 态度 |
| --- | --- |
| pending tool / approval 自动 replay | 继续 inspection / 人工介入，避免重复副作用 |
| Cloud `GetMode` / `session/get_mode` | 已否决；mode 用推送 SoT |
| Cloud 依赖 `zene-runtime` / `zene-agent-runtime` | 禁止 |
| 合并本地与 Cloud `RuntimeEvent` 信封 | 不做强行统一 |
| 完整 Event Sourcing / 历史 `acp` 行迁移 | 不做 |
| Console chrome / 时间线视觉大改 | 不做（本线事件多数不进时间线） |

## 3. 后续新目标（可选，另开产品线）

下列目标需要 **单独产品决策与排期**，不要当作本线未完成勾选：

### 3.1 Durable Subagent session

- 现状：Subagent 使用 `SessionPersistence::Ephemeral`（内存消息）。
- 目标：子 Agent 可落盘 / 恢复，与主 session lineage 可解释关联。
- 依赖：持久化模型、fork/rewind 边界、是否进 Cloud Run 投影。

### 3.2 跨 VM EventOutbox

- 现状：本地 durable outbox；跨 VM 需共享 POSIX 卷或 DB/object spool（见 Cloud deploy 文档）。
- 目标：replacement worker 跨机器仍可可靠投递 / ACK。
- 依赖：部署拓扑选型（共享盘 vs DB spool），不是再改一层 adapter 能替代的。

### 3.3 Cloud `ResumeSafeTurn`（若需要）

- 现状：仅本地；Cloud 已有 session resume + inspect-only recovery 元数据。
- 目标：仅当产品要求 **安全 model-boundary 自动续跑** 且有明确调用方时再做。
- 约束：仍禁止 pending tool 自动 replay；与 reply-shaped 控制面设计绑定。

### 3.4 Agent composition-root 继续退回 wiring

- 现状：step 算法已抽出；`Agent` 仍持有 model/tools/sandbox/permission/hooks/MCP 等。
- 目标：进一步模块化 holdings，降低 God-object 感。
- 杠杆：结构清理，低于有调用方的产品能力。

### 3.5 Provider 面 `ChatRequest`（可选长期）

- 现状：Turn / ContextModel 已中性；`ChatClient` / provider 仍 `ChatRequest`。
- 目标：仅当要替换 provider 栈或共享跨语言边界时再推。
- 态度：保持 provider 边界即可，不必为对称而改。

### 3.6 新的 Cloud 控制能力（非读 mode）

- 仅当出现 **真实调用方**（JobRunner / API / Console）时，再设计 reply-shaped 或新命令。
- 读 mode 已关闭；其他读/写控制另立 RFC，勿挂在本线 changelog 下碎片推进。

## 4. 如何本地体验（验证本线结果）

产品可跑；本线改动多为边界与事件形态，Console 外观不会大变。

### 4.1 一键起 Cloud Console

```bash
cd cloud && ./scripts/dev.sh
# 浏览器打开 http://127.0.0.1:8788/
```

推荐路径：注册 → **Settings** 配置 LLM（BYOK）→ Connect GitHub → **New Agent** → 发消息跑一轮。

可选：先 `./scripts/install.sh`（或让 `dev.sh` 自行构建）确保有仓库根 `zene` ACP 二进制。

### 4.2 UI 热更新（可选）

```bash
cd cloud && ZENE_CLOUD_SKIP_WEB_BUILD=1 ./scripts/dev.sh
# 另开终端：
cd cloud/apps/web && npm run dev
# 打开 http://127.0.0.1:8787/（/api/* 代理到 8788）
```

### 4.3 建议观察点

- 正常对话、工具审批、取消、follow-up（忙时 Steer / 闲时 Prompt）。
- 切换 session mode（`default` / `plan`）：应走推送（`session_started` / `state_changed`），无 GetMode 拉取。
- 事件时间线仍以 text / thought / user / tool 为主；turn/step/error、initialized、projection 等为产品字段但不进时间线 chrome。

细则见 [cloud/README.md](../cloud/README.md) 与根 [README.md](../README.md)。

## 相关文档

- [agent-runtime-optimization.md](./agent-runtime-optimization.md) — 目标架构与 Wave 全记录
- [session-as-source-of-truth.md](./session-as-source-of-truth.md) — 数据面 Session SoT
- [context-engine.md](./context-engine.md) — Context 投影
- [agent-components.md](./agent-components.md) — 可组装 crate 栈
- [ENGINE.md](./ENGINE.md) — turn / compaction 行为
- [ROADMAP.md](./ROADMAP.md) — 历史 CLI 里程碑清单
