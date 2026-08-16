# DeepSeek Harness Context 治理启示录与 Zene Context 优化演进

> 本文基于对 DeepSeek 开源 Agent Harness（`deepseek-ai/deepseek-harness`，简称 DSH）11 个 Agent Skills 的拆解，总结其在 Context 治理上的工程范式，并结合 Zene 的 [ContextEngine](./context-engine.md)、[Session 事实模型](./session-as-source-of-truth.md) 与 [Agent Runtime](./agent-runtime-optimization.md)，提出下一阶段的具体优化路径。

---

## 1. 核心认知：Context 优化的“下半场”

传统的 Context 优化通常只关注 **“输入端（Inbound Context）”**：
- Prompt 压缩与裁切
- KV Cache 命中（前缀三区稳定性）
- RAG / 代码检索拓扑（Select / RepoMap）

而 DeepSeek Harness 展现了至关重要的 **“输出端（Outbound Context Hygiene）”**：

```
+-------------------------------------------------------------------------+
| 当前 Task 会话 (Transient CoT / Tool Logs / PR Arguments / Reviews)     |
+-------------------------------------------------------------------------+
                                     │
                  [ 会话视角猎杀器 / Trim CoT Leakage ]
                  - 移除中间决策编号 (decision 7) / 阶段标签 (T4/W3)
                  - 剥离会话代词与转录辩解 ("正如我们所见" / "在这个PR中")
                  - 将陈述重写为 HEAD 上的独立事实
                                     │
                                     ▼
+-------------------------------------------------------------------------+
| 仓库 HEAD 事实库 & 决策笔记 (Clean Repo State & Lifecycle-managed Notes)|
+-------------------------------------------------------------------------+
                                     │
           [ 高信噪比注入 ]            │ [ 投影与前缀三区优化 ]
                                     ▼
+-------------------------------------------------------------------------+
| 未来 Agent 会话 / Subagents (高自解释性、零未决引用、无多余幻觉)          |
+-------------------------------------------------------------------------+
```

> **核心法则**：**Agent 产出的文档、代码注释、决策记录与工具输出，在未来会成为自身或其他 Agent 的 Context。**
> 如果产出带有“单次会话视角（Session Viewpoint）”或“未定义引用（Dangling References）”，仓库 Context 就会被不可逆地污染。

---

## 2. DeepSeek Harness 的五大 Context 治理机制拆解

### 2.1 猎杀“思维链与会话视角泄漏”（`trim-cot-leakage`）
* **病灶（Epistemic Entanglement / 视角污染）**：Agent 把写作会话内的临时脚手架（审阅记录、阶段编号、PR 差异叙述、对 reviewer 的辩解）写进了持久化 prose/代码注释中。
* **硬判据**：**一个只读当前分支 HEAD、没有会话转录与 PR 线程的读者（人或新 Agent），能否独立解析每个引用并验证每个论断？**
* **重构原则**：
  * **非单纯删除**：提取事实从句，重述为当前 HEAD 成立的声明式状态，然后剥离转录壳。
  * **无事实整句清除**：控制流叙述、审计码、阶段标签直接整句删除。

### 2.2 决策记录的全生命周期管理（`archive-agent-notes`）
* **病灶（Context Bloat & Stale Rules）**：架构决策（ADRs/Notes）越积越多，Agent 被海量历史规则淹没或陷入陈旧规则的死锁。
* **分层治理**：
  * `implemented`（已实施）：仅保留跨时间指导未来改动（所有权边界、协议语义、安全规则、重引入条件）的内容；一次性 UI/小 bug 历史移入归档区。
  * `proposed`（提案）：不归档；不可行直接标记 `rejected`。
  * `rejected`（否决）：**仅当失败方案仍是“诱人且高频的错误”时保留作为负向护栏（Negative Guardrails）**；其余彻底删除。
  * **明确非绝对真理（Not Golden Truth）**：历史 Notes 和 Tests 都不是不可动摇的承重墙，允许新设计讨论。

### 2.3 主动减熵与消除投机冗余（`find-simplifications`）
* **病灶**：Agent 极其容易产生投机通用性（Speculative Generality）——多套未使用的事件模型、瞬时状态与持久状态的重复定义。这些冗余会被代码检索（Grep / Symbol Graph）作为噪声喂回给模型。
* **治理**：要求 Agent 主动寻找无生产消费者的导出符号/事件，且简化必须有“扎实证据”，写进 Note 跟踪。

### 2.4 窄执行域与工具输出约束（`pre-push-checks` & `code-review`）
* **窄域测试**：本地只执行被精准计算影响（Change-scope）的最小失败用例，全量交给 CI，避免大量成功日志淹没 Agent 窗口。
* **工具优先**：先消耗结构化元数据（静态检查报告/Dirty layers），再做语义审查。

---

## 3. 对 Zene Context 优化的借鉴与落地方案

对照 Zene 目前的架构体系（[context-engine.md](./context-engine.md)、[session-as-source-of-truth.md](./session-as-source-of-truth.md)、`zene-context`、ACP 与 Skills 体系），我们可以从以下四个层面进行增强：

### 3.1 增强生成门禁：引入“会话视角残留”检查（Session Leakage Guard）

在 Agent 编写代码注释、生成 PR 描述、更新 Docs 或写 Commit Message 时，增加后处理或 Review Prompt 规则：

```markdown
<!-- 规则参考：Zene Agent Output Sanitization -->
- 禁止会话代词与时序指涉：拦截 "如上文所述"、"正如上一轮讨论"、"在这个 PR 中"。
- 禁止对比废弃态：不写 "不再使用旧的 X"（直接声明当前组件的职责）。
- 辩护代码内化：如果安全断言成立，应当体现在类型系统/断言中，而不是在注释里写 "这个转换是安全的，审阅者请放心"。
```

### 3.2 优化 Zene ContextEngine 的 Tool Output 过滤（Inbound Hygiene）

当前 `ContextEngine` 负责 `estimate → compact → memory → assemble`。在输入到模型前，对工具执行结果做更严格的投影过滤：

1. **Test / Cargo 输出压缩**：
   - 跑 `cargo test` 或 CI 时，过滤掉所有无害的 `test ... ok` 输出，只保留 `FAILED` 栈帧与 Panic 摘要。
2. **Diff / Git 范围计算**：
   - 引入轻量级 `change-scope` 算法，在 Subagent 派发或 Review 任务中，只投影最窄相关的 AST 变更与文件列表，防止 Agent 盲目全量加载。

### 3.3 规范 Zene 架构决策与 Memory 生命周期（Agent Notes / Memories）

在 `zene-context` 的 `memory` 模块与项目根目录文档治理中建立清晰的生命周期：

| 状态 | 存储位置 | Context 注入策略 |
| :--- | :--- | :--- |
| **Active Invariants (现行公理)** | `docs/` 或 System Memory | **常驻 / 按需注入**：保留接口契约、安全边界、持久化协议。 |
| **Negative Guardrails (防坑墓碑)** | `.zene/rejected/` 或 Scratch | **微量/触发式注入**：只保留那些“极其诱人但已经被证实有深坑”的 rejected 设计。 |
| **Archived Context (归档历史)** | Git 历史 / Archived Notes | **完全移出活跃 Context**：避免模型受到过时决策干扰。 |

### 3.4 保持 Session 与 Context 投影的严格正交（Zene 既有优势深化）

Zene 已经在 [session-as-source-of-truth.md](./session-as-source-of-truth.md) 中确立了：
- **Session（L0/L1）** 是不可变事件事实源。
- **Context（L2/L3）** 是单次推演的投影。

结合 DSH 的经验，我们需要进一步确保：
- **Subagent 间通信**：父子 Agent 通信消息应经过“语义蒸馏”，不应把子 Agent 的全量 CoT 乱序塞回父 Agent 的主 Context。
- **Replay / Compaction**：在 Compact 时，不仅做单纯的摘要截断，更要将已解决的中间纠结态（trial-and-error）清洗为最终的事实状态，提升 Prompt Cache 效率与可读性。

---

## 4. 总结行动项 (Next Actions)

- [ ] **文档化规范**：在 `docs/agents/` 或 skills 中沉淀 `trim-cot-leakage` 与 `doc-standards` 规则。
- [ ] **Prompt / Review Skill 注入**：在 Code Review 和 PR 生成的 prompt 中加入“无会话视角、HEAD 可自解释”黄金准则。
- [ ] **Tool Output 窄化**：在 `zene-tools`（如 Bash / Test 运行器）中增加输出清洗过滤器，降低 Token 浪费与注意力稀释。
