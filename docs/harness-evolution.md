# Zene Harness 进化设计(路线 B:机制 native 化)

目标:让 Zene 的 harness(rules、skills、prompts、config)**可被受控变更、同题对比评估、择优采纳**,不依赖 Reef 的 Python 服务栈。机制思想来自 [reef-harness-evolution.md](./reef-harness-evolution.md),本页是 Zene 侧的落地设计。

相关文档:[architecture.md](./architecture.md)、[agent-components.md](./agent-components.md)、[session-as-source-of-truth.md](./session-as-source-of-truth.md)、[context-engine.md](./context-engine.md)。

**不在本文范围**:自动 propose 的模型调用(后续可选层)、权重训练、Reef 代码移植、跨机器分布式评估、把 permission / 端点配置纳入可进化对象。

---

## 1. 一句话

> **harness = 一棵可序列化的树;进化 = mutation + 同题配对 episode + 带签名的选择决策;赢了写基线,输了丢弃。**

四个最小对象:`HarnessTree`、`Mutation`、`evaluate/decide` 契约、配对 episode runner。全部放进现有 crate 边界,不新增服务。

## 2. 现状盘点(地基已在)

以下每条都对照过当前代码:

| 能力 | 现状 | 依据 |
|---|---|---|
| 组合根可重定位 | `ZENE_HOME` 重定义全局 home | `crates/config/src/lib.rs:965` |
| 配置双层合并 | 全局 `~/.zene/config.toml` + 项目 `.zene/config.toml`,`merge_config_toml` 深合并 | `crates/config/src/lib.rs:126,424,972-980` |
| 组合面已有 trait | `WorkspaceProvider`(`agent_instructions` / `workspace_overview` / `discover_skills`) | `crates/workspace/src/provider.rs:4-8` |
| rules 有文件约定 | `AGENTS.md` / `CLAUDE.md`,按序拼接进系统 prompt | `crates/workspace/src/provider.rs:11`、`prompt.rs` `build_system_prompt_ext` |
| skills 有文件约定 | `.agents/skills/<name>/SKILL.md`,发现 + 列入 prompt | `crates/tools/src/skill.rs:55-60`、`crates/workspace/src/skills.rs` |
| prompt 可组装 | `build_system_prompt_ext(base, provider, ...)` | `crates/workspace/src/prompt.rs:29` |
| 事实源可回放 | `RecordEntry` 枚举 + `AgentRecordWriter` append-only 记录 | `crates/session/src/record.rs:30,286-343` |
| 无头运行 | `zene acp` stdio JSON-RPC,yolo 模式自动批准 | `apps/cli/src/main.rs:77-79,188` |

**剩余缺口**:多轮自动循环与周期复查(§3.5 可选层)。P0(树 + 渲染 + mutation 准入)、P1(评估契约 + 配对 runner)、P2(失败驱动 propose)均已落地:树为 `HarnessTree`(4 kind),渲染为 harness 目录文件,提案经严格 JSON 解析 + 准入(`zene-eval::tree`/`propose`),闭环命令 `zene eval evolve`。

## 3. 核心对象

```text
HarnessTree ──render──► episode workdir ──zene acp──► RecordEntry 轨迹
     ▲                                                    │
     │                EpisodeScorer::score ◄──────────────┘
     │                            │
  Mutation                 SelectionDecision
 (create/update/remove)    (select / reject + policy 签名)
     │                            │
     └───── select: 写基线 ────────┘   reject: 丢弃,树不动
```

### 3.1 HarnessTree

扁平 entry 列表,每个 entry:`id`(树内唯一)、`kind`、`config`(该 kind 的字段)。四种 kind,与 Zene 现有文件约定一一对应:

| kind | 渲染目标 | 落盘后由谁消费 |
|---|---|---|
| `config` | `.zene/config.toml` 的深合并段 | `merge_config_toml`(零改动) |
| `rules` | `AGENTS.md` 段(树序拼接) | `FsWorkspaceProvider::agent_instructions`(零改动) |
| `skill` | `.agents/skills/<name>/SKILL.md` | `tools::skill` + `discover_skills`(零改动) |
| `prompt` | 系统 prompt 增补段 | `build_system_prompt_ext`(零改动) |

> **关键懒点**:Zene 的加载侧本来就是"文件系统约定",渲染器只需按约定落盘,**加载路径一行不改**。这正是 Reef adapter 模式的本质 —— kind 中立的树 + 落盘映射,只是这里映射给自己。

`prompt` 与 `rules` 的区别:rules 进项目指令段(`# Project instructions`),prompt 是独立增补段;`build_system_prompt_ext` 的拼接顺序即优先级。

### 3.2 Mutation 与准入

```rust
enum MutationOp { Create, Update, Remove }
struct Mutation { id: String, op: MutationOp, config: Option<...> }
```

准入规则(与 Reef 的 mutation 契约同构,应用于 tree 和 apply 两处):

- `create` 拒绝已存在 id;`update` 拒绝缺失 id 或变更 kind;`remove` 拒绝缺失 id
- 每个 kind 自带校验(config 段必须能进 `merge_config_toml` 的 schema;skill 必须有 frontmatter 的 `SKILL.md`;`prompt`/`rules` 非空)
- **权限与端点字段不进树**:`permission_rules`、`ZENE_API_KEY`/`ZENE_BASE_URL`/provider 选择(`crates/config/src/lib.rs:579-672` 的 env 面)不作为可变异对象 —— 见 §5

### 3.3 评估契约(唯一的新层)

```rust
/// 方法侧实现(与 zene-eval 实现一致):一次 episode 证据 → 分数
trait EpisodeScorer {
    fn score(&self, task_id: &str, run: &EpisodeRun) -> Result<f64>;
}
// EpisodeRun { final_text: String, trajectory: Vec<RecordEntry> }
// 评分对象是"最终回答 + 执行轨迹";轨迹本身不含最终文本,故单独携带

/// 机制侧实现:候选 vs 在役,持久可审计
struct SelectionDecision {
    outcome: Select | Reject,
    policy: String,        // "win_margin" | "floor"
    policy_version: u32,
    reason: String,
    candidate_scores: Vec<f64>,
    incumbent_scores: Vec<f64>,
}
```

两种内置策略(照 Reef 的 ScoreComparison / Floor,够用即止):

- **win_margin**:同题配对,候选赢的任务数 − 输的任务数 > margin 才 select
- **floor**:候选单独跑,所有任务分数 ≥ floor 才 select

决策记录追加进 session 的 `RecordEntry` 流(或独立 manifest),**与 session-as-source-of-truth 同构:决策是事实,不是日志**。

> 已落地:`crates/eval`(`zene-eval`)提供 `EpisodeScorer`、`EpisodeRun`、`SelectionDecision`、`decide_win_margin`/`decide_floor` 和 JSONL 的 `append_decision_record`,含单元测试。`DecisionRecord`(task_ids、candidate/incumbent tree、baseline_commit、决策)即 §3.5 「manifest 每步一行」的落地形态。内置 `ExactAnswerScorer`(精确最终答案比对,fixture: task_id → 期望答案)。

> 配对 runner(§3.4)已落地:`zene-eval::runner::run_paired_episodes` + `EpisodeExecutor` 接缝 + CLI `zene eval run --tasks ... --incumbent ... --candidate ...`。

> P0/P2 已落地:`HarnessTree`(rules/skill/config/prompt 四 kind,渲染到现有文件约定)+ `apply_mutations` 准入(§3.2 契约逐条实现,含 `DENIED_CONFIG_KEYS` 端点/凭证/权限禁改清单 + skill 名防目录穿越 + frontmatter 校验)+ `Proposer` 接缝与严格 JSON 提案解析(`zene-eval::propose`)。闭环命令 `zene eval evolve --tasks ... --tree tree.json`:失败发现 → LLM 提案 → 准入 → 配对评估 → `DecisionRecord` + `tree.candidate.json`(人工 promote,不自动改基线)。

### 3.4 配对 episode runner

每个任务两次运行,唯一变量是树:

1. render(incumbent tree)→ workdir A,render(candidate tree)→ workdir B
2. 各自 `ZENE_HOME=<隔离目录> zene acp` 无头跑同一任务(复用 yolo + sandbox profile)
3. 读 `RecordEntry` 轨迹 → `EpisodeScorer::score`
4. 收集 `(candidate, incumbent)` 分数对 → `decide`

确定性要求(受 Reef episode 密闭性约束启发,但只取必要的):固定模型/温度、禁自更新、任务目录全新。**不追求跨机可复现**,同机同题配对即可。

### 3.5 版本化与回滚

轻量即可,不建 artifact 栈:

- **基线 = git commit**(harness 文件本来就在 workdir 里)。select → commit + tag `harness/<n>`;reject → 丢弃渲染产物
- **manifest**:每步一行(task 集、分数对、decision、commit id),append-only,格式对齐 `AgentRecordWriter`
- **回滚** = `git checkout harness/<n-1>`;周期复查可选,首版不做

## 4. 分阶段落地

| 阶段 | 交付 | 验证 |
|---|---|---|
| **P0** | `HarnessTree` + render(4 kind)+ Mutation 准入 | 渲染产物能被现有加载路径读到:render 后 `build_system_prompt_ext`/skill 发现输出与树一致 |
| **P1** | `EpisodeScorer` / `SelectionDecision` + 配对 runner | 人工构造一好一坏两棵树,win_margin 判定正确,manifest 落盘 |
| **P2** | propose 循环(可选) | 提案来自失败轨迹,准入拒绝越权变更 |

P0 本身独立有价值:树 = 可序列化的 harness 组合,直接服务 `zene acp` 分发、配置校验、多环境复用。

## 5. 安全边界(不进化的东西)

- **permission 规则、provider/端点/凭证永不入树**。理由与 Reef 的端点隔离一致:被优化的对象不能自己改优化器的靶心和钱包。Zene 侧比 Reef 更强的一点是 permission 在 `zene-permission` crate 里天然独立,树碰不到
- `prompt`/`rules` 是自由文本,视为不可信输入:scorer 只读轨迹,不信轨迹里的指令
- 涉及代码加载的 skill(如 `.agents/skills/*/SKILL.md` 引导执行的脚本)首版只允许文本变更

## 6. 明确不做

- 不搬 `cordis_backend` 的训练编排与 Slime 集成
- 不建 Reef 的 artifact 栈(git commit 足够)
- 不做自动 propose(P2 之前一切变更来自人写或外部脚本)
- 不引入 `native_graph`/`native_loop` 级别的控制流进化 —— Zene 的 turn loop 不是变异对象

## 7. 验证记录(verify-design-doc)

- §2 每条现状均出自 `rg` 实测输出,路径与行号如上;「评估层」一度缺口(`rg 'eval|score'` 无命中),现已由 `zene-eval` 补上,剩余缺口见 §2
- 本文无 SQL、无 mermaid(图均为 ASCII)
- 未验证项:P1 runner 与 `zene acp` yolo 模式的实际隔离性(多任务并发时 `ZENE_HOME` 是否充分隔离)需在 P1 动工时用并发 episode 实测
