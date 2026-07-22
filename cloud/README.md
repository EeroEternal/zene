# Zene Cloud

多用户 Cloud Coding Agent 控制面与 Worker。设计文档见仓库内 `zene-cloud-platform/docs/`（若已合并）或对应 PR。

## 当前能力（可本地完整演示）

- 用户注册 / 登录与组织
- GitHub 集成（默认 **live**；凭证可在 Settings 页面配置，也可用 env 覆盖）
- Repository 同步 / 选择
- Run 生命周期：创建、消息、取消、事件
- Worker：clone（mock workspace 或真实 git）、真实 `zene acp` 或 MockAgent
- Permission / AskUser 审批
- Files / Diff / Push / Draft PR（Git Broker，mock 或 live）
- Cursor 风格 Web UI

## 快速启动

```bash
cd cloud
./scripts/dev.sh
```

打开 http://127.0.0.1:8788/

推荐演示路径：

1. 注册账号
2. 点击 **Connect GitHub (mock)**
3. New Agent 选择仓库，输入任务并 Start
4. 在 Run 页查看消息、审批、Files、Changes、PR

## 环境变量

| 变量 | 默认 | 说明 |
|------|------|------|
| `ZENE_CLOUD_GITHUB_MODE` | `live` | `live` 或 `mock` |
| `ZENE_CLOUD_WORKER_TOKEN` | `dev-worker-token` | Worker 鉴权 |
| `ZENE_BIN` | 自动探测 | 真实 `zene` 路径；缺失则 MockAgent |
| `ZENE_CLOUD_ACP_YOLO` | `1`（dev.sh） | 真实 ACP 自动批准工具 |
| `ZENE_CLOUD_PUSH_PR` | `1` | 完成后自动 push + draft PR |
| `GITHUB_CLIENT_ID/SECRET` | — | live OAuth |
| `GITHUB_APP_ID` / `GITHUB_APP_PRIVATE_KEY_PATH` | — | live App |
| `ZENE_API_KEY` 或 `ZENE_BASE_URL` | — | 真实 ACP 需要 LLM |

## Live GitHub（Cursor 同款流程）

用户点 **Connect GitHub** 后会跳转到 `github.com/apps/<slug>/installations/new`，用浏览器里已登录的 GitHub 账号授权（与 Cursor 相同）。

**部署者**一次性配置 GitHub App（用户界面不展示凭证）：

```bash
export GITHUB_APP_ID=...
export GITHUB_APP_PRIVATE_KEY_PATH=/path/to/key.pem
export GITHUB_APP_SLUG=your-app-slug
```

GitHub App 的 **Setup URL** 设为：`http://127.0.0.1:8788/api/v1/github/install/callback`

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
