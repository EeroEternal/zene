# Phase 0 实现说明

## 范围

对应设计文档 Phase 0：架构验证垂直切片。

已落地：

1. Control Plane API（Axum + SQLite）
2. Worker Supervisor（claim/heartbeat/events）
3. Mock Agent 事件回放
4. Cursor 风格 Web：登录、New Agent、Agents、Run 时间线

刻意延后：

- GitHub OAuth / GitHub App
- Git Broker 写路径
- 强隔离 runtime（gVisor/Kata）
- 真实 ACP stdout 多路复用
- Next.js 重构
- Postgres / Redis

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
