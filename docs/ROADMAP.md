# Zene 开发计划

Zene 目标：Rust 实现的本地 code agent CLI（对标 kimi-code / flue 的核心引擎能力，非 Web 服务形态）。

当前状态：MVP 骨架已具备——REPL CLI、agent turn loop、6 个内置工具、session 落盘、OpenAI-compatible LLM。能跑通简单改代码流程，但距离「可靠、长时间、可控地写代码」还差核心引擎层。

---

## 优先级总览

| 阶段 | 主题 | 目标 |
|------|------|------|
| P0 | 工具可靠性 | Edit/Read 改代码不翻车 |
| P1 | Context 地基 | 长会话不爆窗 |
| P2 | Turn loop 加固 | 可取消、可重试、行为可预期 |
| P3 | Workspace 上下文 | Agent 理解项目 |
| P4 | Tool gate | 安全可控地执行危险操作 |
| P5 | 能力扩展 | Subagent / Skills / MCP |
| P6 | 可观测性 | Event / Record，为 TUI 做准备 |
| P7 | 产品化 | TUI、安装分发、权限模式 |

---

## P0 — 工具可靠性（改代码的地基）

> 没有可靠的 Read/Edit，其他能力都建立在沙子上。

### P0.1 Edit 唯一性检查

- [x] `replace_all=false` 时统计 `old_string` 出现次数
- [x] 0 次 → 返回 error，提示重新 Read
- [x] 2+ 次 → 返回 error，提示加更多上下文或 `replace_all=true`
- [x] `old_string === new_string` → 拒绝，返回 no-op error
- [x] `old_string` 为空 → 拒绝

参考：kimi `packages/agent-core/src/tools/builtin/file/edit.ts`，flue `packages/runtime/src/agent.ts` `createEditTool`。

### P0.2 换行符（CRLF/LF）处理

- [x] Read 时检测文件换行风格（lf / crlf / mixed）
- [x] 给模型的文本视图统一为 LF（CRLF 文件 `\r\n` → `\n`）
- [x] Edit 在 LF 视图上匹配和替换
- [x] Write/Edit 写回时按原文件风格 materialize（crlf 文件还原 `\r\n`）
- [x] Read 输出中对 `\r` 做可见化（mixed 换行时）

参考：kimi `packages/agent-core/src/tools/builtin/file/line-endings.ts`。

### P0.3 Read 改进

- [x] 增加字节上限（如 50KB），与行数上限（2000 行）并列
- [x] 支持列目录（path 为目录时返回 entry 列表）
- [x] 错误信息明确：文件不存在、路径越界、二进制文件

参考：flue `createReadTool`（目录 listing + offset/limit + 字节截断）。

### P0.4 辅助工具补强

- [x] Glob 改用标准 glob 引擎（支持 `**/*.rs`）
- [x] Grep 考虑调用系统 `rg`（有则用，无则 fallback 当前实现）
- [x] Bash 增加超时（默认如 120s）和可配置输出上限

### P0.5 工具测试

- [x] `crates/tools` 集成测试：Write → Edit happy path
- [x] 重复 match、CRLF 文件 edge case
- [x] 新建文件、目录 Read 等 edge case

**P0 完成标准**：在真实 Rust/TS 项目里，agent 连续 Edit 同一文件 10+ 轮，不因换行符或 silent partial edit 产生 silent 错误。

---

## P1 — Context 地基（长会话）

> Agent 能不能写大项目，取决于 context 管理，不是取决于工具数量。

### P1.1 Token 估算

- [x] 实现 message + tools 的 token 估算（初期可用字符数/heuristic，后期接 tiktoken 或 provider usage）
- [x] 每次 LLM 调用前记录 context 大小
- [x] 从 provider response 读取真实 usage 并累计

### P1.2 Context compaction

- [x] 定义 compaction 触发条件（token 超过窗口 × 比例，或 LLM 返回 context overflow）— `CompactionConfig.trigger_ratio` placeholder
- [x] 将旧消息 summarize 为一条 compaction summary message
- [x] 保留最近 N token 的原始消息 tail
- [x] overflow 后 compact + 自动 retry 当前 step

参考：kimi `agent/compaction/`，flue `compaction.ts` + `session.ts` overflow 路径。

### P1.3 System prompt 不再每 step 重复注入

- [x] system prompt 只在 context 构建时注入一次
- [x] compaction 后 system 信息保留在 summary 或独立 system slot

### P1.4 Session 数据结构升级

- [x] session JSON 增加 compaction entry（或等价的 summary 节点）
- [x] 区分 user prompt / tool result / compaction summary

**P1 完成标准**：100+ 轮对话或大型 repo 探索后，agent 仍可继续工作，不因 context 爆窗中断。

---

### P2.5 Steer (in-turn follow-up)

- [x] `SteerBuffer` + `Agent::steer(text)` inject user messages between steps
- [x] Reject concurrent `prompt()` with steer hint
- [x] CLI `/steer <msg>`; `AgentEvent::SteerInput`

参考：kimi `packages/agent-core/src/agent/turn/index.ts` `steerBuffer`。

### P1.1 Token 估算（升级）

- [x] 分 role 估算 + tools JSON 单独计数
- [x] 可配置 `chars_per_token` / `model_chars_per_token`
- [x] `estimate_context(messages, tools)` 供 compaction 触发前使用
- [x] 估算 ≥ 90% 窗口时 warn 日志

### P1.2 Context compaction（细化）

- [x] summarize 前先 **truncate-only** 截断旧 tool result
- [x] `min_keep_messages` 消息条数下限（默认 20）

### P4.2 Permission 策略

- [x] Write/Edit 硬拒绝 `node_modules/`、`.git/` 路径（与 sandbox 对齐）

详见 [ENGINE.md](./ENGINE.md)。

---

## P2 — Turn loop 加固

> 让 agent 行为可预期、可中断、可恢复。

### P2.1 Turn / Step 模型

- [x] 引入 turn（一次用户 prompt）和 step（一次 LLM 调用）边界
- [x] 每次 step 生成 step id，便于日志和 replay
- [x] 同一 session 同时只允许一个 active turn

### P2.2 取消与中断

- [x] 全链路 `CancellationToken`（或 tokio cancel）
- [x] REPL 支持 Ctrl+C 中断当前 turn（不退出 CLI）
- [x] Bash / LLM stream 在中断时清理

### P2.3 LLM 错误处理

- [x] connection / timeout / 5xx 自动 retry（指数退避）
- [x] 区分 context overflow（触发 compact）和其他 API error
- [x] 达到 max_steps 时返回明确 error，不静默返回空字符串

### P2.4 Tool result 语义

- [x] tool error 正确传给 provider（OpenAI tool message error 标记或等效文本格式）
- [x] 空 tool output 有明确 handling

参考：kimi `loop/run-turn.ts`、`loop/turn-step.ts`、`loop/retry.ts`。

**P2 完成标准**：网络抖动可自动恢复；用户可随时中断；max_steps 触顶有清晰报错。

---

## P3 — Workspace 上下文

> Agent 知道「这是哪个项目、该遵守什么规则」。

### P3.1 AGENTS.md / CLAUDE.md 发现

- [x] 启动 session 时从 workdir 读取 `AGENTS.md`、`CLAUDE.md`
- [x] 内容注入 system prompt 或首条 context

### P3.2 项目概览

- [x] system prompt 包含 workdir 绝对路径
- [x] 可选：顶层目录 listing（控制 token 预算）
- [x] 可选：git 状态摘要（branch、dirty files）

### P3.3 配置扩展

- [x] `~/.zene/config.toml` 支持 per-project 覆盖
- [x] 支持 `.zene/config.toml` 项目级配置

参考：flue `context.ts` `discoverSessionContext`，kimi `profile/resolve.ts`。

**P3 完成标准**：在新 repo 里启动 zene，agent 能读到 AGENTS.md 并按项目约定行动。

---

## P4 — Tool gate（安全与可控）

> 本地 code agent 必须能控制「写文件 / 跑命令」。

### P4.1 参数校验

- [x] 所有 tool args 走 JSON Schema 校验（已有 schema 定义，需 enforce）
- [x] 非法参数直接 synthetic error 回传 LLM

### P4.2 Permission 模式

- [x] `manual`：Write / Edit / Bash 执行前 CLI 询问（y/n）
- [x] `yolo`：全部自动批准（`--yolo` flag）
- [x] session 级「记住此操作」规则（可选）

### P4.3 Path 策略

- [x] 敏感路径保护（如 `.git/`、`.env`）可配置 deny/ask
- [x] workspace 外路径拒绝（已有基础 check，需完善 symlink 边界）

### P4.4 Tool 去重提醒

- [x] 相同 tool + 相同 args 连续调用时，在 result 里注入 reminder
- [x] 防止 read/bash 死循环

参考：kimi `permission/`、`tools/policies/`、`agent/turn/tool-dedup.ts`。

**P4 完成标准**：默认模式下误删文件、误跑危险命令前用户有机会拦截。

---

## P5 — 能力扩展

> 从「能改单个文件」到「能完成复杂任务」。

### P5.1 Subagent / Task

- [x] 内置 `Task` tool：spawn 子 agent，独立 context，共享 sandbox
- [x] 预置 profile：`explore`（只读）、`coder`（读写）
- [x] 限制嵌套深度（max depth 1）
- [x] 子 agent 工具执行走主 agent 同一 permission gate（继承 manual/yolo 模式）

参考：flue `task` tool + `defineAgentProfile`，kimi `tools/builtin/collaboration/agent.ts`。

### P5.2 Skills

- [x] 发现 `.agents/skills/*/SKILL.md`
- [x] system prompt 列出可用 skills
- [x] `Skill` tool 激活 skill（读 SKILL.md 内容注入 context）

### P5.3 MCP

- [x] 支持 stdio MCP server 配置（`~/.zene/mcp.json` + 项目 `.zene/mcp.json` 合并）
- [x] 动态注册 MCP tools 到 agent（`mcp__{server}__{tool}` 前缀）
- [x] session 生命周期内连接管理（启动连接、退出断开）

### P5.4 多 Provider

- [x] LLM 层抽象出 `Provider` trait（`chat` / `chat_stream`）
- [x] `OpenAiCompatibleProvider`（OpenAI / DeepSeek / Kimi 等兼容端点；底层 intentionally 使用 crates.io 上的 `unigateway-sdk`）
- [x] `AnthropicProvider` MVP（Messages API + tools + streaming）
- [x] 配置 `provider = "openai"|"anthropic"`，环境变量 `ZENE_PROVIDER`、`ZENE_BASE_URL`、`ANTHROPIC_API_KEY`、`ZENE_ANTHROPIC_BASE_URL`
- [x] model 能力元数据：内置 context window 默认值 + `model_context_windows` 配置覆盖

参考：kimi `kosong/`，flue `pi-ai` + `runtime/providers.ts`。

**P5 完成标准**：能 dispatch explore 子 agent 搜代码，主 agent 汇总后改代码；能加载一个 MCP server 的工具。

### P5.5 协作与 Web 工具（Kimi-like）

- [x] `AskUserQuestion`：结构化提问，CLI stdin 编号选项或自由文本；`Agent::set_ask_user_prompter` 供 TUI 覆盖
- [x] `TodoWrite` / `TodoList`：会话级内存 todo 列表，按 id merge，状态 pending / in_progress / completed
- [x] `FetchUrl`：HTTP GET（30s 超时、100KB 上限），HTML 剥离为纯文本
- [x] `WebSearch`：可配置 Tavily API 或 DuckDuckGo HTML 回退；plan mode 下允许（只读调研）
- [x] plan mode 下允许 AskUser / Todo / FetchUrl / WebSearch（只读探索 + 协作）
- [x] todo 持久化到 SessionRecord（reload 后保留）

参考：kimi `tools/builtin/collaboration/ask-user.ts`、`state/todo-list.ts`、`web/fetch-url.ts`。

### P5.6 Agent profile（可配置工具集）

- [x] `agent_profile = "full" | "explore" | "coder"`（`~/.zene/config.toml` 或 `ZENE_AGENT_PROFILE`）
- [x] `explore`：只读 + 协作/Web 工具 + plan mode
- [x] `coder`：读写 + Task 子 agent + 协作/Web 工具

---

## P6 — 可观测性

> 为 debug、replay、未来 TUI 打基础。

### P6.1 Event 模型

- [x] 定义事件：turn_start、step_begin、text_delta、tool_call、tool_result、turn_end、error
- [x] core 通过 callback/channel 广播，CLI 订阅渲染

### P6.2 Agent record

- [x] 结构化 record 落盘（JSONL），不只存 LLM messages
- [x] 支持 export session 为 zip

### P6.3 Usage 统计

- [x] 每 turn 汇总 input/output tokens
- [x] CLI 可选显示 usage 面板

参考：kimi `AgentRecords` + RPC events，flue `FlueEvent`。

**P6 完成标准**：一次完整 session 可导出 record，能离线 replay 工具调用序列。

---

## P7 — 产品化

> 从开发者自用 CLI 到可分发产品。

### P7.1 TUI

- [x] `zene --tui` 启动 ratatui 界面（默认仍为 rustyline REPL）
- [x] 布局 MVP：可滚动聊天历史、底部输入框、状态栏（model / session / usage）
- [x] 通过 `AgentEvent` + `event_handler` 驱动 UI（TextDelta 流式、ToolCall/ToolResult 紧凑行）
- [x] manual 模式权限 overlay（y/n/a，复用 `PermissionGate`）
- [x] Ctrl+C 取消 turn / 退出；Esc 连按两次退出；退出时恢复终端
- [x] 流式 markdown 渲染（**bold** / `code` / 代码块 + 自动换行）
- [x] tool call 富渲染、Edit diff 预览（unified diff 前 20 行）

### P7.2 Plan 模式

- [x] `EnterPlanMode` / `ExitPlanMode` 工具；plan mode 下仅 Read/Grep/Glob/Skill + plan 工具
- [x] `ExitPlanMode` 写入 `.zene/plan.md` 或 session 目录，stdin 审批（`--yolo` 不可跳过）
- [x] plan mode system reminder 注入；CLI `/plan` 进入

### P7.3 Hooks

- [x] PreToolUse / PostToolUse 本地脚本 hook
- [x] hook 可 block tool 执行

### P7.4 安装与分发

- [x] `cargo install` 路径稳定
- [x] 可选：单二进制 release（GitHub Actions，`v*` tag 触发）
- [ ] 验证 Release workflow：tag 推送后各平台二进制成功构建并出现在 GitHub Releases
- [x] 安装脚本

参考：kimi `apps/kimi-code` TUI + native bundle。

---

## 建议实施顺序（单线程）

```
P0.1 → P0.2 → P0.3 → P0.5          # 工具可靠
    → P1.1 → P1.2 → P1.3 → P1.4   # 长会话
    → P2.1 → P2.3 → P2.2 → P2.4   # turn loop
    → P3.1 → P3.2                   # 项目上下文
    → P4.1 → P4.2 → P4.4           # 安全
    → P0.4                          # 辅助工具
    → P5.1 → P5.2 → P5.3 → P5.4   # 扩展
    → P6.*                          # 可观测
    → P7.*                          # 产品化
```

P4.3（path 策略）可与 P4.2 并行。P5 内部 Subagent 优先于 MCP。

---

## 不在近期范围

以下明确不做，避免 scope 膨胀：

- Web UI / Cloudflare Worker 部署（旧 zene 方向，已废弃）
- Workflow / HTTP server 形态（flue 特有，非本地 CLI 所需）
- OAuth / 托管认证（可后期按需加）
- Background tasks、video input 等 kimi 高级特性

---

## 里程碑

### M1 — 能改代码（P0）

Edit 可靠，CRLF 正确，有测试覆盖。

### M2 — 能改大项目（P0 + P1 + P2）

长 session 不爆窗，可中断，LLM 可 retry。

### M3 — 像 code agent（+ P3 + P4）

读 AGENTS.md，危险操作需确认。

### M4 — 能干复杂活（+ P5）

Subagent + Skills + MCP。

### M5 — 可日常使用（+ P6 + P7）

TUI、record/replay、安装分发。

---

## 参考项目对照

| Zene 模块 | Kimi 对应 | Flue 对应 |
|-----------|-----------|-----------|
| `crates/tools` | `packages/agent-core/src/tools` | `packages/runtime/src/agent.ts` |
| `crates/core` | `agent/` + `loop/` | `session.ts` + pi-agent-core |
| `crates/llm` | `packages/kosong` | `@earendil-works/pi-ai`（OpenAI-compatible 经 crates.io `unigateway-sdk`） |
| `crates/sandbox` | `packages/kaos` | `sandbox.ts` / `local()` |
| `crates/session` | `session/store` | Cloudflare/Node session store |
| `apps/cli` | `apps/kimi-code` | `packages/cli` |

---

---

## Grok 对齐迭代（2026-07）

| 阶段 | 状态 | 内容 |
|------|------|------|
| P1 采样/上下文 | [x] | 水位/full-replace/阶梯；续1 截断/context/tool-pair；续2 prefire two-pass/segments；续3 memory flush/注入 + 工具结果 intra 边界 |
| P2 权限 | [x] | default/accept_edits/dont_ask/bypass + allow/deny/ask 规则 |
| P3 会话恢复 | [x] | compaction checkpoints、`/rewind`、`/fork`、`/session-info` |
| P4 运行时 | [x] | 后台 Bash/Task + TaskOutput、`--worktree`、subagent 报告包装 |
| P5 MCP/扩展 | [x] | stdio + HTTP MCP、`zene mcp doctor` |
| P6 集成/产品化 | [x] | `zene -p` headless + `--output-format json`；`zene acp` 最小 ACP stdio |

*最后更新：2026-07-18（P1 续3：memory flush + tool bound）*
