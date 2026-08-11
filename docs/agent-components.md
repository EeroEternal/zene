# Zene 可组装 Agent 基础组件

目标：把 Zene 拆成**可独立发布、按需组合**的 crate，第三方 runtime 不必 fork `zene-core` 也能复用 compaction、tools、sandbox 等能力。

相关文档：[context-engine.md](./context-engine.md)、[agent-inference-context.md](./agent-inference-context.md)。

---

## 组件栈

```
┌─────────────────────────────────────────────────────────┐
│  Host（apps/cli ACP、Cloud worker、第三方 runtime）        │
└───────────────────────────┬─────────────────────────────┘
                            │ 组装
┌───────────────────────────▼─────────────────────────────┐
│  zene-core（可选）— turn loop、permission、hooks 编排      │
└───────────────────────────┬─────────────────────────────┘
                            │
     ┌──────────────────────┼──────────────────────┐
     ▼                      ▼                      ▼
zene-context          zene-tools + mcp       zene-session
estimate/compact      Tool trait/registry    持久化 transcript
assemble/epoch        内置 + MCP 工具         checkpoint/fork
     │                      │
     └──────────┬───────────┘
                ▼
         zene-llm ─── Message / ChatClient / ContextMetadata
                │
         zene-sandbox ─── Sandbox trait（本地 Keel / remote ACP）
                │
         zene-config ─── 配置 schema（逐步内聚到各组件）
```

**发布优先级**（硬依赖最少、复用价值最高）：

| Crate | 角色 | 对外依赖 |
|-------|------|----------|
| `zene-llm` | 协议与 Provider | config（后续可删） |
| `zene-sandbox` | 执行隔离 | 无 Zene crate |
| `zene-context` | 语义上下文引擎 | llm |
| `zene-session` | 会话持久化 | llm |
| `zene-tools` | 工具插件 | llm, sandbox, session |
| `zene-mcp` | MCP 适配 | tools |
| `zene-core` | Zene 产品 runtime | 以上全部 |

---

## 组合原则

1. **引擎不做 IO**：compaction、checkpoint、record、gateway publish 通过 `ContextEvent` 交给 runtime。
2. **trait 边界**：Session、Hooks、Sandbox、Tool 用 trait；Zene 类型提供 adapter，不强制 adopt。
3. **feature 可选**：`memory`、`gateway`、`prefire` 为 `zene-context` 的 cargo feature，轻量集成可关闭。
4. **core 是 composition root，不是唯一入口**：CLI / Cloud / 第三方只依赖需要的 crate。

---

## ContextEngine 边界（进行中）

### 已完成

- 从 `zene-core` 抽出 `zene-context` crate
- `ContextEngine` 统一 estimate → compact → assemble → epoch
- checkpoint 改为 `ContextEvent::Checkpoint`，由 runtime 落盘
- `ContextSession` trait + `SessionRecord` 内置 impl
- `ContextHooks` trait；todos/background 提醒移至 `zene-core::context_hooks`
- `zene-context` 移除对 `zene-tools`、`zene-config` 的依赖
- `CompactionConfig` 权威定义在 `zene-context`；`zene-config` 保留 serde 镜像（避免 config↔context 循环依赖）
- prefire 改用 `PrefireClientFactory`，不再依赖 `ZeneConfig`
- cargo features：`memory` / `gateway` / `prefire`（默认全开）
- `EstimateProvider` 替代对 `zene-config::ProviderKind` 的依赖
- **已发布 crates.io 0.1.11**：[`zene-config`](https://crates.io/crates/zene-config) → [`zene-llm`](https://crates.io/crates/zene-llm) → [`zene-session`](https://crates.io/crates/zene-session) → [`zene-context`](https://crates.io/crates/zene-context)

### 待做

| Phase | 内容 |
|-------|------|
| E | ~~`AgentBuilder`~~ ✅：core 拆 wiring，按需注入 sandbox/tools/context/MCP |

`AgentBuilder`（`zene-core`）示例：

```rust
let agent = AgentBuilder::new(config, sandbox, session, permission_mode)
    .without_mcp()
    .context_engine(ContextEngine::new(64_000))
    .build()
    .await?;
```

`Agent::new(...)` 等价于默认 builder。

### 发布（Phase D — 已完成）

```toml
# 第三方项目
zene-context = "0.1.11"
# 轻量
zene-context = { version = "0.1.11", default-features = false }
```

本地再次发包：`./scripts/publish-crates.sh --verify`（打包检查）或 `./scripts/publish-crates.sh`（按顺序上传，需新版本号）。

---

## 第三方 runtime 最小集成

```rust
// 伪代码 — 不依赖 zene-core
let mut engine = ContextEngine::new(128_000);
let mut messages: Vec<Message> = vec![/* ... */];

loop {
    let result = engine.prepare_step(&mut deps, &tools).await?;
    for event in &result.events {
        match event {
            ContextEvent::Checkpoint { reason } => save_my_checkpoint(reason),
            ContextEvent::EpochBumped { .. } => notify_gateway(event),
            _ => {}
        }
    }
    let response = client.chat(ChatRequest {
        messages: result.step.messages,
        context: Some(result.step.metadata),
        ..Default::default()
    }).await?;
    engine.record_step_usage(&response.usage, &mut session, &tools, &estimator, &cfg);
    messages.push(response.message);
}
```

---

## Core 耦合热点（后续拆分）

- `Agent` 上帝对象：一次构造拉起 client + tools + sandbox + context + permission + MCP
- `ToolContext` 硬编码 `LocalSandbox`
- `TodoItem` 在 session 与 tools 各一份
- Permission 双层：`ToolPermission` vs `PermissionGate`

拆分方向：`AgentBuilder` + trait object hooks（**已实现 builder**）；`ToolContext` 已改为 `Arc<dyn Sandbox>`；`TodoItem` / Permission 双层仍待拆。

### 2026-08-11 — Phase 1 Permission + tool output

- 新增 `zene-permission`：`ToolPermission` trait、`PermissionGate`、modes/rules 从 core 迁出
- 新增 `zene-tool-runtime`：`plan_tool_output_bound`（纯逻辑）+ `ToolOutputStore` / `FsToolOutputStore`（IO adapter）
- core `run_tools` 通过 `bound_tool_output` 组合 plan + spill；路线图见 [decoupling-plan.md](./decoupling-plan.md)

---

## 讨论记录

### 2026-08-10 — Phase C 配置内聚与 features

- `CompactionConfig` / `EstimateProvider` 内聚至 `zene-context`
- prefire 用 `PrefireClientFactory` 替代 `ZeneConfig`
- features：`memory`、`gateway`（reqwest）、`prefire`
- `zene-session` 路径 helper 独立，断开 session→config 依赖

### 2026-08-10 — Phase B ContextSession + ContextHooks

- `ContextSession` trait；`SessionRecord` adapter 在 `zene-context::session`
- `ContextHooks`；Zene todos/background 提醒在 `zene-core::context_hooks`
- `zene-context` 不再依赖 `zene-tools`

### 2026-08-10 — 可组装组件目标

- 确立组件栈与发布优先级
- Phase A：checkpoint 事件化，引擎不再直接 `save_checkpoint`
