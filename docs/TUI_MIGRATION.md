# 从 TUI / 本地 Web Agent 到 CLI 与 Cloud

Zene 已移除 ratatui TUI 与本地 Web Agent（`zene-gateway` / `apps/web-agent`）。

## 本地交互

| 场景 | 命令 |
|------|------|
| 交互 REPL | `zene` 或 `zene --repl` |
| 无头单次提示 | `zene -p "…"` |
| ACP（编辑器 / Cloud worker） | `zene acp` |

## Cloud Console

浏览器产品界面在 `cloud/apps/web/`，由 `zene-cloud-api` 服务；生产见 `cloud/deploy/`。

## 清理

若本机仍有旧二进制，可删除 `~/.local/bin/zene-gateway` / `~/.cargo/bin/zene-gateway`，以及运行时目录 `~/.zene/gateway`。
