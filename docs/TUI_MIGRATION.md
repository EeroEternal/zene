# 从 TUI / 本地 Web Agent / REPL 到 Cloud 与 ACP

Zene 已移除 ratatui TUI、本地 Web Agent（`zene-gateway` / `apps/web-agent`），以及用户向本地 REPL / headless `-p`。

## 产品入口

| 场景 | 方式 |
|------|------|
| 产品 UI | Cloud Console：`cd cloud && ./scripts/dev.sh` → http://127.0.0.1:8788/ |
| ACP（Cloud worker / 编辑器） | `zene acp`（可选 `--yolo` 自动批准工具） |
| 运维辅助 | `zene sessions` / `zene config` / `zene export` / `zene mcp doctor` |

## Cloud Console

浏览器产品界面在 `cloud/apps/web/`，由 `zene-cloud-api` 服务；生产见 `cloud/deploy/`。Worker 通过 `ZENE_BIN` 启动真实 `zene acp`。

## 清理

若本机仍有旧二进制，可删除 `~/.local/bin/zene-gateway` / `~/.cargo/bin/zene-gateway`，以及运行时目录 `~/.zene/gateway`。
