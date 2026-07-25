# Zene Cloud

多用户 Cloud Coding Agent 控制面与 Worker。设计文档见仓库内 `zene-cloud-platform/docs/`（若已合并）或对应 PR。

## 当前能力（可本地完整演示）

- 用户注册 / 登录与组织
- GitHub 集成（默认 **live**；凭证可在 Settings 页面配置，也可用 env 覆盖）
- Repository 同步 / 选择
- Run 生命周期：创建、消息、取消、事件
- Worker：clone（mock workspace 或真实 git）、默认真实 `zene acp`（缺二进制且 `ZENE_CLOUD_ALLOW_MOCK=1` 时才 MockAgent）
- 用户 BYOK LLM（Settings → 注入 `zene acp` 环境变量）
- Permission / AskUser 审批
- Files / Diff / Push / Draft PR（Git Broker，mock 或 live）
- Cursor 风格 Web UI

## 快速启动

```bash
cd cloud
./scripts/dev.sh
```

`dev.sh` 会探测或构建仓库根的 `zene`（`../target/debug/zene`），并以真实 ACP 启动 worker。

Web UI 源码在 `apps/web/`（Next.js + Tailwind CSS，静态导出）。修改后重新构建并提交 `dist/`：

```bash
cd cloud/apps/web
npm install
npm run build   # next build && 导出到 dist/（API 直接静态托管）
```

打开 http://127.0.0.1:8788/

推荐演示路径：

1. 注册账号
2. **Settings** 配置 LLM（API key + base URL，如 DeepSeek / Custom）
3. Connect GitHub
4. New Agent 选择仓库，输入任务并 Start
5. 在 Run 页查看消息、审批、Files、Changes、PR

## 环境变量

| 变量 | 默认 | 说明 |
|------|------|------|
| `ZENE_CLOUD_GITHUB_MODE` | `live` | `live` 或 `mock` |
| `ZENE_CLOUD_WORKER_TOKEN` | `dev-worker-token` | Worker 鉴权 |
| `ZENE_BIN` | 自动探测/构建 | 真实 `zene` 路径 |
| `ZENE_CLOUD_ALLOW_MOCK` | `0`（dev.sh 为 `1`） | 无 `zene` 时是否允许 MockAgent |
| `ZENE_CLOUD_ACP_YOLO` | `1`（dev.sh） | 真实 ACP 自动批准工具 |
| `ZENE_CLOUD_ACP_IDLE_SECS` | `600` | 主轮次结束后保留会话以接收 follow-up |
| `ZENE_CLOUD_PUSH_PR` | `1` | 完成后自动 push + draft PR |
| `GITHUB_CLIENT_ID/SECRET` | — | live OAuth |
| `GITHUB_APP_ID` / `GITHUB_APP_PRIVATE_KEY_PATH` | — | live App |
| 用户 Settings LLM / `ZENE_API_KEY` | — | 真实 ACP 需要 LLM（优先 per-user BYOK） |

## Live GitHub（Cursor 同款流程）

用户点 **Connect GitHub** 后会跳转到 `github.com/apps/<slug>/installations/new`，用浏览器里已登录的 GitHub 账号授权（与 Cursor 相同）。

**部署者**一次性配置 GitHub App（用户界面不展示凭证）：

```bash
export GITHUB_APP_ID=...
export GITHUB_APP_PRIVATE_KEY_PATH=/path/to/key.pem
export GITHUB_APP_SLUG=your-app-slug
```

GitHub App 的 **Setup URL** 设为：本地 `http://127.0.0.1:8788/api/v1/github/install/callback`；生产 `https://zene.run/api/v1/github/install/callback`。

生产部署见 [`deploy/README.md`](deploy/README.md)。

强制 mock：`export ZENE_CLOUD_GITHUB_MODE=mock`

## 目录

```text
apps/api          Control Plane
apps/worker       Supervisor + ACP
apps/web/dist     Web UI
crates/domain
crates/db
crates/acp-bridge
crates/github
crates/git-broker
migrations/
```

## 测试

```bash
cargo test --workspace
cargo build -p zene-cloud-api -p zene-cloud-worker
```
