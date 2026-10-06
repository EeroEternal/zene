# Reef Harness 进化机制对 Zene 的启发

研究范围：Reef（Human-Agent-Society/reef，`reef-infra`）的 harness 进化面 —— harness tree、adapter、mutation、episode 评估与版本化发布。
**只做架构对照与产品取舍，不要求照搬实现或文件格式。**

相关 Zene 文档：

- [architecture.md](./architecture.md) — crate 边界与依赖原则
- [session-as-source-of-truth.md](./session-as-source-of-truth.md) — Session 事实 vs Context 投影
- [agent-components.md](./agent-components.md) — 可组装组件栈
- [context-engine.md](./context-engine.md) — ContextEngine
- [research/pi-agent-harness.md](./research/pi-agent-harness.md) — Pi 的最小核心 + 可扩展外围

外部参考：

- [Reef repository](https://github.com/Human-Agent-Society/reef)
- [Evolve your harness](https://reefinfra.ai/docs/user-guide/evolve-your-harness/)（机制主文档）
- [Harness adapters](https://reefinfra.ai/docs/developer-guide/harness-adapters/)（adapter / rendering 契约）
- [tutorials/evolve-your-harness](https://github.com/Human-Agent-Society/reef/tree/main/tutorials/evolve-your-harness)（最小可跑示例）
- 实现：`reef/harness/`（tree / adapters / episodes / runners / client）、`reef/train/cordis_backend/`（进化循环）

---

## 1. 一句话结论

Reef 最值得学的不是它的 RL 配方，而是它把 **"harness 是一棵可版本化、可变异、可评估的树"** 做成了完整契约：

> **harness tree（声明组合）+ adapter（渲染到具体 agent 文件）+ mutation（受控变更）+ paired episodes（同题对比评估）+ 版本化发布（赢了上线、输了回滚）。**

对 Zene 的四个主启发：

1. **把 harness 组合数据化**：rules / skills / prompts / config / 扩展代码都是树节点，而不是散落文件
2. **adapter 只做映射，不做业务**：一个 agent 一个 descriptor + quirks，树是中立的
3. **进化 = 受控变更 + 同题对比**：候选与在役版本在相同任务上跑配对 episode，赢了才发布
4. **变更要过准入（admission）**：每种节点有自己的校验，模型端点/凭证永远在树之外

## 2. Reef 是什么（harness 视角）

Reef 是持续自我改进 agent 的基础设施，坐在 harness 与模型之间，闭环为：

```text
Serve（服务并记录）→ Record（收据-反馈链接）→ Learn（recipe 产出候选）→ Publish（评估后发布/回滚）
```

它能进化两个面：**模型权重**（Slime + SGLang，GPU）和 **harness 树**（无 GPU，模型只是一个固定端点）。本文只看后者。

harness 的定义（Reef 文档原话）：

> everything around the model: the control loop, rules, prompt templates, skills, tools, config, and extension code

这与 Zene 对「harness」的边界理解基本一致 —— 差别在于 Reef 把这一切**收进了一个可变异的对象**。

## 3. 核心模型：harness tree

### 3.1 树的形状

一个 harness 的可变、可版本化文件收在一个 tree 对象里：扁平的 entry 列表，每个 entry 三个字段：

- `id`：树内唯一
- `name`：节点种类（node kind）
- `config`：该种类自己的字段

10 种节点分两组：

| 通用节点 | 渲染成 | 说明 |
|---|---|---|
| `config` | JSON 深合并进 agent 配置 | |
| `rules` | 追加到规则文件 | |
| `skill` | 命名 `SKILL.md` | |
| `agent_command` | 命名 prompt 模板 | Codex 下即 `$name` skill |
| `code_extension` | 进程内加载的代码文件 | 最窄：Codex 拒绝，Terminus 有条件接受 |

| native 节点（仅 Reef 自带 loop） | 渲染成 |
|---|---|
| `native_tool` | 命名工具（schema + code） |
| `native_hook` | loop 事件监听器（code） |
| `native_graph` | loop 控制流：stages + edges（data） |
| `native_agent` | 子 agent：prompt、graph、tools、skills、budget |
| `native_loop` | loop 本身作为代码 `run_turn(ctx)`，**永远需人工审核** |

native 五种的含义很激进：agent 不仅能改「文本外围」，还能改**自己的工具、钩子、控制流图、loop 代码**。`run_turn(ctx)` 拿到的 `ctx`（`ctx.model()` / `ctx.run_tools()` / `ctx.agent(name)` / `ctx.end()`）是 Reef 拥有的上下文 API —— loop 代码可以换，但预算、session 日志、沙箱仍是平台的。

### 3.2 kind 说"是什么"，adapter 说"放哪"

- **节点 kind** 只声明 entry 内容，不声明落盘位置
- **adapter** 把 kind 映射到某个 agent 的具体文件（pi 的 `settings.json` / `AGENTS.md` / `skills/<name>/SKILL.md` / `extensions/<name>.ts` …）
- **rendering** 不是拷贝：`config` 深合并、`rules` 拼接
- adapter 不渲染的 kind 在准入和渲染时**明确拒绝，而不是静默丢弃**

已带 adapter：`pi`、`opencode`、`claude`、`codex`、`dsh`、`hermes`、`terminus`（Terminal-Bench Terminus 2 + Harbor runner）、`native`。

每个 adapter 附带 `harness_facts.yaml` —— 提案模型读的"这个 harness 长什么样"事实面，以及 quirks（各 agent 的怪癖适配）。

### 3.3 模型端点永远在树外

树不决定模型调用去哪。Reef 的 model binding 在渲染时写入 endpoint / key / model；`claude`、`dsh`、`hermes`、`pi`、`terminus` 的渲染会**拒绝**树里任何设置 provider、transport、proxy、凭证、fallback 的 config 或命令（包括请求体里藏的）。

> 注意其诚实边界：这些检查读的是渲染出的配置，不是运行时请求 —— 工具/插件自己构造的请求仍可能指向别的模型。文档明说了这一点。

## 4. 进化循环：一步（step）的解剖

```text
snapshot 快照树
   → propose  提案：一个 mutation、一组原子 mutation、或 None
   → apply    在快照上应用（create / update / remove）
   → episodes 候选树 vs 当前树，同一批任务，各自全新目录跑无头 episode
   → publish  赢了 → 版本化发布；输了 → 恢复快照
```

关键实现事实：

- **实现位置**：循环本身在 `reef.train.cordis_backend`（propose、gate、publish）；版本化（staging、publish、commit log、恢复、回滚）在 `reef.artifact` + `reef.scenario.committer`
- **mutation 契约**：`create` 拒绝已有 id、`update` 拒绝缺失 id 或改 kind、`remove` 拒绝缺失 id；拒绝组 entry 和保留 id
- **准入（admission）与服务共享**：`reef.harness.tree.mutations` 的准入规则在训练和常驻服务两条路径上同一份 —— 进化出来的树和人写的树过同样的检查
- **树状态随 algorithm state 走**（`"entries"` key），由 commit 记录持久化和恢复 —— 所以崩溃恢复后树与版本历史一致
- **步的触发**：`training_mode` = `auto`（记录流量驱动）/ `manual`（`POST /reef/train` 驱动）/ `hybrid`（两者，训练请求优先）

## 5. 怎么判定成功

### 5.1 两层判定

1. **episode 打分**（方法提供）：`EpisodeScorer` 对单个 episode 结果打分（tutorial 里是精确最终答案比对；GEPA 是任务级评分）
2. **候选选择**（`CandidateEvaluationPlugin`）：`evaluate(candidate)` 量指标，`decide(candidate, evaluation)` 出 `SelectionDecision`（`outcome` + `policy` + `policy_version` + `reason`，持久可审计）

内置选择策略（cordis_backend）：

- **ScoreComparison**（默认）：候选任务**赢的次数多于输的次数**（可设 margin）
- **Floor**：候选单独跑，所有任务得分 ≥ `floor_score` 才发布

### 5.2 失败驱动的提案

cordis_backend 的 `FailureManifest` / `FailureObservation` 把失败记录整理给 propose —— 提案者（可以是被服务的模型自己）读当前树 + 失败请求（带分数和反馈），提出严格 JSON 的 mutation。`untrusted_text` 标注了提案里不可信文本的边界。

## 6. 护栏（这是机制的另一半）

| 护栏 | 行为 |
|---|---|
| 落选即回滚 | 恢复快照，树不留脏状态 |
| 人工审核 | `native_loop` 等高危 kind 的胜出永远是 pending release，人 promote 才上线；`harness_try` 拒绝把 loop 挂到服务进程 |
| 失败样本入题库筛查 | 凭证 + 指令覆盖筛查，每个客户端有上限 |
| 周期复查回滚 | 题库长大后重评已发布版本，判退步则回滚 |
| 沙箱 episode | 无宿主凭证、网络只通模型端点；残留文件可直接判 0 分 |
| 端点隔离 | 见 3.3，树碰不到模型端点和凭证 |

## 7. 与 Zene 的架构对照

| 维度 | Reef | Zene 现状 | 差距 / 契合 |
|---|---|---|---|
| harness 组合 | harness tree：扁平 entry + 10 种 kind | agent-components：可组装组件栈 | Zene 有组件思想，但**未数据化成可变异对象** |
| 落盘映射 | adapter：descriptor.yaml + quirks + harness_facts | 各组件自己管自己的配置 | Reef 的"kind 中立、adapter 映射"更利于多宿主 |
| 变更单位 | Mutation（create/update/remove）+ 准入 | 文件/配置直接改 | Reef 变更有审计与拒绝语义 |
| 评估 | paired episodes + EpisodeScorer + SelectionDecision | 评测散在外部 | Reef 把"对比评估 + 选择决策"做成了契约 |
| 版本化 | artifact 栈：staging/publish/commit log/回滚 | CHANGELOG / 手动 | Reef 全自动且崩溃一致 |
| 事实源 | records（收据-反馈链接） | session-as-source-of-truth | **理念同构**：都拒绝"模型碰巧看到的即历史" |
| 进化对象 | 文本 + 代码 + 控制流图 + loop 代码 | — | native 五 kind 是最激进部分 |
| 安全边界 | 端点/凭证树外 + 沙箱 + 人工审核 | sandbox / permission crates | 关注点一致，Reef 补的是"学习回路里的安全" |

## 8. 建议优先级（给 Zene）

1. **P0 — 借概念，不借代码**：把 Zene 的 agent 组合（rules、skills、prompts、config）表述为**树 + 节点 kind**。哪怕暂时没有进化功能，"组合 = 可序列化数据"直接服务于 Zene 自己的 `zene acp` 分发、配置校验和多宿主渲染。
2. **P1 — 同题对比评估契约**:如果 Zene 要做 harness 优化(哪怕只是 A/B 两个 prompt 组合),先定义 `evaluate(episode) -> score` + `decide(candidate, incumbent) -> SelectionDecision` 这一层,再考虑自动化。Reef 的教训是:**判定必须持久、带策略签名、可审计**。落地设计见 [harness-evolution.md](./harness-evolution.md)(路线 B:机制 native 化)。
3. **P2 — adapter 边界**:Zene 若需输出到 Claude Code / Codex / pi 格式,直接采用"kind 中立树 + 每宿主一个 descriptor"的形状,渲染语义(合并 vs 拼接)要显式。
4. **P3 — 端点与凭证隔离**：任何"让模型改自己配置"的功能，端点/凭证必须在被改对象之外 —— Reef 连渲染期拒绝列表都备好了，这是最低成本的安全设计。
5. **P3 — 高危变更人工 promote**：代码类变更（对应 `code_extension` / `native_loop`）走 pending release + 人审，不要直接生效。

## 9. 明确不照搬

- **cordis_backend 的训练编排**：Reef 是 Python 训练栈 + Slime，与 Zene（Rust、ACP、无训练面）无关
- **10 种节点 kind 的全集**：Zene 没有 native loop 进化需求就不要引入 `native_graph` / `native_loop` 这类抽象
- **GEPA / SAO 等 recipe 生态**：方法层是 cookbook，Zene 需要时对接外部评估即可
- **artifact 栈的完整实现**：git-LFS + repository 的版本化很重；Zene 用 crate 边界内的轻量 manifest 足够起步

## 10. 四句收束

1. Reef 把 harness 从"一堆文件"升维成"一棵可变异、可评估、可版本化的树"。
2. adapter 把树渲染到具体 agent，kind 中立让同一套进化机制服务所有宿主。
3. 进化 = 受控 mutation + 同题配对 episode + 带签名的选择决策，赢了发布、输了回滚。
4. 对 Zene：先要"组合数据化"和"评估契约"这两块地基，自动化进化是它们之上的可选层。
