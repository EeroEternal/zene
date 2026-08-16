# Zene Context 治理与优化计划 (借鉴 DeepSeek Harness Skills)

> **目标**：以 DeepSeek Harness (DSH) 的 11 个 Agent Skills 为蓝本，将「Context 优化」从单一的输入端压缩扩展为涵盖 **输入剪裁、输出保洁、决策生命周期、减熵巡检与工具收敛** 的完整体系，提升 Zene 模型推理效率、消除 AI 味与视角污染、降低幻觉与上下文成本。

---

## 1. DSH 11 个 Skills 的 Context 映射矩阵

DSH 的 11 个 Skills 本质上是一套完整的 Context 治理闭环：

| 领域分类 | DSH Skill 名称 | 治理的 Context 隐患 | 对 Zene 的转化与借鉴 |
| :--- | :--- | :--- | :--- |
| **文档与文本 (Prose/Doc)** | `dsh-trim-cot-leakage` | **会话视角泄漏**：残留中间 decision 编号、PR 审阅辩解、死掉的阶段标签等 | **PR / Commit / 注释门禁**：强制站在 HEAD 视角重构事实句 |
| | `dsh-prose-standard` | **低信噪比文本**：冗长、空话、AI 腔调 | **Agent 生成文本纪律**：精简、无修饰词、契约明确 |
| | `dsh-doc-standards` | **结构失范**：教程与参考混杂导致 RAG 索引污染 | **Docs 结构规范化**：分离 Invariants 与 Guides |
| | `dsh-doc-site-sync` | **文档与实现漂移**：代码与文档脱节产生错误 Context | **文档与代码联动门禁** |
| | `dsh-translate-docs` | **多语言语义不对齐** | **双语文档同步保证** |
| **工程纪律 (Engineering)** | `dsh-find-simplifications` | **投机通用性 & 冗余 AST**：未使用接口/事件塞满代码库 Context | **减熵/瘦身巡检 Skill**：主动发现未消费的符号与重复状态 |
| | `dsh-code-review` | **盲目全量阅读**：缺乏结构化指引直接把大文件塞入上下文 | **先元数据/脏层后语义**的 Review 流水线 |
| | `dsh-pre-push-checks` | **庞大测试日志炸毁窗口**：几百行 ok 测试塞满 Tool 结果 | **窄域测试 (Change-scope)**：本地只跑受影响失败用例 |
| **流程协作 (Workflow)** | `dsh-archive-agent-notes` | **决策库膨胀 & 过期规则死锁** | **Agent Notes 状态机**：implemented 语义归档，rejected 提炼负向护栏 |
| | `dsh-merging-stacked-prs` | **多层 PR 上下文缠绕** | **堆叠改动的上下文解耦** |
| **多媒体 (Media)** | `dsh-record-browser-gif` | **大体积/长文本的 DOM 描述** | **多模态直观验证替代冗长文本输出** |

---

## 2. 优化实施计划：四阶段演进路线

```
Phase 1: 输出保洁与会话视角猎杀 (Immediate)
├── 规则与 Skill 落地: zene-trim-cot-leakage & zene-prose-standard
└── 集成至 PR Description / Commit / Docs 生成流程

Phase 2: 决策记录与 Memory 生命周期 (Short-term)
├── Agent Notes / Architecture Decisions 结构化标准
└── 状态机治理: Implemented 语义归档 / Rejected 负向护栏

Phase 3: 输入端与工具执行日志收敛 (Mid-term)
├── 窄域测试 (Change-scope) 机制
└── Tool Output 降噪与结构化过滤 (Cargo / Test / Linter)

Phase 4: 代码库减熵与巡检常态化 (Long-term)
├── zene-find-simplifications (主动发现冗余 AST / 重复状态)
└── ContextEngine 前缀三区与 Compaction 蒸馏深度协同
```

---

### Phase 1: 输出保洁与会话视角猎杀 (立即启动)

**核心任务**：防止 Agent 在单次会话中的临时脚手架污染持久化代码与文档。

1. **制定 `zene-trim-cot-leakage` 规则库**：
   - **拦截死引用**：检查并剔除 `(decision 3)`、`Phase 2`、`W3/T1`、`按照讨论 4` 等无全局定义的会话内部引用。
   - **拦截会话叙述代词**：剔除 `如上所述`、`正如我们发现的`、`本 PR 修复了上一轮的问题`、`不再使用旧的 X`。
   - **转换辩解句**：将给 Reviewer 解释的注释（如 `// safe cast because reviewer asked`）重构为自包含的代码断言或类型约束。
2. **黄金检查准则（Hard Assertion）**：
   > *“一个只检出当前分支 HEAD、脱离了本次对话和 PR 历史的人类/Agent，能否独立理解并验证每一个术语和论断？”*
3. **交付物**：
   - 编写 `.gemini/skills/trim-cot-leakage/SKILL.md`（或 Zene 内置 skills）。
   - 在 PR Description / Commit 自动化流程中加入 Leakage Check 校验步。

---

### Phase 2: 决策记录与 Memory 生命周期治理 (短期 · ✅ 已落地)

**核心任务**：防止 ADR / Notes 无限膨胀拖慢 Context，保证注入给模型的知识高信噪比。

1. **规范 Agent Notes 格式与生命周期**：
   - **`implemented` 归档规则**：
     - 保留：协议语义、跨时间的所有权边界、安全规则、重引入条件。
     - 归档至 `archived/`：一次性 UI 细节、窄适配器改动、修完即闭合的小 bug 流程历史。
   - **`rejected` 提炼为负向护栏（Negative Guardrails）**：
     - 仅保留“方案看起来很诱人、容易让后续 Agent/新人反复重犯，但被证实有大坑”的记录，写明失败原因。
     - 其余过时/无重提可能的 rejected 记录直接清除，避免占用上下文。
2. **解除教条死锁（Not Golden Truth 原则）**：
   - 在 Prompt 中明确：历史 Notes 和 Tests 是上下文背景与历史理据，**不是不可推翻的承重墙**。鼓励在新场景下发起演进讨论。
3. **交付物**：
   - ✅ [docs/agent-notes-design.md](./agent-notes-design.md)（三层存储隔离规范）
   - ✅ `.agents/skills/archive-agent-notes/SKILL.md`（生命周期管理 Skill）
   - ✅ `crates/context/src/memory_store.rs`（原生支持从 `.zene/notes/active/` 与 `docs/notes/active/` 自动注入现行公理至 System Prefix）

---

### Phase 3: 输入端与工具输出收敛 (中期 · ✅ 已落地)

**核心任务**：避免巨大的终端日志和无差别全量阅读炸毁 Agent 上下文窗口。

1. **窄域执行 (Change-Scope)**：
   - 引入精确的 diff/依赖分析，本地验证只跑直接受影响的最小测试子集。
   - 全覆盖和矩阵验证交给 CI，不把海量测试日志塞进本地 Agent 对话。
2. **工具输出清洗器（Tool Output Sanitizer · ✅ 已实现）**：
   - 在 `crates/tools/src/output_sanitizer.rs` 中实现 `OutputSanitizer`，并在 `BashTool` 中自动接入：
     - **测试成功折叠**：过滤掉成百上千行无害的 `test ... ok` 输出，仅保留结果 Summary。
     - **测试失败精准提取**：仅定位并返回 `FAILED` 用例的错误帧与 Panic 堆栈。
     - **超长输出智能省略**：单次命令输出超过阈值（如 300 行）时自动对中间内容进行截断提示。
3. **交付物**：
   - ✅ `crates/tools/src/output_sanitizer.rs`
   - ✅ `crates/tools/src/bash.rs` 联动过滤

---

### Phase 4: 代码库主动减熵与巡检 (长期 · 持续演进)

**核心任务**：消除代码库中的投机冗余，提升代码检索（Select / RepoMap）给模型喂回上下文时的纯度。

1. **落地 `zene-find-simplifications` 技能**：
   - 扫描**“镜像同一事实的多个表示”**（尤其是持久 session 事件 vs 瞬时 runtime 事件的重复抽象）。
   - 扫描**“无生产消费者的公开方法/配置旋钮”**以及**“仅为测试而存在的非承重代码”**。
   - 强制要求“简化需要扎实证据”，每次提议生成简化候选 Note，杜绝盲目大删大改。
2. **ContextEngine Compaction 深度协同**：
   - 在 `ContextEngine::handle_overflow` 和 `compact_forced` 时，将历史会话中的 Trial-and-error 纠结过程直接“语义编译”为干净的声明式状态，提升 KV cache 稳定性与上下文密度。
3. **交付物**：
   - ✅ `.agents/skills/find-simplifications/SKILL.md`（减熵巡检 Skill）
   - ✅ [docs/context-engine.md](./context-engine.md)（三区布局与 Compaction 去抖）

---

## 3. 衡量指标 (Success Metrics)

1. **Context 净荷信噪比**：
   - Agent 单轮交互消耗 Token 下降（消除测试 ok 日志与全量文件加载）。
   - 生成的注释与 PR 中“会话视角残留”率降至 0。
2. **KV / Prompt Cache 命中率**：
   - 规范化前缀与清洗后的 Notes 保持高稳定性，提升推理速度与降低费用。
3. **代码库健康度**：
   - 定期巡检消除死代码与冗余状态，保持符号表与 RepoMap 极简。
