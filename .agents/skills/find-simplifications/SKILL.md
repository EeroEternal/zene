---
name: find-simplifications
description: >-
  Evidence-backed codebase entropy reclaim: find, rank, and safely delete unconsumed
  APIs, mirrored facts, speculative generality, extra layers, lifecycle duplication,
  and added-then-abandoned residue. Use when asked to simplify, clean up, reclaim
  entropy, reduce over-engineering, find deletion candidates, collapse duplicate
  state/APIs, or run a post-PR simplification pass. Also trigger for 代码化简、熵回收、
  删代码、清理冗余、收敛抽象、去除过度设计, /find-simplifications.
  Audit by default; apply only when asked. Not a performance audit.
---

# Find Simplifications

熵是团队必须持续对齐、却已无承重理由的事实、契约、状态和概念。扫描器只出候选；能否删，看消费者、所有权、历史和验证。宁可少而可证，不要长而投机。找不到安全可删项也是有效结论。

每提交数个 PR 后做一轮，优先从最大生产面开始，不要停在「没用到的符号」。

## 模式

- **审计**（find / review / 有什么可删）：只报告，不改仓库。
- **执行**（apply / simplify / 删掉）：只做已证明且用户点名或明确授权的切口，并验证。
- 删可达用户能力、公开 API、持久化格式或兼容路径 = 产品决策。用户没拍板就先摊开取舍。

## 先立契约

1. 读 `AGENTS.md`、相关 `docs/agents/*`、现行决策记录。有 `archive-agent-notes` 时用它判断 note 是否仍有效，不要在本 skill 里重写归档规则。
2. `git status`：别碰无关改动。先标出 generated / vendored / migration / fixture / 对外 crate 与 Console `@/cap/*`。
3. 顺着真实运行路径走：CLI `zene acp`、Cloud API/worker、Console capabilities、session/context/llm、ACP 协议、持久化与 git-broker。
4. 执行模式先弄清仓库里真正在用的窄/宽验证（`cargo test` / `cargo clippy`、`cloud/apps/web` 的 `npm test` / typecheck）。基线已红就记下来，别拿它当「没回归」。

默认保留、不得当「低成本清理」删掉：

- 信任边界校验、授权、数据丢失防护、资源真正停干净的 cleanup
- 刻意的双后端 / 双适配器 / capability 缝（有 note 或架构文档背书时）
- Console 已命名 capability；禁止再发明平行 picker/fetch/composer
- 持久化兼容、wire / ACP 字段仍被对端消费的路径

## 猎杀类型

1. **无生产消费者**：公开方法、导出、事件、配置旋钮、hook、包、registry、协议字段。
2. **镜像事实**：两套事件、缓存、摘要、状态或适配器必须同步同一真相。
3. **投机通用性**：从未拨动的开关、单实现接口、无主扩展点、abandoned stub。
4. **多余入口/层**：同一行为多扇前门、只转发的包装、单调用方的 helper 包。
5. **生命周期重复**：多个 flag / promise / disposer 表达同一次 ready / cancel / settle / dispose。
6. **错位防御**：对同进程、已类型化交接做 copy/freeze/hostile-getter 测试。
7. **手写基础设施**：标准库、已有依赖或平台已覆盖的 parser / retry / glob / diff。
8. **仅测试/文档承重**：行为本身不承重，测试或文档是唯一消费者。
9. **加了又撤**：实现没了，flag / schema / 文档 / 测试 / 兼容枝还在。

必要的独立性不是熵：刻意双后端、不同所有者的生命周期、测试一个契约的第二适配器。

## 证明或否决

对每个精确符号/行为：

1. 全库搜符号、路径、包名、配置键、事件/wire 字符串、`.name(` 与 `name(`。
2. 分类：生产（runtime、已发布配置、入口、migration、运维脚本）/ 非生产（测试、文档、注释、snapshot）/ 含糊（examples、反射、插件、对外导出）——含糊的先读再判。
3. 读调用点，不只数命中。查动态加载、字符串分发、路由、serde 字段、env 查找、ACP 方法名。
4. 读历史与 note：当初解决什么问题、问题是否还在、新证据是否压过原理由。
5. 异步/有状态代码画出所有权：每个 sentinel、ready、cancel、dispose 对应一个主人或一次转换。
6. 写明删掉会失去什么能力，哪怕答案是「对外不可见」。
7. 估净减：实现 + 仅为它存在的测试/文档/配置/依赖 − 留下的胶水和 migration。
8. 点名「若删错了，哪一条最小检查必红」。

出现任一则保留或降级：有生产/外部消费者；动态可达或兼容说不清；现行 note 仍成立；其实是产品决策；复杂度只是搬家；新依赖的胶水不比删掉的实现更轻；候选又小又飘、或不在本次范围。

审计条目用这个骨架：

```text
[confidence / risk] candidate
evidence: 生产消费者；动态/公开/兼容检查；现行理由
cut: 精确删什么（代码、产物、依赖、概念）
tradeoff: 可观察能力或行为损失
verify: 最小决定性检查；预估净减
```

极小且局部的清理用行内 `TODO(tag)` / `FIXME`，不要升格成 note。

## 执行一个已证明切口

1. 一次一个所有权边界。修熵的源头，不要给每个调用点打补丁。
2. 旧契约从头删到尾：声明、实现、仅服务旧行为的测试、导出、配置、文档、snapshot、依赖条目。
3. 留下描述仍存活、可观察契约的测试。测试是证据，不是金科玉律，也不是为了行数更好看就能扔。
4. 镜像状态折回承重表示。禁止用同步包装把「两个真相」伪装成一个。
5. 优先删除，其次标准库/已有依赖。新依赖只在净删实现+专属测试大于胶水与供应链成本时才加。
6. 没有兼容义务就不要留 shim。有义务就显式 migration，不要悄悄删。
7. 批次可审、可回滚。

行数变少是切口的证据，不是目标。安全化简可以加一条测试；大规模删除也可以是错的。

## 验证与回报

每批非平凡改动后：再搜已删符号/文案；先跑最小决定性测试，再跑相关 `cargo` / Console 检查；`git diff --check`；核对公开、持久化、wire、用户可见边界。

验证失败：判断是候选承重、实现不完整、还是基线已红。只回滚本批或补证据，不要为了删过去而削弱有意义的检查。

审计：按置信度、风险、净维护面排序。被否决的高价值项只在缺的证据可行动时列出。

执行：报告去掉了哪条契约或重复真相、可测的删减、用户可见变化、实际跑过的验证、故意留下的高价值候选及原因。

不要只凭测试全绿宣称安全，也不要只凭删除行数宣称有价值。
