# Cloud MVP 状态

本地可完整演示的多用户 Cloud Agent 链路：

1. 注册 / 登录
2. Connect GitHub（mock 或 live OAuth/App）
3. 同步仓库并创建 Run
4. Worker clone（mock workspace 或真实 git）
5. MockAgent 或真实 `zene acp`（需 LLM）
6. 事件流、权限审批
7. Files / Diff
8. Push + Draft PR（Git Broker）

## 验证过的命令路径

```bash
cd cloud
./scripts/dev.sh
# http://127.0.0.1:8788/
```

自动化冒烟：

- `cargo test --workspace`
- 注册 → mock connect → create run → completed → files + pull-requests 非空

## Live 模式前置

- `ZENE_CLOUD_GITHUB_MODE=live`
- GitHub OAuth app + GitHub App 私钥
- `ZENE_BIN` + `ZENE_API_KEY`/`ZENE_BASE_URL` 以启用真实 ACP
