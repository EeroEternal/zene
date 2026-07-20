# Zene Headless Agent 与 Web UI 优化方案

## 1. 背景与决策

Zene 的核心价值是本地 coding agent 引擎，而不是终端界面。当前引擎已经具备会话、上下文压缩、权限、工具、MCP、沙箱、后台任务、Plan 模式和标准 ACP 接口；现有 TUI 只覆盖其中一部分能力，继续独立维护会产生重复的交互实现和长期兼容成本。

后续产品方向统一为：

- Zene 作为 **Headless Agent Runtime**，不再承担复杂 UI。
- Web Agent UI 成为主要交互界面。
- Web UI 与 Zene 之间只通过标准 ACP 语义通信，不引入 `x.ai/*` 私有扩展。
- 增加一个职责严格受限的本地 HTTP Gateway，负责 Web 与 ACP stdio 之间的传输适配。
- HTTP 长轮询作为默认实时通道，SSE 作为可选增强，普通短轮询作为最终降级；WebSocket 不作为首选或必需能力。
- Web 功能达到迁移门槛后删除 TUI 代码和依赖，保留 headless、ACP 和必要的命令行运维入口。

目标拓扑：

```text
┌──────────────────────┐
│      Web Agent UI    │
│ chat / plan / diff   │
│ tasks / sessions     │
└──────────┬───────────┘
           │ same-origin HTTP
           │ POST + long polling
           │ optional SSE
┌──────────▼───────────┐
│  Local HTTP Gateway  │
│ auth / event journal │
│ process supervision  │
│ ACP transport bridge │
└──────────┬───────────┘
           │ stdin/stdout NDJSON JSON-RPC
┌──────────▼───────────┐
│   zene acp process   │
│ Agent / tools / MCP  │
│ session / sandbox    │
└──────────────────────┘
```

## 2. 目标与非目标

### 2.1 目标

1. 让浏览器在不依赖 WebSocket 的环境中稳定使用 Zene。
2. 保持 Zene core 与 Web 技术栈解耦，使同一个 runtime 仍可被 IDE、桌面端和其他 ACP 客户端复用。
3. 支持消息流、reasoning、工具调用、权限审批、Plan、会话恢复、取消、prompt 队列、后台任务和 terminal 等完整 agent 交互。
4. 支持页面刷新、网络中断和 Gateway 重启后的可恢复交互。
5. 默认只暴露本机最小权限，并明确远程访问的安全边界。
6. 删除无人维护的 TUI，减少 `ratatui`、`crossterm` 和双套交互状态机的维护成本。

### 2.2 非目标

- 不把 Agent loop、工具执行或会话逻辑迁入 Gateway。
- 不让浏览器直接访问本地文件系统、PTY 或 Zene 子进程。
- 不把 ACP 替换成自定义 Agent 协议。
- 不要求本地默认配置 TLS 证书。
- 不在第一阶段建设云端多租户控制平面、账号系统或远程托管 Agent。
- 不实现 `x.ai/*` 私有 ACP 扩展。

## 3. 设计原则

### 3.1 单一业务真相

会话、消息、权限、工具状态和模式的业务真相属于 Zene。Gateway 可以缓存和投递事件，但不能自行推断 Agent 状态或复制 Agent loop。

### 3.2 ACP 语义不变

Gateway 只改变传输方式。HTTP 请求中的 ACP 消息应保持 JSON-RPC 字段和标准 ACP 方法名，使协议升级可以独立于 UI 迭代。

### 3.3 默认 HTTP，可渐进增强

所有核心流程必须在长轮询下可用。SSE 只能降低延迟和请求数量，不能成为正确性的前提。WebSocket 可以以后作为可选 adapter 增加，但不得成为唯一通道。

### 3.4 本地优先和最小暴露

默认监听 `127.0.0.1`，Gateway 与 Web 静态资源同源。远程监听必须显式开启，并要求 TLS、强认证和更严格的工作区策略。

### 3.5 可恢复而不是假定长连接可靠

每个下行事件都必须有单调递增的游标。浏览器重连时从最后确认的游标继续读取，不依赖某条 TCP 连接持续存在。

## 4. 组件职责

### 4.1 Zene Headless Runtime

继续由现有 Rust workspace 提供：

- `crates/core`：Agent turn loop、上下文管理、compaction、Plan、steer、subagent 和事件。
- `crates/llm`：OpenAI-compatible、Anthropic、流式输出、reasoning 和重试分类。
- `crates/tools`：文件、搜索、Shell、Todo、Task、AskUser 等工具。
- `crates/sandbox`：Keel profile、路径限制、网络出口控制。
- `crates/session`：会话、checkpoint、record、todo 和持久化。
- `crates/mcp`：stdio/HTTP MCP 客户端。
- `apps/cli`：`zene acp`、`zene -p`、诊断和配置入口。

Zene 通过 stdin/stdout NDJSON JSON-RPC 运行 ACP。stdout 只输出协议帧，日志写入 stderr。

### 4.2 Local HTTP Gateway

Gateway 只负责：

- 托管编译后的 Web 静态资源。
- 创建、复用、监控和终止 `zene acp` 子进程。
- 在 HTTP envelope 与 ACP NDJSON 帧之间双向转发。
- 维护连接、请求相关性、事件序号和短期事件日志。
- 实现长轮询、可选 SSE 和短轮询降级。
- 承担浏览器侧 ACP client capabilities，例如权限回复、terminal UI 桥接和可选 FS 桥接。
- 执行 loopback token、Origin、CSRF、工作区和远程访问控制。
- 暴露健康检查与本地诊断信息。

Gateway 不应：

- 解析模型输出或修改 tool call。
- 替 Zene 决定权限结果。
- 直接写入 Zene session 文件。
- 保存 API key 到浏览器。
- 在 HTTP API 中重新定义一套与 ACP 平行的 Agent 消息模型。

### 4.3 Web Agent UI

Web UI 负责纯交互状态：

- 会话列表、创建、恢复、关闭和切换。
- 用户输入、prompt 队列、取消与 steer。
- assistant message 和 thought 的增量渲染。
- tool call 生命周期、参数、结果、diff 和错误展示。
- 权限、AskUser 和 Plan 审批。
- Todo、后台任务、usage、context 水位和模式展示。
- terminal 输出展示与输入交互。
- 断线、重连、过期游标和 Agent 崩溃提示。

浏览器不得持有可直接绕过 Gateway 的文件或执行权限。

## 5. 进程与会话模型

### 5.1 推荐模型

第一版采用“一个 Gateway 管理一个或多个 `zene acp` 进程”：

- 一个 ACP 进程可管理多个 Zene session。
- Gateway 为每个进程分配稳定的 `agentId`。
- Web session 通过 `agentId + sessionId` 定位。
- 不同 workspace 默认使用不同 ACP 进程，避免 cwd、sandbox 和配置边界混杂。
- 同一 workspace 内可以复用 ACP 进程，减少启动成本。

后续若发现单进程多 session 存在隔离或阻塞问题，可以切换为“一 workspace 一进程”或“一活跃 session 一进程”，HTTP API 不需要变化。

### 5.2 生命周期

```text
Gateway 启动
  → 生成启动 token
  → 监听 loopback
  → 浏览器打开 Gateway 首页
  → POST 创建/附加 workspace agent
  → Gateway 启动 zene acp
  → ACP initialize
  → session/new、session/load 或 session/resume
  → 浏览器持续读取事件
```

子进程退出时，Gateway 必须产生明确的本地系统事件，包含退出码、是否可重启和 stderr 尾部摘要。Gateway 不应静默自动重放未确认的 prompt；只有具备幂等键且确认 Zene 未接收时才能自动重试。

## 6. HTTP 传输设计

### 6.1 基础约定

- API 前缀：`/api/v1`
- 内容类型：`application/json`
- 字符编码：UTF-8
- 所有写请求包含 `X-Zene-Token`
- 所有写请求包含 `requestId`，推荐 UUID。
- 所有响应包含 `serverTime` 和可用于诊断的 `traceId`。
- ACP payload 保持原始 JSON-RPC 结构，不改变 method 和 params。

### 6.2 核心端点

#### 启动信息

```http
GET /api/v1/bootstrap
```

返回 Gateway 版本、协议版本、传输能力、认证状态、轮询建议和 Web 构建版本。不得返回明文 API key 或完整环境变量。

#### 创建或附加 Agent

```http
POST /api/v1/agents
```

请求：

```json
{
  "requestId": "uuid",
  "workspace": "/absolute/path",
  "sandboxProfile": "workspace"
}
```

返回 `agentId`、进程状态和 ACP initialize 状态。路径必须经过允许目录、规范化和符号链接边界检查。

#### 发送 ACP 帧

```http
POST /api/v1/agents/{agentId}/messages
```

请求：

```json
{
  "requestId": "uuid",
  "messages": [
    {
      "jsonrpc": "2.0",
      "id": 12,
      "method": "session/prompt",
      "params": {
        "sessionId": "session-id",
        "prompt": [
          { "type": "text", "text": "修复测试" }
        ]
      }
    }
  ]
}
```

该接口接受 ACP request、notification，以及浏览器对 Zene 反向 request 的 JSON-RPC response。HTTP 202 只表示 Gateway 已接收，不表示 Agent 操作完成；业务结果通过事件通道返回。

#### 默认长轮询

```http
GET /api/v1/agents/{agentId}/events?cursor=123&waitMs=25000&limit=200
```

返回：

```json
{
  "events": [
    {
      "cursor": 124,
      "createdAt": "2026-07-20T05:00:00Z",
      "payload": {
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {}
      }
    }
  ],
  "nextCursor": 124,
  "hasMore": false,
  "agentState": "running"
}
```

行为约束：

- 有新事件时立即返回。
- 无事件时最多等待 `waitMs`，随后返回空数组。
- `limit` 到达时设置 `hasMore=true`，客户端立即继续读取。
- 客户端必须保存最后完整处理的 cursor，而不是最后接收的 cursor。
- 同一浏览器标签只保持一个长轮询请求，避免重复消费和代理压力。

#### 可选 SSE

```http
GET /api/v1/agents/{agentId}/events/stream?cursor=123
Accept: text/event-stream
```

SSE 每条事件携带 `id: <cursor>`，支持 `Last-Event-ID`。Gateway 需要发送心跳，并禁用反向代理缓冲。SSE 断开后客户端立即退回长轮询，不应阻塞用户操作。

#### 普通短轮询降级

仍使用 events 接口，但设置 `waitMs=0`，由客户端根据状态采用退避：

- Agent 活跃：250–1000ms。
- Agent 空闲：2–5s。
- 页面后台：5–15s。
- 网络错误：指数退避并增加抖动，上限 30s。

#### 健康检查

```http
GET /api/v1/health
GET /api/v1/agents/{agentId}/health
```

全局健康检查只表示 Gateway 可服务；Agent 健康检查另外返回子进程、ACP initialize、事件队列和最近心跳状态。

### 6.3 为什么不直接使用 REST 业务模型

可以为创建 Agent、健康检查等 Gateway 自身能力使用 REST，但 Agent 操作应继续携带 ACP JSON-RPC。若为 prompt、permission、tool、plan 各设计一套 REST 模型，会形成第三套协议：

```text
Web REST 模型 ↔ Gateway 内部模型 ↔ ACP ↔ Zene 模型
```

这会增加字段漂移、版本兼容和重复测试成本。推荐保持：

```text
HTTP envelope ↔ 原始 ACP frame
```

## 7. ACP 双向请求映射

ACP 不只是服务器推送通知；Zene 还会向客户端发起 request，例如 `session/request_permission`。HTTP 是客户端发起式协议，因此 Gateway 必须把反向 request 写入事件日志：

```text
Zene stdout: request id=88
  → Gateway event cursor=301
  → Browser polling receives request id=88
  → User approves
  → Browser POSTs JSON-RPC response id=88
  → Gateway writes response to Zene stdin
```

规则：

- Gateway 维护 `(agentId, jsonRpcId)` pending map。
- 相同 id 的重复 response 必须幂等处理。
- pending request 超时由 Zene 或协议语义决定，Gateway 只报告超时，不擅自批准。
- 浏览器刷新后必须能通过事件日志重新获得尚未回复的 request。
- 多标签页同时打开时，仅持有 lease 的控制端可以回复权限和 AskUser；其他标签页只读。

## 8. 事件日志、重连与一致性

### 8.1 事件游标

Gateway 为每个 `agentId` 维护 64 位单调递增 cursor。cursor 是 HTTP 传输元数据，不写入 ACP payload。

事件至少保留：

- 最近 10,000 条；或
- 最近 30 分钟；或
- 当前所有未完成反向 request 所需事件；

三者取更安全的范围。具体默认值应可配置。

### 8.2 过期游标

若客户端 cursor 早于最老事件，返回 HTTP 409：

```json
{
  "error": "cursor_expired",
  "oldestCursor": 500,
  "latestCursor": 900,
  "recovery": "reload_session"
}
```

Web UI 随后调用 ACP `session/load` 重建 transcript，再从最新 cursor 继续。不要尝试凭 UI 本地缓存猜测缺失 tool state。

### 8.3 幂等

- Gateway 对写请求按 `requestId` 保存短期结果。
- 重复 `requestId` 返回第一次的接收结果，不重复写入 Zene stdin。
- `session/prompt` 的自动重试必须特别保守，防止同一用户输入执行两次。
- JSON-RPC id 由 Web client 在单个 Agent 范围内保证唯一。

### 8.4 多标签页控制权

推荐使用带过期时间的 `controllerLease`：

- 第一个活跃标签页获得可写 lease。
- 其他标签页可以读取事件，但发送 prompt、cancel 或审批时收到冲突提示。
- lease 定期续约，页面关闭或失联后自动释放。
- 用户可以显式“接管”，旧页面收到控制权变化事件。

## 9. Web UI 信息架构

### 9.1 主界面

建议至少包含：

- **会话导航**：workspace、session、运行状态和最近活动。
- **主 scrollback**：user、assistant、thought、tool、system event。
- **composer**：多行输入、队列、发送、取消、steer 和模式切换。
- **上下文状态**：模型、token usage、context 水位、permission mode。
- **辅助面板**：Plan、Todo、后台任务、terminal 和文件 diff。

### 9.2 消息与 thought

- assistant message 按 `agent_message_chunk` 增量拼接。
- thought 按 `agent_thought_chunk` 单独显示，默认折叠并明确标识。
- 页面刷新后通过 `session/load` 重建历史；若持久化记录不含 thought，不应伪造。
- Markdown 渲染必须进行 HTML sanitization，禁止模型输出注入脚本。
- 代码块支持复制，后续再增加语法高亮和文件跳转。

### 9.3 Tool call

工具卡片按 `toolCallId` 更新：

```text
pending → in_progress → completed | failed
```

卡片应展示工具名、关键参数、持续时间、输出摘要和错误。Read/Grep 默认折叠；Bash 提供流式输出或 terminal 跳转；Edit/Write 优先展示 diff。大结果只展示摘要，并允许通过受控接口读取 spill 文件。

### 9.4 权限与 AskUser

- 权限 request 使用阻塞式但不锁死全页面的审批卡片。
- 明确展示工具、参数、workspace、风险和可选项。
- “始终允许”必须对应 ACP 返回的具体 option，不由 UI 自行扩大权限。
- AskUser 支持单选、多选和自由文本；断线重连后仍可继续回复。
- 同时存在多个待回复 request 时按创建时间排列。

### 9.5 Plan

- 显示当前模式和 Plan 状态。
- Plan 内容作为独立可滚动面板，而不是普通聊天文本。
- 支持批准、拒绝和附加修改意见。
- Plan 模式下明确标示只读约束。
- 第一版通过标准 ACP mode/update 和现有工具事件实现；标准 ACP 未表达的高级逐行批注暂不私有扩展，可转换为普通用户反馈。

### 9.6 Prompt 队列与取消

- Agent 运行时发送的新 prompt 明确进入 FIFO 队列。
- UI 展示队列顺序，允许取消尚未执行的本地草稿。
- `session/cancel` 只取消当前 turn，不隐式清空队列。
- steer 与“取消后立即发送”必须在文案和操作上区分。

### 9.7 Todo 与后台任务

- Todo 面板按 ACP `plan` 更新展示状态。
- 后台 Bash/Task 展示 running/completed/failed、启动时间和摘要。
- kill 操作通过 Agent 工具或标准能力发起，Gateway 不直接 kill 工具子进程。
- 若当前标准 ACP 事件不足，应优先把 Zene 内部状态映射到标准 `tool_call_update`/`plan`，而不是增加供应商字段。

### 9.8 Terminal

ACP terminal capability 由 Gateway 实现，Web UI 只是远程视图：

- Gateway 创建并管理 PTY/进程资源。
- 浏览器通过 HTTP 提交输入、resize、kill 和 release 意图。
- 输出进入统一事件日志，可通过 cursor 恢复。
- 每个 terminal 有独立资源 ID、输出 offset 和最大保留量。
- ANSI 渲染必须使用安全解析器，禁止 OSC 52 等敏感控制序列默认生效。
- terminal 创建、命令和 workspace 必须受 sandbox 与 permission 控制。

## 10. 安全模型

### 10.1 默认本地模式

- 只监听 `127.0.0.1` 和可选 `::1`。
- 启动时生成高熵随机 token。
- token 放入启动 URL fragment 或一次性 exchange 流程，避免长期出现在访问日志。
- Web UI 与 API 由同一 Gateway origin 提供。
- 校验 `Origin`/`Host`，防止恶意网页对 localhost 发起跨站请求。
- 使用严格 CSP、`frame-ancestors 'none'`、`X-Content-Type-Options: nosniff`。
- Cookie 若被采用，必须 `HttpOnly`、`SameSite=Strict`；写请求还需 CSRF 防护。

### 10.2 本地 HTTP 与 HTTPS

loopback 默认使用 HTTP，原因是本地自签证书会引入生成、信任、更新和跨平台问题。Gateway 同源托管 Web UI，因此不会发生 HTTPS 页面访问 HTTP localhost 的 mixed-content 问题。

只有以下场景启用 HTTPS：

- 监听非 loopback 地址。
- 通过局域网或远程主机访问。
- 由可信反向代理终止 TLS。

远程模式必须显式配置证书或反向代理，并启用强认证；不允许仅凭“隐藏端口”或固定弱 token 暴露 Agent。

### 10.3 Workspace 与文件权限

- 创建 Agent 时必须对 workspace 做 canonicalize。
- 默认只允许启动命令显式给出的根目录及其子目录。
- 拒绝通过符号链接逃逸允许目录。
- Gateway 不提供任意路径文件 API。
- Agent 的真实文件访问继续经过 Zene sandbox/path policy。
- 浏览器下载工具输出时，只能通过不透明资源 ID，不能提交任意本地路径。

### 10.4 Secret

- LLM key 只存在于 Zene 配置或 Gateway 进程环境，不发送到浏览器。
- bootstrap、health、错误和日志必须清理 Authorization、Cookie、env 和命令敏感参数。
- Web localStorage 不保存长期访问 token；优先内存或受限 session storage。

## 11. Gateway 模块建议

建议新增独立 Rust crate，避免把 HTTP 依赖扩散到 core：

```text
apps/gateway/
  src/
    main.rs           # CLI 与启动
    http.rs           # router、middleware、静态资源
    agent_manager.rs  # zene acp 子进程生命周期
    acp_transport.rs  # NDJSON 读写与 JSON-RPC 相关性
    event_journal.rs  # cursor、保留、长轮询通知
    auth.rs           # token、Origin、lease
    terminal.rs       # ACP terminal client capability
    diagnostics.rs    # health、受控日志
```

如果希望只发布单一二进制，也可以最终将 Gateway 作为 `zene web` 子命令编译进 `apps/cli`。内部仍应保持独立模块边界，且通过启动独立 `zene acp` 子进程维持协议隔离。第一版推荐独立 `zene-gateway` 二进制，便于故障隔离和测试。

Web 项目建议从当前静态占位目录中分离：

```text
apps/web-agent/
  src/
  public/
  package.json
```

构建产物可嵌入 Gateway 二进制或随 release 一起分发。开发模式允许 Gateway 代理前端 dev server，但发布模式必须同源托管。

## 12. 可观测性与诊断

Gateway 应提供结构化日志，但默认不记录 prompt、模型输出、文件内容和完整命令。建议字段：

- `traceId`
- `agentId`
- `sessionId`（可截断）
- `requestId`
- ACP method
- HTTP 状态码
- 处理耗时
- 子进程状态
- 当前/最老 event cursor
- pending request 数量

关键指标：

- 长轮询请求数、平均等待时间和超时比例。
- SSE 活跃连接与降级次数。
- event journal 条数、字节数和 cursor 过期次数。
- ACP stdin 队列深度和 stdout 解析错误。
- Agent 重启与异常退出次数。
- permission/AskUser 等待时长。

诊断包必须显式由用户触发并在导出前脱敏。

## 13. 背压与资源限制

- 限制单个 POST body、单条 ACP frame 和批量 message 数量。
- Zene stdout 读取与 journal 写入不得被慢浏览器阻塞。
- journal 使用有界内存；大 terminal/tool 输出落盘或截断。
- 长轮询 `limit` 防止一次响应过大。
- 每个 agent 限制并发 poll 数；异常客户端触发 429。
- stdin 写入使用有界队列，满时返回明确的 backpressure 错误。
- Gateway 关闭时停止接受写请求，等待已接收 frame 写入，再终止或保留 Agent。

推荐初始限制：

| 项目 | 默认值 |
|------|--------|
| POST body | 1 MiB |
| 单次 messages | 100 |
| 长轮询等待 | 25s |
| 单次事件数 | 200 |
| journal 事件数 | 10,000/agent |
| journal 保留时间 | 30min |
| 并发 poll | 2/agent/client |
| stderr 尾部 | 64 KiB |

限制应可配置，并在 bootstrap 中返回客户端需要知道的部分。

## 14. 错误模型

Gateway 错误与 ACP 错误必须区分：

- HTTP 4xx/5xx：认证、路径、限流、Agent 不存在、cursor 过期或 Gateway 故障。
- HTTP 202 + 后续 JSON-RPC error：ACP 方法执行失败。
- 本地系统事件：子进程退出、传输损坏或 Gateway 正在关闭。

推荐 Gateway 错误体：

```json
{
  "error": "agent_not_running",
  "message": "Zene ACP process is not running",
  "traceId": "trace-id",
  "retryable": true
}
```

Web UI 不应把所有失败都显示成“网络错误”，应区分可重试传输错误、ACP 业务错误和需要用户介入的安全错误。

## 15. TUI 删除与命令行边界

最终删除：

- `apps/cli/src/tui/`
- TUI 启动参数与默认启动分支。
- `ratatui`、`crossterm` 及仅为 TUI 使用的依赖。
- TUI 专用 permission/AskUser adapter、渲染代码和测试。

保留：

- `zene acp`：标准集成入口。
- `zene -p`：脚本和 CI headless 入口。
- `zene mcp doctor`、配置检查和必要诊断命令。
- 可选的基础 REPL，前提是其维护成本低且明确标注为调试入口；若仍复制权限、session 和渲染逻辑，也应一并移除。

Web 未达到最低迁移门槛前不要先删除 TUI。最低门槛见第 17 节。

## 16. 分阶段实施

### 阶段 A：协议与 Gateway 骨架

- 固化 Web 所需的标准 ACP capability 清单。
- 新增 Gateway 进程管理和 NDJSON bridge。
- 实现 bootstrap、agents、messages、health。
- 实现有游标的长轮询和内存 event journal。
- 实现 token、Origin 和 loopback 限制。
- 用模拟 ACP 子进程做双向 JSON-RPC 集成测试。

完成标准：浏览器或测试客户端可以 initialize、创建 session、发送 prompt、接收 streaming update 并响应 permission request。

### 阶段 B：Web 最小可用界面

- 会话创建/恢复。
- Chat streaming 和 thought 折叠。
- tool call/update 与基础 diff。
- 权限和 AskUser。
- cancel、prompt queue、模式和 usage。
- 网络断开/刷新恢复。

完成标准：真实项目中不依赖 TUI 完成“提出任务 → 审批 → 修改 → 测试 → 查看结果”。

### 阶段 C：Agent 工作面

- Plan 审阅。
- Todo 和后台任务面板。
- terminal bridge。
- session 列表、关闭、恢复和 workspace 切换。
- SSE 增强与自动降级。
- 多标签 controller lease。

完成标准：现有 TUI 能力全部覆盖，并完整展示当前 Zene 引擎已有的重要状态。

### 阶段 D：可靠性与发布

- Gateway 重启和 Agent 崩溃恢复。
- journal 持久化策略评估。
- 负载、背压、超大 tool output 测试。
- 浏览器兼容和代理环境测试。
- 单二进制或配套产物发布。
- 安全文档、配置文档和升级指南。

### 阶段 E：删除 TUI

- 将默认交互入口切到 `zene web` 或 `zene-gateway`。
- 删除 TUI 代码、参数、依赖、文档和 CI 路径。
- 保留一版迁移说明。
- 验证 headless、ACP、Gateway 和 release 构建。

## 17. 测试与验收标准

### 17.1 Gateway 自动化测试

- NDJSON 分帧：半帧、多帧、超长帧、非法 JSON。
- JSON-RPC request/response/notification 双向相关性。
- 重复 `requestId` 不重复写 stdin。
- 长轮询立即返回、等待超时、limit 和并发 poll。
- cursor 重连、重复读取、过期恢复。
- pending permission 在页面刷新后仍可回复。
- Agent 正常退出、崩溃、stderr 过大和启动超时。
- Origin、token、路径逃逸、符号链接和限流。
- journal 背压及 terminal 大输出。

### 17.2 Web 自动化测试

- streaming chunk 正确合并且不重复。
- tool card 按 `toolCallId` 更新。
- markdown/XSS sanitization。
- permission、AskUser 和 Plan 交互。
- prompt queue、cancel 和 steer 的行为差异。
- 断网、恢复、cursor 过期和 Agent 重启。
- 长轮询、SSE 断开降级和短轮询 fallback。

### 17.3 端到端场景

1. 启动 Gateway，创建 workspace Agent 和 session。
2. 请求读取、编辑并测试一个真实项目。
3. 在 Bash permission 出现时刷新页面，恢复后批准。
4. 执行期间提交第二个 prompt，确认进入队列。
5. 取消当前 turn，确认队列与 session 状态符合定义。
6. 观察 thought、tool、diff、usage 和 Plan 更新。
7. 运行后台任务并从面板读取结果。
8. 断开事件通道后重连，确认无丢失、无重复显示。
9. 强制结束 Zene 子进程，确认 UI 明确报告并可恢复。
10. 在禁止 WebSocket 的代理环境中只用 HTTP 完成全过程。

### 17.4 删除 TUI 的硬性门槛

只有同时满足以下条件才删除 TUI：

- Web 可完成 session new/load/resume/close。
- 支持 prompt、streaming、cancel 和队列。
- 支持 permission 与 AskUser。
- 支持 tool call/update 和 Edit/Write diff。
- 展示 Plan、mode、usage/context 和关键错误。
- 页面刷新不会丢失当前审批或破坏 session。
- 无 SSE/WebSocket 时长轮询完整可用。
- Gateway 默认安全绑定 loopback，并通过 Origin/token 测试。
- 至少 Linux 和 macOS release 路径验证通过。

## 18. 兼容性与版本管理

Gateway bootstrap 返回：

- Gateway API version。
- 支持的传输方式。
- ACP protocol version。
- Zene runtime version。
- Web build version。

兼容策略：

- `/api/v1` 内只做向后兼容字段增加。
- 破坏性 HTTP envelope 变化使用 `/api/v2`。
- ACP capability 通过 `initialize` 协商，不由 Gateway 硬编码猜测。
- Web 发现 runtime capability 缺失时隐藏对应功能并显示明确说明。
- Gateway 与 Web 推荐同版本发布，Gateway 与 Zene runtime 允许在声明的兼容区间内独立升级。

## 19. 主要风险与缓解

| 风险 | 缓解 |
|------|------|
| HTTP 双向语义比 WebSocket 复杂 | 统一 event journal；所有反向 request 也作为事件；POST 回传 response |
| 长轮询产生额外请求 | 25s 等待、keep-alive、活跃/空闲退避；可选 SSE |
| 浏览器刷新导致审批丢失 | pending request 进入 journal，使用 cursor 重放 |
| 多标签重复操作 | controller lease + 幂等 requestId |
| 本地服务遭恶意网页攻击 | loopback token、Origin/Host、CSRF、CSP |
| tool/terminal 输出耗尽内存 | 有界 journal、spill、截断和背压 |
| Gateway 逐渐承载业务逻辑 | 明确模块边界；ACP payload 透明；业务测试留在 Zene |
| 提前删除 TUI 导致不可用 | 达到第 17.4 节门槛后再删除 |
| 本地 HTTPS 配置复杂 | loopback 同源 HTTP；远程模式才强制 TLS |

## 20. 推荐的首个交付切片

首个 PR 不应同时引入完整 Web UI。建议最小垂直切片：

1. 新增 `zene-gateway`。
2. Gateway 启动一个 `zene acp`。
3. 实现 token、Origin、`POST /messages` 和 `GET /events` 长轮询。
4. 用一个极简静态页面完成 session/new、session/prompt、消息流和 permission 回复。
5. 增加模拟 ACP 与真实 `zene acp` 的端到端测试。

这个切片可以最早验证三个关键假设：标准 ACP 是否足够、无 WebSocket 的 HTTP 双向映射是否可靠、Gateway 是否能保持足够薄。验证通过后再建设完整 Web Agent UI，并按迁移门槛移除 TUI。

## 21. 实施状态

### 阶段 A（已落地骨架）

- [x] 设计文档
- [x] `apps/gateway` / 二进制 `zene-gateway`
- [x] ACP 子进程管理与 NDJSON 转发
- [x] `GET /api/v1/bootstrap`、`GET /api/v1/health`
- [x] `POST /api/v1/agents`
- [x] `POST /api/v1/agents/{id}/messages`（含 `requestId` 幂等）
- [x] `GET /api/v1/agents/{id}/events` 长轮询 + cursor journal
- [x] loopback 默认绑定、`X-Zene-Token`、Origin 校验
- [x] 嵌入式最小 Web 页（session/new、prompt、stream、permission）
- [x] `zene-gateway-mock-acp` + HTTP 集成测试

### 阶段 B（已落地）

- [x] 真实 `zene acp` 端到端 smoke（mock OpenAI-compatible LLM）
- [x] 独立 `apps/web-agent` 静态前端（零构建，由 gateway `include_str!` 嵌入）
- [x] session list/new/load UI、tool/diff 卡片、usage/context
- [x] SSE 可选通道（`/events/stream`）与 Web 自动降级到长轮询
- [x] 多标签页 controller lease（acquire/heartbeat/release + 写保护）

### 阶段 C（已落地）

- [x] Plan 审阅面板与模式切换 UI（`session/set_mode`）
- [x] Todo / 后台任务面板（基于 `plan` / tool_call 更新）
- [x] Gateway 本地 `terminal/*` host + Web 终端面板 / HTTP 查询与 kill
- [x] session close 与 sessions 列表刷新

### 阶段 D（已落地）

- [x] journal 落盘（`~/.zene/gateway` / `--data-dir` / `--no-persist`）
- [x] Agent 崩溃 `restart` 与 Gateway 重启后 `attach`（不静默重放 prompt）
- [x] 背压：并发 poll 上限、POST/message 大小限制、超大 payload 截断
- [x] `zene web` 入口与 [GATEWAY_OPS.md](./GATEWAY_OPS.md) 运维文档

### 下一刀（阶段 E）

- [ ] 达到删除 TUI 硬性门槛后移除 ratatui
- [ ] 默认交互入口切到 Web Agent
