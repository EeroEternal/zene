# Zene Gateway 运维指南

本地 HTTP Gateway（`zene-gateway` / `zene web`）把浏览器 Web Agent UI 接到 `zene acp`。本文覆盖配置、安全与升级。

## 启动

推荐入口：

```bash
# 与 zene 同目录安装时
zene web --yolo --sandbox-off

# 或直接
zene-gateway --zene-bin "$(which zene)" --yolo --sandbox-off
```

`zene web` 会自动把当前 `zene` 可执行文件传给 gateway（可用 `ZENE_GATEWAY_BIN` 覆盖 gateway 路径）。默认监听 `127.0.0.1:8787`，启动时打印带 token 的 URL。

常用参数：

| 参数 | 含义 |
|------|------|
| `--bind` / `--port` | 监听地址；非 loopback 必须加 `--allow-remote` |
| `--token` | 共享访问令牌；省略则每次生成 |
| `--data-dir` | journal/meta 目录；默认 `$ZENE_GATEWAY_DATA` 或 `~/.zene/gateway` |
| `--no-persist` | 关闭落盘（进程退出后不可 attach） |
| `--yolo` / `--sandbox-off` | 传给 `zene acp` 子进程 |
| `--acp-env KEY=VALUE` | 子进程环境变量（如模型 API） |
| `--acp-command` / `--acp-arg` | 覆盖 ACP 命令（测试用） |

## 持久化与恢复

默认启用 journal 持久化：

```text
~/.zene/gateway/agents/<agentId>/
  meta.json       # workspace、时间戳
  journal.jsonl   # 事件游标日志（含系统事件与 ACP 帧）
```

恢复策略：

- **Agent 子进程崩溃**：journal 保留；`POST /api/v1/agents/{id}/restart` 重新拉起 ACP，不静默重放未确认 prompt。
- **Gateway 进程重启**：内存中的 live agent 丢失；`GET /api/v1/agents` 的 `persisted` 列出可恢复项，再 `POST .../attach` 重挂 journal 并启动新 ACP 子进程。
- **页面刷新**：用 cursor 长轮询/SSE 续读即可，无需 attach。

## 背压与限制

| 限制 | 默认 |
|------|------|
| POST body | 1 MiB |
| 单条 ACP message | 1 MiB |
| 单次 POST messages 条数 | 100 |
| journal 事件数 / 字节 | 约 1 万条 / 32 MiB（超限淘汰旧事件） |
| 单条 journal payload | 256 KiB（超限记 `payload_truncated`） |
| 每 agent 并发 poll/SSE | 2（超额 429 `too_many_polls`） |
| 长轮询 waitMs | 默认 25s，上限 30s |

超大 tool/terminal 输出会先在 journal 层截断，避免拖垮 Gateway 内存。

## 安全

- 默认只绑 loopback；远程绑定需 `--allow-remote`，生产环境应另加 TLS、反向代理与更强认证。
- 写请求需要 `X-Zene-Token`（或 `Authorization: Bearer` / query `token`）。
- 带 `Origin` 的浏览器请求必须来自本机允许源。
- 多标签写保护：controller lease；非持有者写操作返回冲突。
- 不要把 API key 放进浏览器；通过 `--acp-env` 或本机环境注入给 `zene acp`。

## 升级

1. 升级 `zene` 与 `zene-gateway`（建议同版本）。
2. `/api/v1` 内只做向后兼容字段增加；破坏性 envelope 变化会走 `/api/v2`。
3. journal 文件为 JSONL；升级后旧 journal 仍可 attach，但 cursor 过期时客户端应从 `oldestCursor` 重新同步。
4. 升级窗口内可先 `--no-persist` 做一次性验证，再切回默认落盘。

## 健康检查

- `GET /api/v1/health`：进程存活
- `GET /api/v1/bootstrap`：传输能力、特性开关、限制
- `GET /api/v1/agents/{id}/health`：子进程状态、journal 游标、是否可 restart

设计细节见 [WEB_AGENT_GATEWAY.md](./WEB_AGENT_GATEWAY.md)。
