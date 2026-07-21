# Phase 0 实现说明

## 范围

对应设计文档 Phase 0：架构验证垂直切片。

已落地：

1. Control Plane API（Axum + SQLite）
2. Worker Supervisor（claim/heartbeat/events）
3. Mock Agent 事件回放
4. Cursor 风格 Web：登录、New Agent、Agents、Run 时间线

刻意延后：

- GitHub OAuth / GitHub App（脚手架进行中）
- Git Broker 真实写路径
- 强隔离 runtime（gVisor/Kata）
- Next.js 重构
- Postgres / Redis

已补齐 Worker ↔ ACP：

- `acp-bridge`：spawn `zene acp`、stdout NDJSON 泵、按 id 匹配 JSON-RPC response、转发 `session/update`、处理 `session/request_permission`
- Worker：clone-auth → workspace clone/mock → 真实 ACP 或 MockAgent → events / approvals / commands / commit
- Internal API：`/clone-auth`、`/commands`、`/approvals`、stub `/git/push|pull-request`

## 本地验证路径

1. `./scripts/dev.sh`
2. 打开 `http://127.0.0.1:8788`
3. 注册用户
4. 添加仓库 `owner/name`
5. New Agent 输入任务并 Start
6. 观察左侧 Agents、中央消息、右侧事件

## API 摘要

- `POST /api/v1/auth/register|login`
- `GET /api/v1/me`
- `GET|POST /api/v1/repositories`
- `GET|POST /api/v1/runs`
- `GET /api/v1/runs/{id}`
- `GET|POST /api/v1/runs/{id}/messages`
- `GET /api/v1/runs/{id}/events`
- `POST /api/v1/runs/{id}/cancel`
- `POST /internal/v1/runs/claim`
- `POST /internal/v1/runs/{id}/heartbeat|events|status`
- `GET|POST /internal/v1/runs/{id}/clone-auth`
- `GET /internal/v1/runs/{id}/commands`
- `POST /internal/v1/runs/{id}/approvals` + `GET .../approvals/{approvalId}`
- `POST /api/v1/runs/{id}/approvals/{approvalId}/decide`
- `POST /internal/v1/runs/{id}/git/push|pull-request`（Phase 0 stub）
