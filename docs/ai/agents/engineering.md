# Engineering hard rules

## 一、禁止夹带变更（Silent Drift）

单个 commit / PR 不得夹带与声明目标无关的修改。特别是以下行为一律视为违规：

1. **调参私货**：以 "remediate" / "refactor" / "format" 名义修改默认值、阈值、超时、并发上限、限流配置等生产参数。
2. **批量重格式化**：`cargo fmt` / `prettier` / `black` 的结果必须单独 commit，不得与逻辑改动混在同一 commit。
3. **`#[allow(...)]` 压制**：用 `#[allow(clippy::xxx)]` 让警告消失，但没说明原因。
4. **跨模块 refactor**：修复 A 模块的 bug 时，不得同时重命名 B 模块的内部结构。

违规 commit 一律要求 `git reset --mixed HEAD~1` 重新拆分。

---

## 二、`git stash` 使用规则

`git stash` 极容易把 feature 依赖一起卷走。硬性约束：

1. **命名必须诚实**：`git stash push -m "..."` 不得用含糊描述掩盖逻辑改动。
2. **stash 前** `git diff --stat`，确认当前 feature 依赖不被抽走。
3. **stash 后必须** `cargo check --tests`（不可跳过）。
4. **不得 stash** `Cargo.toml` / `Cargo.lock` / 构建脚本。

完整步骤与误判症状见 skill [`git-stash-safe`](../../../.agents/skills/git-stash-safe/SKILL.md)。

---

## 三、引入后台任务 / daemon / background spawn 的规范

`tokio::spawn` / 其它 background task 是高风险构造，必须遵守：

1. **生命周期显式化**：不得在 `impl Default::default()` / `impl X::new()` 构造器里裸写 `tokio::spawn`。必须提供显式的 `start_background_xxx(&self)` 或类似入口方法，由 `init()` / `main()` 调用。
2. **Tokio runtime 守卫**：spawn 代码必须用 `tokio::runtime::Handle::try_current()` 包一层，或者只在已知 runtime 上下文中触发。避免同步测试/CLI 场景 panic。
3. **超时保护**：长跑任务里的每次异步调用都必须有 `tokio::time::timeout(...)` 兜底，避免 daemon 永久挂死。
4. **错误不得吞掉**：panic / timeout / calculate 错误必须 `tracing::error!` 至少记录一条，不得 silent `continue`。
5. **暴露 observability**：daemon 写入的共享状态（`Arc<AtomicX>` / `Arc<RwLock<X>>`）必须有至少一个 metric / debug endpoint 可以观察到最新值，否则无法判断 daemon 是否存活。

---

## 四、避免生成幻觉代码（Ghost Code）

以下模式一律视为幻觉：

1. **有定义无调用**：写了函数 / 类型，但全仓库 `grep` 找不到任何 caller。
2. **有路径无写入**：新增 `cached_xxx: Arc<AtomicU64>` 字段，有 load 点但没有 store 点。
3. **有埋点无分类**：metric counter 只埋上升事件（scale_up）不埋下降事件（scale_down），或反之。关键路径必须**正反两向**都有埋点。
4. **有 TODO 无 issue**：`// TODO: ...` 必须关联 GitHub issue 或 ticket。

发现此类幻觉，要求一次性补齐 caller / store / 下游埋点，不接受"下一轮再做"。

---

## 五、脚本化代码修改（`sed` / Python regex / awk）的规范

用脚本批量修改源码是高频犯错场景：

1. **regex 替换之后必须 grep 验证**。例如替换 `foo` 为 `bar`，必须 `rg "foo"` 确认剩余零匹配，或剩余匹配在预期内。
2. **`re.sub` 失败不会抛异常**。Python 正则不匹配时返回原串，不会报错。脚本跑完必须亲眼对比 diff 或重跑 grep 验证，不得直接 `git commit`。
3. **多行 regex 特别危险**：空白、引号、关键词拼写的任何一点偏差都会导致替换静默失败。优先使用 `StrReplace` 工具而非自写脚本。
4. **脚本内嵌字符串要小心转义**：Python 的 `'...'` 里如果包含 `"don't"` 这种，逃逸容易残留在最终代码里。脚本修改后必须人肉 review 一次生成的代码段。

---

## 六、AIMD / 限流 / 队列相关参数变更额外约束

`src/pool/` 下流控、限流、队列、AIMD、ETA、saturation 等默认值直接影响生产稳定性。任何默认值调整必须：

1. **单独 commit**，不得与 bug fix / refactor 合并。
2. **commit message 必须列出**：旧值、新值、调整理由、预期影响范围。
3. **至少跑一次 `cargo test --lib pool::`**。
4. **影响线上 SLO 的改动**必须在 `docs/changelog.md` 记录。

完整拆分与验证步骤见 skill [`pool-param-change`](../../../.agents/skills/pool-param-change/SKILL.md)。

---

## 十二、数据库迁移

新增 `migrations/NNN_*.sql` 后，必须按 skill [`add-sql-migration`](../../../.agents/skills/add-sql-migration/SKILL.md) 强制重建二进制；仅 `cargo build` 而不 `touch` 嵌入点时，新迁移常不会进二进制，极易误判为 SQL 写错。

---

## 十三、插件化优先 (Plugin-First Architecture)

针对非通用协议转换、业务侧定制逻辑，必须遵循**插件优先**原则：

1. **进入插件系统的场景**：
   - Agent Session ID 提取、解析与会话粘性/追踪；
   - 企业级 Prompt 模板注入、前缀/后缀装饰（如 `ai-prompt-decorator`）；
   - 数据脱敏、PII 拦截与合规替换（如 `ai-data-masking`）；
   - 基于客户端自定义 Header 或参数的动态路由分流（如 `ai-intent-router`）；
   - 结构化输出语法校验与修补（如 `ai-json-response`）；
   - 企业自定义 Bearer Token / JWT 签名校验（如 `custom-jwt-auth`）。
2. **禁止在内核主干硬编码**：禁止在 `src/adapter/`、`src/endpoints/` 内部为特定产品或租户硬编码 `if header == "x-custom" { ... }` 业务分支。
3. **内核只保留通用基础设施**：仅当功能属于全局通用设施（如上游 TCP 连接池、AIMD 熔断降级、KV-Cache LCP 亲和调度、令牌桶限流、多维配额等）时才置入网关核心。

---

## 十四、命令管道的退出码与「看似验证」陷阱

管道的退出码默认取**末位命令**：`失败命令 | tail -3` 的退出码是 tail 的 0。两个独立场景复现过此坑（CI watcher `| tail` 误报全绿；试合并 `| tail` 把 fatal 当成功打印 ✓）。

- **症状**：管道后的 `&&` 链继续执行；后台监听、脚本分支依据「末位命令」的退出码给出假结论。
- **正确做法**：判断上游成败时用 `set -o pipefail`（脚本内）、临时文件承接输出、或先跑命令再读输出；交互式判断「完成了吗」直接查权威源（`gh pr checks` / `gh run view` 的 conclusion 字段），不信代理进程退出码。
- **验证**：`bash -n` 只验语法；必须**实跑并人为制造一次上游失败**（如 `false | true`）确认链路中断。注意 `bash -o pipefail -n` 是无效组合——`-n` 不执行脚本，pipefail 不起作用，这本身就是一个「看似验证」。

---

## 八、违规处理

以下情形视为严重违规，审查方有权要求：

1. **报告与代码不符**：`git reset --hard` 回退到上一个诚实状态，重写提交。
2. **编译失败还报完成**：工作权限临时暂停，恢复前需提交书面复盘。
3. **重复同类错误 3 次以上**：该代理后续所有 commit 必须 pair review，两人 sign-off 才能合并。
4. **夹带生产参数私货**：立即 revert commit，重新按单独 commit 格式提交。

---

## 附：本规范的由来

本规范源自 2026-04-17 Phase 4 队列与 AIMD 优化的审查过程。该过程中发现以下反复出现的问题：

- 宣称"lock-free 全局令牌桶已实现"，实际代码里还是 `Mutex<TokenBucket>`
- 宣称"AIMD scale_down 埋点已补全"，实际 regex 静默失败，埋点从未落地
- 宣称"saturation daemon 每 5 秒刷新 cache"，实际 `cached_saturation_ratio.store()` 从未被调用
- 宣称"所有 49 个测试通过"，实际 `git stash` 把核心依赖抽走后根本不编译
- 在 remediation commit 里夹带 AIMD 默认值从 `5/100/10` → `100/500/5` 的生产参数调整

每一次都是"流程走得看似到位、实际验证没做"导致的。本规范旨在减少这类幻觉报告与夹带变更。
