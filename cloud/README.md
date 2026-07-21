# Zene Cloud（Phase 0）

本目录是可独立演进的 Cloud Coding Agent 控制面与 Worker 实现，对应设计文档：

- [`../zene-cloud-platform/docs/PRODUCT_AND_SYSTEM_DESIGN.md`](../zene-cloud-platform/docs/PRODUCT_AND_SYSTEM_DESIGN.md)

Phase 0 目标：本地可运行的垂直切片。

## 已实现

- 用户注册 / 登录（邮箱密码）
- Organization / Repository
- Run 创建、消息、事件、取消
- Worker claim / heartbeat / event / status
- Cursor 风格 Web UI（New Agent、Agents 列表、Run 时间线）
- Mock Agent（无 `zene` 时也能演示完整链路）

## 目录

```text
cloud/
  apps/api       # Control Plane HTTP API + 静态 Web
  apps/worker    # Worker Supervisor
  apps/web/dist  # Phase 0 Web UI
  crates/domain
  crates/db
  crates/acp-bridge
  migrations
```

## 快速启动

```bash
cd cloud
./scripts/dev.sh
```

浏览器打开：

```text
http://127.0.0.1:8788/
```

默认：

- API: `127.0.0.1:8788`
- SQLite: `./data/zene-cloud.db`
- Worker token: `dev-worker-token`
- Workspaces: `./data/workspaces`

## 手动启动

```bash
cd cloud
cargo run -p zene-cloud-api
# 另一个终端
cargo run -p zene-cloud-worker
```

如需真实 `zene acp`：

```bash
# 显式指定（推荐）
export ZENE_BIN=/workspace/target/debug/zene
# 可选：本地自动批准工具（等价于 zene acp --yolo）
export ZENE_CLOUD_ACP_YOLO=1
# 也需要 LLM key / mock base URL，例如：
# export ZENE_BASE_URL=http://127.0.0.1:9xxx
# export ZENE_API_KEY=sk-...
cargo run -p zene-cloud-worker
```

Worker 会自动发现常见路径（`ZENE_BIN`、`/workspace/target/debug/zene`、`../target/debug/zene`、PATH 上的 `zene`）。找不到二进制时回退到增强版 MockAgent（permission / tool_call / 多文件变更）。

## GitHub 集成（mock / live）

默认 **mock** 模式，无需真实 GitHub 凭证：

```bash
export ZENE_CLOUD_GITHUB_MODE=mock   # 默认值，可省略
```

代码侧：

```rust
use zene_cloud_github::GithubClient;
use zene_cloud_git_broker::GitBroker;

let github = GithubClient::mock();           // 或 GithubClient::from_env()
let broker = GitBroker::mock(db.clone());    // issue_read_clone_token / accept_bundle_and_push / create_draft_pr
```

切换 live：

```bash
export ZENE_CLOUD_GITHUB_MODE=live
export GITHUB_CLIENT_ID=...
export GITHUB_CLIENT_SECRET=...
export GITHUB_APP_ID=...
export GITHUB_APP_PRIVATE_KEY_PATH=/path/to/app.pem
export GITHUB_APP_SLUG=your-app-slug
```

相关 crate：`zene-cloud-github`、`zene-cloud-git-broker`；DB migration `002_github_git.sql`。

## 下一步

- API 路由挂载 GitHub OAuth / installation sync / approvals
- gVisor/Kata Worker
- 真实 ACP stdout pump / permission 异步审批
- Next.js 产品前端替换零构建页面
- Postgres / Redis 替换 SQLite
