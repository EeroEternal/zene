# Zene Cloud Platform 产品与系统设计

状态：方案评审稿  
目标版本：0.1  
目标读者：产品、前端、后端、Agent Runtime、基础设施与安全工程师

## 1. 摘要

Zene Cloud Platform 是一个可注册登录、连接 Git 仓库、在隔离环境中运行 Coding Agent、实时查看过程、阅读代码、审查变更并创建 Pull Request 的多租户平台。

产品体验参考现代 Cloud Coding Agent：

- 用户登录后进入 Dashboard。
- 左侧是 New Agent、历史 Agent、Automations、代码仓库与团队入口。
- New Agent 页面选择仓库、分支、环境、模型，输入任务并启动。
- Agent 在后台持续运行，用户可以关闭页面后再回来。
- 运行页实时展示消息、思考摘要、工具调用、Todo、终端、文件变更与 Git 状态。
- 用户可以追加指令、取消、批准敏感操作、接管控制、查看代码和 diff。
- Agent 完成后可提交、推送并创建 PR；用户可以继续追问或开启后续 Run。

本方案不是把现有本地 `zene-gateway` 直接暴露到互联网。正确边界是：

```text
Web / API Client
      │
      ▼
Cloud Control Plane
      │  调度、鉴权、事件、Git、配额
      ▼
Isolated Agent Worker
      ├── zene acp
      ├── Keel strict sandbox
      ├── git workspace
      └── terminal / tools
```

Zene 是 Worker 内的 Agent Runtime；Keel 是 Worker 内的第二层执行约束；microVM/容器才是租户之间的主安全边界。

## 2. 目标与非目标

### 2.1 产品目标

- 支持邮箱、OAuth、Passkey 注册登录与团队组织。
- 支持 GitHub 仓库授权、安装、选择、克隆、分支、提交、推送和 PR。
- 支持后台长时间 Agent Run，浏览器断线不影响执行。
- 支持实时消息、工具、权限、Todo、终端、diff 和状态恢复。
- 支持代码树、全文搜索、符号跳转、代码阅读、引用与变更审查。
- 支持个人和团队的访问控制、审计、配额和密钥管理。
- 支持水平扩展、故障恢复和可观测性。
- 保留未来 GitLab、Bitbucket、自托管 Git、Automations 与 API 接入能力。

### 2.2 技术目标

- 控制面无状态化，业务真相持久化。
- 每个 Run 有独立 workspace、身份、资源配额和网络策略。
- Agent 协议保持 ACP 语义，避免再创造一套 Agent 业务协议。
- 所有事件可排序、可重放、可断线续传。
- Git 写操作具备明确授权、幂等与审计。
- Secret 不进入浏览器、不写入仓库、不落入 Agent transcript。

### 2.3 非目标

- 第一版不做完整云 IDE，不提供任意插件市场。
- 第一版不做多人同时编辑同一工作区。
- 第一版不支持用户直接 SSH 登录 Worker。
- 第一版不支持任意自带容器镜像直接运行。
- 第一版不承诺与 Cursor 私有协议或内部实现兼容。
- 不复制 Cursor 的商标、图标或受保护视觉资产，只借鉴信息架构与交互模式。

## 3. 关键设计原则

### 3.1 控制面与执行面分离

控制面管理用户、仓库、Run、事件索引、权限和调度；执行面持有代码和运行 Zene。控制面不得直接执行仓库代码。

### 3.2 单一业务真相

- 用户、组织、仓库授权、Run 元数据：Postgres。
- Agent turn、tool 状态、session 上下文：Zene Runtime。
- 实时事件：持久事件流。
- Git 提交与分支最终状态：Git provider。
- Workspace 文件：运行中以 Worker volume 为准，结束后以 commit/patch/snapshot 为准。

### 3.3 默认拒绝

默认无公共仓库写权限、无任意网络、无跨 workspace 文件访问、无长期 Git 凭证、无宿主机访问。需要的能力按 Run 临时授予。

### 3.4 可恢复优先

浏览器、API Gateway、Worker 任一连接中断，都不能导致已确认任务静默重复执行。所有写入操作使用幂等键，事件使用单调序号，Run 使用显式状态机。

### 3.5 人在回路

高风险动作必须支持审批：首次向外部域名发送数据、写敏感路径、运行高风险命令、push、创建 PR、修改 CI/权限文件等。组织管理员可配置策略。

## 4. 用户与核心场景

### 4.1 用户角色

| 角色 | 能力 |
|---|---|
| Visitor | 查看公开页面、注册登录 |
| Member | 使用组织已授权仓库、运行 Agent |
| Repo Maintainer | 管理仓库默认策略、环境、自动化 |
| Org Admin | 成员、角色、账单、密钥、审计、策略 |
| Platform Operator | 平台运维，不默认读取用户代码或 prompt |

### 4.2 核心用户旅程

#### 首次使用

1. 用户通过 GitHub OAuth 或邮箱注册。
2. 创建个人空间或加入组织。
3. 安装 GitHub App，选择允许访问的仓库。
4. 平台同步仓库元数据和默认分支。
5. 用户点击 New Agent，选择仓库与基线分支。
6. 输入任务，选择模型和环境，启动 Run。
7. Worker 创建隔离空间、获取短期 Git token、clone 代码、启动 `zene acp`。
8. 用户实时查看 Agent 行为并处理审批。
9. Agent 完成测试，用户检查 diff，授权 push 和创建 PR。

#### 回到后台 Run

1. 用户关闭浏览器，Run 继续执行。
2. 再次打开运行页，客户端带 `afterSeq` 拉取缺失事件。
3. 控制面返回 Run snapshot 和增量事件。
4. 若存在待审批，UI 恢复审批卡片。

#### 阅读代码并追加指令

1. 用户在右侧 Files 面板搜索符号或文件。
2. 点击结果打开代码阅读器。
3. 选中代码范围，添加为上下文。
4. 在 composer 中发送后续指令。
5. 控制面转成 ACP prompt content，并记录引用的 commit/path/range。

## 5. 产品信息架构

### 5.1 全局导航

桌面端采用左侧固定导航，窄屏切换抽屉：

- New Agent
- Agents
- Automations
- Repositories
- Pull Requests
- Team
- Settings

左下角显示头像、当前组织、套餐和账号菜单。

### 5.2 Dashboard

Dashboard 展示：

- 最近运行的 Agents，按 running / waiting / completed / failed 分类。
- 待处理审批。
- 最近 PR 与 CI 状态。
- 可用仓库和常用任务模板。
- 用量、并发额度和运行环境状态。

### 5.3 New Agent

页面中心为大输入框，必要参数保持简洁：

- Organization
- Repository
- Base branch
- Environment
- Model
- Permission mode
- Task prompt
- 可选附件、Issue/PR URL、代码引用

高级设置折叠：

- 是否创建新分支
- 是否允许 push
- 网络 allowlist
- 启动脚本
- 资源规格
- 最大运行时间

提交后立即创建 Run，路由跳转 `/agents/{runId}`。

### 5.4 Agent 运行页

采用三段式布局：

```text
┌──────────────┬──────────────────────────────┬──────────────────────┐
│ 左侧导航      │ 中央 Agent 时间线             │ 右侧 Inspector        │
│ Agents        │ 消息 / Tool / Permission     │ Diff / Files / Code   │
│ Repo / branch │ Composer / Stop / Steer      │ Terminal / Tests / PR │
└──────────────┴──────────────────────────────┴──────────────────────┘
```

中央时间线：

- User message
- Agent message
- Thought summary（默认折叠）
- Tool call 卡片
- Permission / AskUser 卡片
- Plan / Todo
- System event
- Checkpoint / handoff

右侧 Inspector tabs：

- Changes：按文件展示 staged/unstaged diff
- Files：仓库树、搜索、代码阅读
- Terminal：只读流式输出；有权限时输入
- Tests：测试命令和结果
- Git：branch、commit、push 状态
- PR：标题、描述、checks、review comments
- Context：模型、token、费用、上下文水位

顶部栏：

- 仓库、分支、Run 状态
- Worker 健康状态
- Share
- Stop / Restart
- Create PR
- More

### 5.5 状态表达

Run 状态必须用文字与颜色同时表达：

- Queued
- Provisioning
- Cloning
- Starting
- Running
- Waiting for approval
- Waiting for user
- Stopping
- Completed
- Failed
- Timed out
- Cancelled

不能用“在线/离线”代替具体故障。失败需展示错误分类、可重试性与 trace ID。

## 6. 总体架构

```text
┌────────────────────────────────────────────────────────────────────┐
│                           Client Layer                             │
│ Next.js Web │ Mobile Web │ Public API │ GitHub Checks / Webhooks   │
└───────────────────────────────┬────────────────────────────────────┘
                                │ HTTPS / SSE
┌───────────────────────────────▼────────────────────────────────────┐
│                         Edge / API Layer                           │
│ CDN │ WAF │ Rate Limit │ API Gateway │ Auth Session │ SSE Gateway │
└───────────────┬──────────────────────────────┬─────────────────────┘
                │                              │
┌───────────────▼────────────────┐  ┌──────────▼────────────────────┐
│         Control Plane          │  │      Git Integration Service  │
│ User / Org / Repo / Run API    │  │ GitHub App / Webhooks / PR    │
│ Policy / Billing / Audit       │  │ Short-lived installation token│
│ Scheduler / Reconciler         │  └──────────┬────────────────────┘
└──────┬───────────┬─────────────┘             │
       │           │                           │
       │      ┌────▼─────────┐        ┌────────▼────────┐
       │      │ Postgres     │        │ Object Storage  │
       │      │ Redis        │        │ artifacts/logs  │
       │      │ Event Stream │        │ snapshots       │
       │      └──────────────┘        └─────────────────┘
       │ RunSpec / commands
┌──────▼─────────────────────────────────────────────────────────────┐
│                       Worker Orchestration                         │
│ Queue │ Scheduler │ K8s controller / Firecracker manager          │
└──────┬─────────────────────────────────────────────────────────────┘
       │
┌──────▼──────────────── Isolated Run Boundary ──────────────────────┐
│ Agent Worker                                                        │
│ ┌──────────────┐   ACP NDJSON   ┌───────────────────────────────┐  │
│ │ Worker Agent │◄──────────────►│ zene acp                     │  │
│ │ Supervisor   │                │ core / tools / session / MCP  │  │
│ └──────┬───────┘                └──────────────┬────────────────┘  │
│        │                                       │                    │
│        │ event / command                       ▼                    │
│        │                              Keel strict sandbox           │
│        │                              Bash / Git / MCP processes    │
│ ┌──────▼───────┐  ┌─────────────┐  ┌──────────────────────────┐   │
│ │ Git workspace│  │ Local journal│  │ Egress proxy / Secret FD │   │
│ └──────────────┘  └─────────────┘  └──────────────────────────┘   │
└────────────────────────────────────────────────────────────────────┘
```

## 7. 组件设计

### 7.1 Web Application

建议技术：

- Next.js + TypeScript
- React Query 处理 server state
- Zustand 或 Redux Toolkit 处理 Run 实时状态
- Monaco Editor 或 CodeMirror 6 处理代码阅读
- Shiki 处理静态语法高亮
- 虚拟列表处理长时间线和大 diff
- Web Worker 处理 diff、ANSI 与搜索结果转换

职责：

- 登录、组织、仓库、Run、PR UI。
- REST/JSON 写操作。
- SSE 接收 Run 事件，断线后按序号续传。
- 本地 optimistic state，但不作为业务真相。
- Permission、AskUser、Stop、Retry 等命令携带幂等键。

不负责：

- 保存 provider secret。
- 直接访问 Git provider token。
- 直接连接 Worker。
- 在浏览器执行 Agent tool。

### 7.2 API Gateway / Backend for Frontend

职责：

- 验证 session/JWT。
- 解析 active organization。
- 执行 RBAC 和 repository ACL。
- 为 Web 聚合页面数据。
- 对写接口实施 CSRF、rate limit、idempotency。
- 为 SSE 连接签发短期 scoped token。

建议 API：

```text
POST   /v1/auth/*
GET    /v1/me
GET    /v1/organizations
GET    /v1/repositories
POST   /v1/repositories/sync

POST   /v1/runs
GET    /v1/runs
GET    /v1/runs/{runId}
POST   /v1/runs/{runId}/messages
POST   /v1/runs/{runId}/commands
POST   /v1/runs/{runId}/permissions/{requestId}
POST   /v1/runs/{runId}/cancel
POST   /v1/runs/{runId}/restart
GET    /v1/runs/{runId}/events?afterSeq=123&limit=500
GET    /v1/runs/{runId}/events/stream?afterSeq=123

GET    /v1/runs/{runId}/files
GET    /v1/runs/{runId}/file?path=...
GET    /v1/runs/{runId}/search?q=...
GET    /v1/runs/{runId}/diff
GET    /v1/runs/{runId}/terminals

POST   /v1/runs/{runId}/git/commit
POST   /v1/runs/{runId}/git/push
POST   /v1/runs/{runId}/pull-requests
GET    /v1/pull-requests/{id}
```

Agent 业务事件保持 ACP 载荷，不把 `tool_call`、`plan` 等重新定义成另一套格式：

```json
{
  "runId": "run_...",
  "seq": 124,
  "type": "acp",
  "createdAt": "2026-07-21T00:00:00Z",
  "payload": {
    "jsonrpc": "2.0",
    "method": "session/update",
    "params": {}
  }
}
```

平台事件使用独立 namespace：

```json
{
  "runId": "run_...",
  "seq": 125,
  "type": "platform",
  "payload": {
    "event": "git.push.completed",
    "commitSha": "..."
  }
}
```

### 7.3 Identity Service

支持：

- GitHub OAuth
- Google OAuth
- 邮箱 magic link
- Passkey/WebAuthn
- 企业 OIDC/SAML（后续）

Session：

- 浏览器使用 Secure、HttpOnly、SameSite=Lax/Strict cookie。
- Access session 短期有效，refresh session 支持轮换与撤销。
- API token 使用明确 scope，不与浏览器 session 共用。
- 敏感动作要求近期认证或 Passkey step-up。

组织权限：

```text
organization
  ├── owner
  ├── admin
  ├── member
  └── billing

repository grant
  ├── read
  ├── run
  ├── push
  └── manage
```

一次操作的有效权限是以下条件的交集，而不是任一条件成立：

```text
有效权限 =
  平台组织成员状态
  ∩ Run / Repository ACL
  ∩ GitHub App 当前 installation repository selection
  ∩ 操作级 read/run/push/manage
  ∩ 组织策略与有效审批
  ∩ 分享对象 scope
```

GitHub OAuth 登录身份与 GitHub App 仓库授权是两个独立对象，不能相互替代。成员被移除或 installation 被撤销时，立即撤销 session、SSE token 和未开始 Run；运行中的 Run 进入 `stopping`，Git 写权限即时失效。邀请必须绑定目标邮箱/身份并有过期时间。分享链接默认只读、短期、可撤销，且不能访问 terminal、secret、Git 写操作。

### 7.4 Git Integration Service

第一版使用 GitHub App，不使用用户 Personal Access Token。

GitHub App 权限建议：

- Repository metadata: read
- Contents: read/write
- Pull requests: read/write
- Checks: read
- Commit statuses: read
- Issues: read（可选，用于任务上下文）
- Actions: read（可选，用于展示 CI）

安全流程：

1. 用户安装 GitHub App。
2. 平台保存 installation ID，不保存 installation token。
3. clone/fetch 由 Git Broker 签发 repository-scoped、只读、短期凭证。
4. 只读凭证通过一次性 Secret channel 送入 Worker credential helper，不进入环境变量、命令行或 remote URL。
5. Worker 永不持有 Git 写凭证；即使恶意仓库攻破 Worker，也不能直接 push。
6. Worker 完成 commit 后，向 Git Broker 上传 Git bundle 或受校验的 commit/object 集合。
7. Git Broker 重新验证用户 `push` 权限、GitHub App installation、审批记录、目标仓库、目标分支和 `expectedHeadSha`。
8. Git Broker 使用独立的短期写 token 执行 push，并记录 before/after SHA。
9. PR 由 Git Broker 调用 provider API 创建，不把 provider token交给 Worker。

Git Broker 是 Git 写入的唯一可信边界。用户在 Agent prompt 中要求“直接 push”不能绕过平台结构化审批。

分支策略：

- 默认基于用户选择的 base SHA 创建 `zene/{user}/{run-slug}`。
- RunSpec 固定 `baseCommitSha`，避免默认分支移动导致任务不可复现。
- push 前 `git fetch` 并检查远端 branch 是否发生非预期更新。
- 禁止 force push，除非仓库管理员显式授权。
- commit author 使用用户身份或组织配置的 bot；committer 可使用 Zene Cloud Bot。
- 每个 Git 写操作记录 before/after SHA。
- 上传 bundle 前校验对象数量、总大小、commit parent、tree path 与目标仓库，拒绝替换无关历史。

PR 创建：

- Agent 可生成标题和描述草案。
- 用户策略决定自动创建还是人工确认。
- PR body 包含摘要、测试、风险、Agent Run 链接，不泄露 prompt/secret。
- 支持 draft PR。
- 记录 provider PR number、URL、head/base SHA。
- Webhook 同步 checks、review、merge/close 状态。

Webhook：

- 验证 GitHub HMAC signature。
- 先落库再异步处理。
- 使用 delivery ID 幂等。
- 支持 installation deleted、repository renamed/transferred、push、pull_request、check_suite。

### 7.5 Run Service

Run 是平台调度和授权的最小单位。

核心对象关系必须固定：

- **Run**：一个持久工作空间和 Git 分支的协作任务，可包含多个 Turn。
- **Turn**：一条用户 prompt 到 Agent 完成、取消或等待用户的执行周期。
- **Attempt**：承载同一 Run 的一次 Worker 执行实例；基础设施重试会创建新 Attempt。
- **ACP Session**：Zene Runtime 上下文，属于 Run；正常情况下跨 Attempt 恢复。
- **Workspace Generation**：工作区快照代次，防止旧 Attempt 写入新状态。

Run 进入 `completed` 表示当前没有活跃 Turn，不表示永久关闭。用户继续追问时创建新 Turn，并把 Run 从 `completed` 转回 `queued/running`；若 workspace 已过保留期，则显式 **Fork Run**，基于原 head SHA 创建新 Run。`cancel` 结束当前 Turn；`stop` 停止整个 Run；基础设施 `restart` 创建新 Attempt，不重新提交已确认 prompt。

RunSpec：

```json
{
  "runId": "run_01...",
  "organizationId": "org_...",
  "repositoryId": "repo_...",
  "requestedBy": "usr_...",
  "baseRef": "main",
  "baseCommitSha": "abc...",
  "branchName": "zene/pi/fix-login",
  "prompt": "...",
  "modelPolicyId": "model_policy_...",
  "environmentId": "env_...",
  "permissionPolicyId": "perm_...",
  "resourceClass": "standard",
  "timeoutSeconds": 7200
}
```

Run 状态机：

```text
created
  → queued
  → provisioning
  → cloning
  → starting
  → running
     ↔ waiting_for_approval
     ↔ waiting_for_user
     ↔ paused
  → stopping
  → completed | failed | timed_out | cancelled

completed
  → queued              # 保留期内新增 Turn

completed | failed
  → forked new Run      # 工作区已归档或用户要求新分支
```

约束：

- 状态只允许按显式 transition table 变化。
- 每次 transition 使用数据库 compare-and-swap version。
- Scheduler 至少一次投递，Worker start 必须幂等。
- 同一 Run 同时只能有一个 active Worker lease。
- 每个 Attempt 带单调递增 fencing generation；事件、命令 ack、snapshot 和副作用都必须携带 generation，旧 Attempt 的写入被拒绝。
- Worker lease 过期后 Reconciler 判断重连、恢复或失败，不能盲目重跑 prompt。

### 7.6 Scheduler 与 Reconciler

Scheduler：

- 检查用户权限、组织额度、仓库状态和并发限制。
- 选择 region、资源规格、镜像和环境模板。
- 生成不可变 RunSpec。
- 投递至 Worker queue。

Reconciler：

- 周期性比较期望状态与实际状态。
- 处理 Worker 心跳丢失、启动超时、停止超时。
- 对可恢复 Run 重新挂载 volume 并启动 Supervisor attach。
- 对不可恢复 Run 保存 patch/journal 并标记失败。

不要把调度逻辑放进 HTTP request 生命周期。

### 7.7 Agent Worker Supervisor

Worker 是 Zene 与云控制面的适配层，建议用 Rust 实现。

职责：

- 接收并验证签名 RunSpec。
- 准备 `/workspace`、`HOME`、`ZENE_HOME`。
- 通过 Git credential helper clone 指定 SHA。
- 启动 `zene acp`。
- 完成 ACP `initialize`、`session/new/load/resume`。
- 在 ACP NDJSON 与平台事件流之间桥接。
- 实现 permission/AskUser 的异步挂起与恢复。
- 实现 terminal capability，但所有命令必须在同一 sandbox boundary 内。
- 上报 heartbeat、resource usage、git status 与日志。
- 停止时保存 session、patch、journal 和 artifacts。

不直接复用公网形态的本地 `zene-gateway`，但可复用其：

- 子进程管理模式。
- NDJSON frame 处理。
- cursor/event journal 思想。
- restart/attach 测试方式。

Worker 内建议进程：

```text
PID 1 worker-supervisor
  ├── zene acp
  ├── keel-managed tool processes
  ├── git credential helper
  └── optional language/index services
```

ACP 映射：

- 平台 message → `session/prompt`
- Stop current turn → `session/cancel`
- 恢复会话 → `session/resume`
- 重建 UI → `session/load`
- 模式切换 → `session/set_mode`
- Agent update → event stream
- `session/request_permission` 是 Zene 发给 Supervisor 的反向 JSON-RPC request；Supervisor 先持久化其 JSON-RPC ID、request key 和 payload，等待平台审批，再以同一 ID 向 Zene 返回 response。

ACP Client 状态机：

```text
spawn
  → initialize
  → session/new | session/resume
  → ready
  → prompt active
     ↔ reverse request pending
  → ready
  → shutdown
```

`session/load` 会重放 UI 历史，`session/resume` 只恢复 Runtime 上下文，二者都依赖可访问的 Zene session 文件，不能单独承担 Worker 故障恢复。Supervisor 必须在一致性点保存 Zene session、event seq、workspace generation 和 Git SHA。未知 ACP notification 应原样持久化并安全忽略；协议版本和 capability 在每个 Attempt 启动时协商。

需要补充的 Zene 能力：

- 标准化 `session/steer`，避免只能取消后重发。
- 暴露 prompt queue 状态。
- 暴露 subagent lifecycle。
- Permission request 增加 stable request ID 与 deadline。
- ACP 启动路径必须完整应用 sandbox profile。

### 7.8 Keel 与隔离边界

必须采用两层隔离：

#### 第一层：租户边界

生产推荐 Firecracker microVM；早期可用以下方案之一：

- Kubernetes Pod + gVisor
- Kubernetes Pod + Kata Containers
- 单租户开发环境可用普通容器，但不能作为公开多租户生产边界

要求：

- 非 root 用户。
- 只读 rootfs。
- 独立 user/pid/mount/network namespace。
- 禁止 privileged、hostPath、Docker socket。
- seccomp + AppArmor/SELinux。
- cgroup CPU、内存、PID、IO 限制。
- workspace 专属 volume。
- metadata service 禁止访问。

#### 第二层：Keel

Worker 内默认：

- profile = `strict`
- workspace = `/workspace`
- network = deny-all
- allow_hosts 按环境和任务临时增加
- `auto_allow_bash = false`
- credential deny 覆盖 `~/.ssh`、云凭证、`.env*`、`~/.zene/config.toml`

重要限制：

- Keel 不能防御已被完全攻破的 Zene 主进程。
- 当前 Zene ACP 路径与 CLI sandbox 配置存在不一致，必须在云端上线前修复。
- 当前本地 Gateway terminal 直接 spawn 进程，会绕过 Keel；Cloud Worker 不能复用该实现。
- Glob/grep fallback 等所有文件读取路径应统一经过 sandbox policy。

### 7.9 Workspace Service

职责：

- 创建 workspace volume。
- clone/fetch/checkout 固定 commit。
- 应用环境模板与缓存。
- 提供受控的文件读取、搜索、diff 与 artifact 上传。

生命周期：

```text
empty volume
  → clone base SHA
  → create run branch
  → environment setup
  → agent changes
  → commit/push or patch
  → snapshot(optional)
  → retention expiry
```

缓存：

- Git object cache 按 provider/repo 隔离，只读挂载。
- Package cache 按生态和组织隔离，不共享用户 secret。
- 缓存 key 包含 lockfile digest、image version、arch。
- 不缓存 `.env`、credential、home。

### 7.10 Code Intelligence

第一版优先可靠的按需能力，不先建设全量语义索引：

- `git ls-tree` / filesystem tree
- ripgrep 全文搜索
- 文件内容与 blame
- diff 与 history
- tree-sitter 符号提取
- LSP 按 workspace 按需启动

第二阶段：

- 默认分支增量索引。
- symbol/definition/reference graph。
- embedding 语义搜索，按组织和 commit 隔离。
- PR 变更影响分析。

Code API 必须绑定：

- repository ID
- commit SHA 或 run workspace version
- canonical path
- 用户 ACL

禁止仅以磁盘绝对路径作为 API 参数。

### 7.11 Event Service

事件要求：

- 每个 Run 单调递增 `seq`。
- append-only。
- 支持 `afterSeq` 重放。
- 写入幂等。
- 支持 snapshot + delta，避免重放百万事件。
- 支持 retention 与归档。

建议：

- 初期 Postgres `run_events` 分区表即可。
- 高吞吐后引入 NATS JetStream 或 Kafka。
- Redis 只做 fanout/presence/lease，不做唯一持久真相。
- 大 tool output 存对象存储，事件只存引用和摘要。

事件分类：

- ACP events
- Run lifecycle
- Worker health
- Git operations
- Approval
- Artifact
- Billing usage
- Audit

SSE：

- 默认实时通道。
- `Last-Event-ID` 对应 seq。
- 心跳 15–30 秒。
- 代理缓冲关闭。
- 断开后指数退避。
- 仍提供普通分页 API 作为 fallback。

#### 命令—事件—副作用恢复协议

仅有 `seq` 只能恢复 UI，不能防止旧 Worker 或重试重复执行工具和 Git 写操作。所有控制命令进入持久 `run_commands`：

```text
accepted → delivered → acknowledged → executing → succeeded | failed | cancelled
```

命令字段至少包含 `commandId`、`runId`、`turnId`、`attemptGeneration`、`idempotencyKey`、`payloadHash`、`createdBy`。Worker ack 和结果必须携带 generation；数据库拒绝旧 generation。

Worker 事件携带稳定 `sourceEventId`，Event Service 以 `(run_id, attempt_generation, source_event_id)` 去重。Worker 仅在控制面确认 event checkpoint 后截断本地 journal。

Snapshot 必须记录：

- `lastEventSeq`
- `lastCommandId`
- `attemptGeneration`
- `workspaceGeneration`
- `zeneSessionObject`
- `workspaceSnapshotObject`
- `headCommitSha`

恢复裁决：

- pending approval：从数据库恢复，不重复向用户创建新 request。
- active tool：若工具无幂等保证则标记 `interrupted`，由用户决定继续；不能自动重跑。
- Git push/PR：只查询 Git Broker 的 operation 状态，绝不由 Worker重试。
- 旧 Worker恢复连接：fencing generation 不匹配，立即停止且拒绝其事件和副作用。
- snapshot 缺失或校验失败：保留 patch/journal，Run 标记为可诊断失败，不猜测恢复。

### 7.12 Artifact Service

对象：

- terminal log chunk
- test reports
- screenshots
- patches
- session exports
- workspace snapshots
- build artifacts

规则：

- 对象 key 带 org/run 前缀。
- DB 保存 owner、sha256、size、content type、retention。
- 下载使用短期 signed URL。
- 上传由 Worker 使用 scoped credential。
- 服务端做 size limit、MIME sniffing、malware scan。

## 8. 数据模型

核心表：

### 8.1 Identity

```text
users
  id, email, display_name, avatar_url, status, created_at

identities
  id, user_id, provider, provider_subject, metadata

sessions
  id, user_id, token_hash, expires_at, revoked_at

organizations
  id, slug, name, plan, created_at

organization_members
  organization_id, user_id, role, joined_at
```

### 8.2 Git

```text
git_installations
  id, organization_id, provider, external_installation_id, status

repositories
  id, organization_id, installation_id, provider_repo_id
  owner, name, default_branch, visibility, archived, last_synced_at

repository_grants
  repository_id, principal_type, principal_id, permission

repository_environments
  id, repository_id, name, setup_script, image_ref, policy_id
```

### 8.3 Run

```text
runs
  id, organization_id, repository_id, requested_by
  status, status_version, base_ref, base_sha, head_branch, head_sha
  model_policy_id, environment_id, permission_policy_id
  resource_class, region, created_at, started_at, finished_at

run_attempts
  id, run_id, attempt, worker_id, lease_expires_at
  generation, status, failure_code, started_at, finished_at

run_turns
  id, run_id, ordinal, prompt_message_id, status
  started_at, finished_at

run_commands
  id, run_id, turn_id, attempt_generation, command_type
  idempotency_key, payload_hash, payload, status, acknowledged_at, finished_at

run_messages
  id, run_id, client_message_id, author_id, content, created_at

run_events
  run_id, seq, attempt_generation, source_event_id
  event_type, payload_json, object_ref, created_at

run_snapshots
  id, run_id, last_event_seq, last_command_id
  attempt_generation, workspace_generation, session_object_key
  workspace_object_key, head_sha, created_at

approval_requests
  id, run_id, request_key, kind, risk, payload
  status, allowed_decisions, decision_scope, requested_at
  expires_at, resolved_by, resolved_at

artifacts
  id, organization_id, run_id, kind, object_key, sha256, size, retention_until

worker_leases
  run_id, attempt_id, generation, worker_id, expires_at

idempotency_records
  organization_id, scope, key, request_hash, response, expires_at

outbox_events
  id, aggregate_type, aggregate_id, event_type, payload, published_at
```

### 8.4 PR 与审计

```text
pull_requests
  id, repository_id, run_id, provider_number, url
  base_sha, head_sha, title, state, draft, created_at

audit_logs
  id, organization_id, actor_type, actor_id, action
  resource_type, resource_id, metadata, ip, user_agent, created_at

webhook_deliveries
  provider, delivery_id, installation_id, event_type
  payload_object_key, status, attempts, received_at, processed_at

git_operations
  id, organization_id, repository_id, run_id, operation
  expected_head_sha, result_head_sha, approval_id, status
  idempotency_key, provider_request_id, created_at, finished_at

usage_ledger
  id, organization_id, run_id, category, quantity, unit, cost, occurred_at
```

所有多租户表必须包含或可不可歧义地关联 `organization_id`。数据库查询层强制 tenant scope；可叠加 Postgres RLS。

关键唯一约束包括：`run_events(run_id, seq)`、`run_events(run_id, attempt_generation, source_event_id)`、`run_commands(run_id, idempotency_key)`、`webhook_deliveries(provider, delivery_id)`、`git_operations(repository_id, idempotency_key)`。环境、permission policy、模型 policy 在 Run 创建时保存不可变版本快照，避免管理员后续修改影响正在执行的 Run。

消息真相边界：`run_messages` 保存用户可见输入与引用；Zene Session 保存 Runtime 上下文。控制面不尝试从 `run_messages` 重建完整 LLM 上下文，恢复时使用已校验的 Zene session snapshot。

## 9. API 与命令模型

### 9.1 创建 Run

```json
POST /v1/runs
Idempotency-Key: 01J...

{
  "repositoryId": "repo_...",
  "baseRef": "main",
  "prompt": "修复登录失败并添加测试",
  "environmentId": "env_...",
  "modelPolicyId": "model_...",
  "permissionPolicyId": "perm_..."
}
```

服务端：

1. 验证 repo `run` 权限。
2. 解析 base ref 到不可变 SHA。
3. 检查配额与并发。
4. 生成 branch name。
5. 创建 Run + outbox event。
6. 异步排队。

### 9.2 向运行中 Agent 发消息

```json
POST /v1/runs/{runId}/messages
Idempotency-Key: 01J...

{
  "clientMessageId": "msg_local_...",
  "text": "先不要改 API，只修前端",
  "references": [
    {
      "repositoryId": "repo_...",
      "commitSha": "abc...",
      "path": "src/login.ts",
      "startLine": 20,
      "endLine": 45
    }
  ]
}
```

Run workspace 已归档或处于不可恢复状态时返回明确冲突，并提示 fork：

```json
{
  "error": "run_not_accepting_messages",
  "status": "archived",
  "recovery": "fork_run",
  "retryable": false
}
```

保留期内的 `completed` Run 接收消息会创建新 Turn，并异步重新调度 Attempt。

### 9.3 审批

```json
POST /v1/runs/{runId}/permissions/{requestId}
Idempotency-Key: 01J...

{
  "decision": "allow_once",
  "comment": "只允许本次"
}
```

服务端必须验证：

- request 属于 run。
- 用户对 repo 有 run 权限。
- request 仍 pending。
- 决策在服务端允许集合内。
- `allow_always` 是否被组织策略允许。

### 9.4 Git 写操作

Git 写操作不能只依赖 prompt 中“请创建 PR”，必须转成结构化平台命令：

```json
POST /v1/runs/{runId}/pull-requests
Idempotency-Key: 01J...

{
  "title": "fix: handle expired sessions",
  "body": "...",
  "draft": true,
  "expectedHeadSha": "def..."
}
```

`expectedHeadSha` 防止用户审查后代码发生变化。

### 9.5 通用 API 契约

- 所有列表使用 cursor pagination。
- 所有异步写操作返回 `operationId` 和当前状态。
- 所有错误包含稳定 `error`、人类可读 `message`、`retryable`、`traceId`。
- `409` 用于状态/CAS/幂等冲突，`412` 用于 `expectedHeadSha` 等前置条件失败。
- Run 修改接口携带 `expectedStatusVersion`。
- Event history 被归档时返回 `snapshotRequired`、snapshot ID 和首个可用 seq。
- Terminal control 使用独立 controller lease，并支持 input、resize、kill；只读 viewer 无 input 权限。
- SSE token 通过 Authorization header 或安全 cookie 传递，不放 URL query；token 只包含单个 Run 的 read scope，短期有效且可撤销。
- `POST /v1/runs/{runId}/fork` 创建新 Run；`POST /messages` 在保留期内创建新 Turn。
- completed workspace 不存在时 Files/Diff API 从最终 snapshot、patch 或 commit 读取，并明确返回数据来源。

## 10. 安全设计

### 10.1 威胁模型

必须假设：

- 仓库内容恶意。
- Prompt injection 存在于代码、Issue、网页和 tool output。
- Agent 会尝试运行危险命令。
- 用户可能越权访问其他组织。
- Worker 可能被攻破。
- Git/Webhook/LLM provider 可能临时不可用。

### 10.2 租户隔离

- 每个 Run 独立 sandbox boundary。
- Worker 使用唯一 workload identity。
- 数据库和对象存储按 organization scope 授权。
- 事件订阅在服务端做 run ownership 验证。
- 禁止用户控制 host mount、namespace、runtime flags。

### 10.3 Secret

- Secret 存 KMS-backed secret manager。
- Worker 获取短期 scoped secret。
- 优先通过 FD、Unix socket 或 credential helper 传递，不使用命令参数。
- 日志层统一 redact token、Authorization、Cookie、私钥。
- Agent 无权列出组织全部 secret。
- 用户配置的环境变量按名称 allow/deny policy 检查。

### 10.4 网络

- Worker 默认 deny-all egress。
- LLM 流量通过平台代理，便于隐藏 provider key、计费与审计。
- GitHub、包管理器、MCP 按域名和端口 allowlist。
- DNS 解析结果防 rebinding。
- 禁止 link-local、RFC1918、metadata endpoint，除非平台内部明确服务。
- HTTP fetch 限制响应体大小、重定向次数和内容类型。

### 10.5 Git 安全

- checkout 前检查 submodule、LFS、hooks 策略。
- 默认禁用 repository hooks。
- 不执行来自仓库的 credential helper。
- Git 配置使用受控 system/global config。
- Push 目标必须与已授权 repository ID 对应。
- branch protection 由 provider 保持，不绕过。
- 创建 PR 前重新确认 head SHA。

### 10.6 Web 安全

- CSP，禁用 inline script。
- `frame-ancestors 'none'`。
- CSRF token + SameSite cookie。
- HTML/Markdown 全部 sanitization。
- ANSI parser 禁用 OSC 52 等敏感控制。
- signed URL 短期有效且校验 owner。
- OAuth state + PKCE。
- Webhook HMAC。

### 10.7 审计

记录：

- 登录、身份绑定、组织成员变化。
- GitHub App 安装与权限变化。
- Run 创建、停止、恢复。
- Permission request 与决策。
- Secret 使用元数据，不记录值。
- Git clone/push/commit/PR。
- 管理员策略修改。
- 数据导出与删除。

## 11. 可靠性与一致性

### 11.1 Outbox

数据库写入与异步事件使用 transactional outbox：

```text
BEGIN
  INSERT runs ...
  INSERT outbox_events ...
COMMIT
```

Publisher 将 outbox 投递到 queue，消费者按 event ID 幂等。

### 11.2 幂等

需要幂等的动作：

- 创建 Run
- 发送消息
- Permission response
- Worker start
- Git commit/push
- PR create
- Webhook consume

每个动作保存 idempotency key、request hash 和首次结果。相同 key 不同 request body 返回冲突。

### 11.3 Worker 失败

分类：

- Provision failure：可换节点重试。
- Clone failure：按错误类型重试或要求重新授权。
- Runtime crash：保留 workspace 与 journal，尝试 resume。
- Node lost：从 volume/snapshot 恢复，不能重复已确认 Git 写操作。
- Policy violation：立即停止并标记不可自动重试。

### 11.4 数据保留

默认建议：

- Run 元数据：长期。
- Event：30–90 天，之后归档。
- Terminal 全量：7–30 天。
- Workspace：Run 完成后 24–72 小时。
- Patch/commit/PR metadata：长期。
- Secret access log：按合规要求。

组织管理员可选择更短策略。

## 12. 可观测性

统一字段：

- trace_id
- request_id
- organization_id
- user_id
- repository_id
- run_id
- attempt_id
- worker_id
- event_seq

指标：

- Run queue latency
- Provision/clone/start latency
- Active/waiting/completed/failed Runs
- Worker heartbeat loss
- Agent turn latency
- Tool success/failure
- Approval wait time
- Event lag
- SSE connections/reconnects
- Git operation latency/failure
- Token/费用
- CPU/memory/disk/network

日志默认不记录：

- prompt 全文
- 代码全文
- terminal 全量
- tool result 全量
- secret

需要调试时由用户显式生成脱敏诊断包。

### 12.1 初始 SLO 与容量边界

- Control API 月可用性：99.9%。
- 已排队 Run 的调度 p95：30 秒内；资源紧张时明确显示容量等待。
- Worker 启动到首个 Agent 事件 p95：120 秒内，不含用户自定义 setup。
- 已连接 SSE 的事件可见延迟 p95：2 秒内。
- 已确认事件 RPO：0；控制面元数据 RPO：5 分钟以内；单 region 恢复 RTO：60 分钟以内。
- cancel 后 Worker 开始停止 p95：10 秒，资源强制回收上限：2 分钟。
- 单 Run 事件 payload：256 KiB；大输出转对象存储。
- 单 Run terminal 保留：默认 64 MiB；artifact、磁盘、PID、CPU、内存均按 resource class 限额。

降级策略：

- Redis 故障：禁止新 lease/实时 fanout，已有数据从 Postgres 拉取；不丢业务真相。
- S3 故障：暂停依赖 artifact/snapshot 的完成与恢复，不丢弃本地 journal。
- GitHub 故障：Agent 可继续本地工作，push/PR operation 保持 pending 并按 retry budget 重试。
- LLM 故障：Turn 进入 retrying，超过预算后等待用户；禁止无限重试计费。
- Postgres 故障：停止接受写操作和新 Run，已有 Worker 进入安全暂停并保留本地 journal。
- poison command/event 进入 DLQ，不能阻塞整个 Run 分区。

每月执行数据库备份恢复演练；每季度执行 Worker 丢失、消息重复、GitHub 超时和对象存储不可用的故障注入。

## 13. 技术选型建议

### 13.1 初始可运行版本

| 领域 | 建议 |
|---|---|
| Web | Next.js / TypeScript |
| Control API | Rust Axum |
| Worker Supervisor | Rust |
| Database | PostgreSQL 16+ |
| Cache / lease | Redis |
| Queue | PostgreSQL outbox + Redis Streams，或 NATS JetStream |
| Object storage | S3 compatible |
| Runtime | Kubernetes |
| Sandbox | gVisor/Kata + Keel |
| Auth | 自建 session + GitHub OAuth，或成熟 OIDC provider |
| Telemetry | OpenTelemetry + Prometheus + Loki/Tempo |

### 13.2 为什么控制面建议 Rust

- 与 Zene/ACP 类型和工具链一致。
- Worker Supervisor 可复用协议与测试代码。
- 对高并发 SSE、进程管理和安全边界更合适。

如果团队更熟悉 TypeScript，也可以使用 Next.js BFF + Rust Run/Worker services；不建议把 Worker Supervisor 写成 Node 子进程拼装。

## 14. 建议仓库结构

当前目录未来独立为 `zene-cloud`：

```text
zene-cloud/
  apps/
    web/                 # Next.js
    api/                 # BFF / public API
    control-plane/       # Run/repo/policy services
    worker-supervisor/   # ACP bridge
  crates/
    domain/
    auth/
    git-provider/
    event-store/
    acp-cloud/
    policy/
  deploy/
    docker/
    helm/
    terraform/
  docs/
    PRODUCT_AND_SYSTEM_DESIGN.md
    adr/
    threat-model/
    runbooks/
  migrations/
  tests/
    contract/
    integration/
    e2e/
```

Zene 集成方式：

- 早期：Worker image 中安装固定版本的 `zene` binary。
- 稳定后：将 ACP schema 抽为独立 crate；Supervisor 依赖 schema，不直接依赖全部 Zene core。
- 每次升级 Zene 跑 ACP contract suite。
- RunSpec 记录 `zeneVersion`、Worker image digest 和 Keel version。

## 15. 分阶段实施

### Phase 0：架构验证

范围：

- 单用户。
- GitHub OAuth。
- 一个 GitHub 仓库。
- Control API 创建 Run。
- 本地 Docker/K8s Worker 启动 `zene acp`。
- SSE 展示消息和 tool。
- 手动停止。

验收：

- 浏览器关闭后 Run 继续。
- 重开页面无事件重复或丢失。
- 固定 SHA clone。
- Worker 无宿主机目录访问。

### Phase 1：可用 MVP

范围：

- 用户/组织。
- GitHub App 与 repo picker。
- gVisor 或 Kata 强隔离 runtime；Keel strict 作为第二层。
- 只读 clone token + 服务端 Git Broker 写入。
- 受控 terminal、deny-all egress 与按 Run allowlist。
- New Agent、Agents 列表、Run 页面。
- Permission/AskUser。
- Changes/Files/Terminal。
- commit/push/draft PR。
- Postgres、Redis、S3。
- 基础配额与审计。

验收：

- 两个租户无法互相访问任何 Run、事件、artifact。
- 运行真实仓库任务并创建 PR。
- Worker 崩溃后保留 patch 与事件。
- Git token 不出现在环境、日志、remote URL。

### Phase 2：团队可用

范围：

- RBAC。
- 环境模板和 setup cache。
- Automations。
- PR checks/review 同步。
- cursor 过期 snapshot recovery。
- Run restart/resume。
- 代码搜索与符号索引。
- 用量与账单。

### Phase 3：生产强化

范围：

- Firecracker（如规模与风险需要从 gVisor/Kata 继续提升）。
- 多 region。
- 分布式 event stream。
- 企业 OIDC/SAML。
- Policy as code。
- BYOK/LLM gateway。
- GitLab。
- 灾备和合规。

## 16. 测试策略

### 16.1 单元测试

- Run 状态机。
- RBAC。
- branch name。
- webhook signature。
- idempotency。
- event sequence。
- policy evaluation。
- secret redaction。

### 16.2 Contract 测试

- Zene ACP initialize/session/prompt/cancel/load/resume。
- permission reverse request。
- terminal capability。
- session update schema。
- Zene 版本兼容矩阵。

### 16.3 集成测试

- GitHub App mock + clone/push/PR。
- Postgres outbox。
- Redis lease。
- Worker provision/heartbeat/stop。
- event replay。
- object storage signed upload/download。

### 16.4 安全测试

- 跨 tenant IDOR。
- symlink/path traversal。
- malicious repository hooks。
- prompt injection 导致 secret/metadata 请求。
- egress bypass。
- terminal escape。
- fork bomb / disk fill / output flood。
- webhook replay。

### 16.5 端到端场景

1. 注册并安装 GitHub App。
2. 选择仓库并创建 Run。
3. Agent 读取代码、修改、运行测试。
4. 浏览器断开并恢复。
5. 批准一次 Bash。
6. 查看代码和 diff。
7. commit、push、创建 draft PR。
8. GitHub webhook 更新 checks。
9. 第二个用户只读查看，不能抢占写权限。
10. 终止 Run，确认资源释放和 artifact 保留。

## 17. 上线门槛

以下条件全部满足才允许公开多租户：

- Worker 使用强隔离 runtime。
- Keel ACP 路径配置一致性已修复。
- Terminal 不绕过 sandbox。
- Git credential 不进入 Agent 可读环境。
- 跨租户授权测试通过。
- 事件恢复、Worker lease 和 Git 写幂等通过故障注入。
- WAF、rate limit、CSP、CSRF、OAuth/Webhook 校验完成。
- 数据删除与 retention 可执行。
- 审计和 incident runbook 完成。
- 至少一次第三方安全评审。

## 18. 当前 Zene/Keel 差距与前置工作

现有 Zene 已具备 Agent loop、ACP、session、tools、MCP、permission、Plan、Todo、subagent 和 streaming，适合作为 Runtime。但云端前必须处理：

| 差距 | 处理 |
|---|---|
| 本地 Gateway 只有共享 token，无用户/租户 | 新建 Cloud Control Plane，不公网复用本地认证 |
| Gateway 状态主要在内存/本地文件 | 外置事件、Run、lease |
| ACP sandbox 未完全复用 CLI sandbox options | 修复 Zene ACP 启动路径 |
| Gateway terminal 直接 spawn，绕过 Keel | Cloud Worker 内重写 terminal host |
| `sandboxProfile` 在本地 Gateway 创建请求中未落地 | RunSpec 强制 policy，Worker 验证 |
| `session/steer` 未暴露 | 扩展 ACP 或先 cancel + new prompt |
| subagent 生命周期不完整暴露 | 增加结构化事件 |
| prompt queue 无完整 UI 状态 | 增加 queue updates |
| 本地 session 使用 `~/.zene` | 每 Run 独立 `ZENE_HOME`，结束后归档 |
| Keel 无租户级 CPU/内存/磁盘边界 | 使用 microVM/container cgroup |

## 19. 需要评审的关键决策

建议本轮先确认以下方向：

1. 产品首先面向个人开发者，还是从一开始支持组织和团队。
2. 第一版只支持 GitHub Cloud，还是必须支持 GitLab/self-hosted。
3. Worker 第一版采用 Kubernetes + gVisor，还是直接 Firecracker。
4. 用户使用平台统一模型，还是第一版就支持 BYOK。
5. Agent 是否默认允许 push/PR，还是始终二次确认。
6. Workspace 完成后保留多久，是否支持 Resume。
7. 是否需要公开 API 与 Automations 进入第一版。
8. Web UI 是在现有 `apps/web-agent` 上重构，还是新建 Next.js；本方案建议新建。

## 20. 推荐结论

推荐以“单体控制面 + 独立 Worker”的模块化架构启动：

- Web 与 Control API 可以先单仓库部署。
- Postgres 作为核心真相与初期 event store。
- Redis 只做 lease、缓存和实时 fanout。
- 每个 Run 一个 gVisor/Kata Pod，内部运行 Supervisor + `zene acp` + Keel。
- GitHub App 作为唯一 Git provider。
- SSE 作为默认实时通道，分页事件作为恢复通道。
- 新建 Next.js UI，不继续扩展当前零构建本地页面。
- MVP 完成后再拆服务和引入更重的消息系统。

这个方案能以较低复杂度得到可运行产品，同时保留向 Cursor 类完整 Cloud Agent 平台演进的边界。
