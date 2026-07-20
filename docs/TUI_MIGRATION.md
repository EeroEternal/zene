# 从 TUI 迁移到 Web Agent

Zene 已移除 ratatui TUI。默认交互入口是本地 Web Agent（`zene-gateway`）。

## 怎么启动

```bash
zene                      # 等同于 zene web（默认）
zene web --yolo --sandbox-off
zene -p "fix the suite" # headless
zene acp                  # 编辑器 / Gateway 用的 ACP stdio
zene --repl               # 调试用行 REPL（非默认 UI）
```

旧命令 `zene --tui` 会退出并提示改用 Web。

## 能力对照

| 原 TUI | Web Agent |
|--------|-----------|
| 会话创建 / 列表 | Sessions 面板 + New / load |
| 恢复上下文 | 每条会话的 Resume（`session/resume`，不重放） |
| 历史重放 | 点击会话 → load（`session/load`） |
| 流式对话 / thought | 主日志区 |
| 权限审批 | Permission 卡片 |
| AskUser | Ask user 卡片（选项 + 自由文本） |
| Plan / Todo / 终端 | 右侧面板 |
| 模式切换 | default / plan 按钮 |

运维与恢复见 [GATEWAY_OPS.md](./GATEWAY_OPS.md)；协议设计见 [WEB_AGENT_GATEWAY.md](./WEB_AGENT_GATEWAY.md)。

## 仍保留的非 Web 入口

- `zene -p` / `--output-format json`：脚本与 CI
- `zene acp`：标准 ACP
- `zene sessions` / `config` / `export` / `mcp doctor`：运维
- `zene --repl`：轻量调试 REPL（无完整权限/Plan UI）
