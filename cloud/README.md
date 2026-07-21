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
export ZENE_BIN=/path/to/zene
cargo run -p zene-cloud-worker
```

当前 Phase 0 worker 仍默认走 mock prompt 路径，保证本地演示稳定；ACP 桥接 crate 已预留。

## 下一步（Phase 1）

- GitHub App + Git Broker
- gVisor/Kata Worker
- 真实 ACP stdout pump / permission 异步审批
- Next.js 产品前端替换零构建页面
- Postgres / Redis 替换 SQLite
