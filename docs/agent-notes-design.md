# Zene Agent Notes 架构设计与存储规范

> **定位**：Agent Notes 是 Zene 中用于跨会话、跨任务沉淀架构决策、模块边界与“负向防坑护栏”的高信噪比持久知识体系。
> 本文针对 Zene Cloud 多租户/多会话运行态与本地 CLI，定义 Notes 的分层存储位置、生命周期状态机、Context 注入与提升规则。

相关文档：
- [context-engine.md](./context-engine.md) — 语义上下文引擎与前缀布局
- [session-as-source-of-truth.md](./session-as-source-of-truth.md) — Session 事实源模型
- [context-optimization-plan.md](./context-optimization-plan.md) — Context 治理与优化演进计划
- [deepseek-harness-context-lessons.md](./deepseek-harness-context-lessons.md) — DeepSeek Harness 经验启示

---

## 1. 核心设计原则

1. **写给未来的 Agent 和人类看**：Notes 是自包含的架构契约与避坑指南，严禁携带单次对话的临时脚手架（杜绝“思维链与会话视角泄漏”）。
2. **三层作用域隔离（Layered Isolation）**：区分“全团队共享的代码库资产”、“工作区级跨 Run 缓存”与“单会话瞬时草稿”，保证 Cloud 运行态不污染 Git 树。
3. **严格的生命周期状态机（Lifecycle Managed）**：活跃真相、防坑墓碑与历史流水账物理隔离，防止上下文膨胀。
4. **决策不是绝对金科玉律（Not Golden Truth）**：Notes 记录的是特定上下文下的理据与权衡，业务演进时允许讨论和推翻。

---

## 2. 三层存储架构 (Cloud & Local)

在 Zene Cloud 环境下（多组织、多工作区 `{workspace_root}/ws/{workspace_id}`、动态分支 `session-xxx`）以及本地 CLI 中，Notes 严格划分为三层：

```
+-------------------------------------------------------------------------------+
| Layer 1: Repo-Level Durable Notes (代码库级 · 团队共享 · 进 Git)                |
| 路径: <repo_root>/.zene/notes/ (或 docs/notes/)                                |
| 包含: active/ (现行承重墙), negative-guardrails/ (防坑墓碑), archived/ (归档)  |
+-------------------------------------------------------▲-----------------------+
                                                        │ [Promote via PR]
+-------------------------------------------------------┴-----------------------+
| Layer 2: Workspace-Level Persistent Cache (工作区级 · 跨 Run 记忆 · 不进 Git)   |
| 路径: <workspace_root>/ws/<workspace_id>/.zene/state/notes.jsonl (gitignored) |
| 包含: 单租户跨会话试错记忆、待评审提案 (proposed)、本地构建环境约束              |
+-------------------------------------------------------▲-----------------------+
|                                                       │ [Extract on Task Done]
+-------------------------------------------------------┴-----------------------+
| Layer 3: Session-Level Scratchpad (单次会话级 · 瞬态运行内存)                  |
| 路径: .zene/scratch/ 或 zene-session Events (Checkpoint / Todo / Memory)     |
| 包含: 当前 Task 执行计划、中间工具输出解析、临时调试草稿                       |
+-------------------------------------------------------------------------------+
```

### 2.1 详细目录规范

```text
<repo_root>/
└── .zene/notes/                  # (或 docs/notes/) [Layer 1]
    ├── active/                   # 现行架构公理 (Active Invariants)
    │   ├── 001-context-engine-prefix.md
    │   └── 002-dual-backend-rationale.md
    ├── negative-guardrails/      # 高频诱人错误 / 防坑墓碑 (Negative Guardrails)
    │   └── reject-sync-acp-blocking.md
    └── archived/                 # 历史决策与实施记录 (Archived History)
        └── 2026-08-10-initial-tui.md

<workspace_root>/ws/<workspace_id>/
└── .zene/state/                  # [Layer 2: Cloud 持久但受 gitignore 保护]
    ├── notes.jsonl               # 跨 Run 共享的经验与草稿
    └── workspace-invariants.json # 本机特定依赖缓存 (如 cargo target 路径约束)
```

---

## 3. Note 结构与生命周期状态机

每条 Note 采用带有 YAML 元数据的 Markdown 格式，具备明确的状态机流转：

```
                    ┌─────────────┐
                    │  proposed   │ (提案/讨论中)
                    └──────┬──────┘
                           │
             ┌─────────────┴─────────────┐
             ▼                           ▼
      ┌─────────────┐             ┌─────────────┐
      │ implemented │ (已落地)    │  rejected   │ (已否决)
      └──────┬──────┘             └──────┬──────┘
             │                           │
   [语义评估：是否具有跨时间价值？]   [语义评估：是否是高频诱人错误？]
      ├── 具有持久架构意义 ──► active/    ├── 是高频大坑 ──► negative-guardrails/
      └── 一次性实现细节 ───► archived/  └── 显而易见/过时 ──► 物理删除 (Purged)
```

### 3.1 Note 标准模板

```markdown
---
id: note-20260816-context-prefix
title: Prefix-adjacent 注入顺序约束
status: active # active | proposed | negative-guardrail | archived
author: agent-run-8492
created_at: 2026-08-16
tags: [context-engine, inference, kv-cache]
---

### 1. 核心约束 (Invariant)
在 `ContextEngine::assemble` 阶段，System Prompt 的静态冻结区必须永远保持在 Token 序列头部，动态注入内容必须后置。

### 2. 理据与上下文 (Rationale)
LLM Provider（如 Anthropic / OpenAI）的前缀缓存严格依赖起始字节的一致性。将动态时间戳或动态 Todo 放在头部会导致全量 KV Cache 失效。

### 3. 负向保证 (Negative Guarantees / 防踩坑)
- 严禁在 System Prefix 头部添加带随机 ID 或即时时间戳的探针。
- 即使为了调试便利，也不得在此区域插入临时 Log 标记。
```

---

## 4. ContextEngine 与 Notes 的动态组装策略

为了避免 Notes 膨胀占用宝贵的模型 Context 窗口，`zene-context` 在组装上下文（`assemble`）时采取**分级、分流、按需挂载**机制：

| Note 类型 | 注入时机 | 注入方式 | 预算控制 |
| :--- | :--- | :--- | :--- |
| **Active Invariants** (`active/`) | 每次 Run 初始化时 | 挂载在 System Prompt 的静态规则区 | 严格限制条数（≤ 5 条），每条摘要 ≤ 200 Tokens |
| **Negative Guardrails** (`negative-guardrails/`) | **条件触发**（当 Agent 计划修改关联模块或匹配到特定关键词时） | 注入在 Agent Step 的 Tail Decoration 或 Task Prompt | 按需精准注入 1~2 条，平时不常驻 |
| **Workspace Notes** (Layer 2) | 本 Workspace 启动新 Run 时 | 作为 Local Workspace Context 动态追加 | 仅提取活跃状态条目 |
| **Archived Notes** (`archived/`) | 仅当 Agent 主动调用搜索工具时 | 不主动注入，仅作为 RAG / Search 资源 | 0 常驻 Token 消耗 |

---

## 5. Agent 协作与提升工作流 (Promotion Workflow)

### 5.1 运行中产生（Writing in Task）
1. Agent 在执行 Cloud 任务时，若发现重要架构边界或重大试错教训，先写在 Layer 3（`.zene/scratch/`）。
2. 若该经验对本 Workspace 的后续 Run 有价值，写入 Layer 2（`notes.jsonl`）。

### 5.2 任务结束与 PR 生成（PR Gate & Promotion）
当 Agent 准备提交 PR 时：
1. **自动审查**：触发 `trim-cot-leakage` 门禁，将 Note 中的会话代词（如“根据本次任务”、“正如刚才报错所见”）清洗为声明式的 HEAD 事实。
2. **提升至 Layer 1**：将具有全项目价值的 Note 放置在 `.zene/notes/active/` 或 `.zene/notes/negative-guardrails/`，随同代码变更一起提交到 PR。
3. **人类 Review**：人类在审查代码时同步审查 Note，合并后正式成为全团队共享资产。

---

## 6. 后续演进计划 (Roadmap)

- [ ] **CLI / Core 集成**：在 `zene-context` 中增加对 `.zene/notes/` 目录的解析与按需装载能力。
- [ ] **Skill 支持**：提供 `zene-note-create` 与 `zene-note-archive` skills，指导 Agent 自动管理笔记生命周期。
- [ ] **Console UI 呈现**：在 Cloud Web Console 的 Workspace 设置页展示当前 Workspace 的 Active Notes 与 Negative Guardrails。
