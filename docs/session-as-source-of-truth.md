# Session 是事实来源，Context 只是投影

> Session 记录「发生过什么」（事实日志）。  
> Context 是「下一次发给模型看什么」（运行时视图）。  
> 两者相关，但不是同一件事；也绝不能反过来让 Context 变成事实来源。

本文是架构心智模型，不是实现清单。对照实现见 [ENGINE.md](./ENGINE.md)、[context-engine.md](./context-engine.md)、[agent-inference-context.md](./agent-inference-context.md)、[agent-components.md](./agent-components.md)。  
控制面（谁在跑、命令与状态归谁）见 [agent-runtime-optimization.md](./agent-runtime-optimization.md)；Context 投影落地见 [context-engine-projection.md](./context-engine-projection.md)。  
灵感来源：Pi Session Tree（JSONL event tree + `buildSessionContext` 投影）；Zene 不必照搬其格式。更完整的 Pi→Zene 对照见 [pi-agent-harness-lessons.md](./pi-agent-harness-lessons.md)。

---

## 1. 先分清两样东西

### Session = 事实来源（Source of Truth）

Session 回答的是：

- 用户说了什么
- 模型回了什么
- 调了哪些工具、结果是什么
- 谁被拒绝了、谁取消了
- 换过什么模型
- 做过几次 compaction
- 用户何时 fork / rewind / 换分支
- 有哪些自定义状态、label、checkpoint

它是 **append-only（或近似 append-only）事件历史**：完整、可回放、可审计、可分支。

像黑匣子：

```text
t1  user: 修一下登录 bug
t2  assistant: 我先读 auth.rs
t3  tool: read auth.rs → ...
t4  assistant: 准备改 validate()
t5  tool: edit auth.rs → ok
t6  user: 别动测试
t7  compaction: 总结了前半段
t8  assistant: 继续改...
```

这些「发生过」的事，原则上都不该因为 token 不够就被抹掉。

### Context = 投影（Projection）

Context 回答的是：

> **这一次 LLM 请求，模型实际看到了哪些消息？**

它是一个 **从 Session 算出来的视图**，会做很多变换：

- 截断过长 tool output
- 丢掉太旧的中间过程
- 插入 system prompt / memory / todo reminder
- 用 compaction summary 代替旧历史
- 只取当前 branch 的 active path
- 过滤只给 UI 看的消息
- 转成 OpenAI / Anthropic 各自需要的格式

所以 Context 更像：

```text
Session 事件树
      │
      ▼
  过滤 / 裁剪 / 总结 / 注入
      │
      ▼
  本次 Provider Request 的 messages[]
```

它是 **当前这一刻对模型有用的摘要视图**，不是完整历史本身。

---

## 2. 日常类比

把 Session 想成 **Git 仓库**：

- 每次 commit / 分支 / merge 都真实发生过
- 历史可以很长
- 你可以 checkout 到任意点
- 你可以 branch 出另一条探索路径

把 Context 想成 **当前工作区 + 你这次 review 时贴给同事看的 diff 摘要**：

- 同事不需要整个 git history
- 你只给他「目标、约束、当前状态、关键文件」
- 这个摘要可以变，但不代表 git history 没了

Pi 的做法接近这一模型：

- Session JSONL 一直完整保存
- Compaction 只是在树上多挂一个 `compaction` entry
- 下次 build context 时，用 summary + 最近消息
- 老消息还在文件里，可以用 tree 导航回去看

---

## 3. 为什么必须是这个方向

### 正确方向

```text
Session（事实）
   └──► Context（投影给模型）
   └──► UI transcript（投影给用户）
   └──► Replay / Analytics（投影给调试与分析）
   └──► Export（投影给分享）
```

所有视图都从 Session 派生。

### 错误方向

```text
Context / messages[]（当前给模型的那一包）
   └──► 这就是「会话历史」
```

一旦把「当前 messages 数组」当成唯一真相，会出现这些问题：

| 问题 | 原因 |
|------|------|
| compaction 后历史不可逆 | 旧消息直接被删改，不在 session 里留痕迹 |
| fork / rewind 很难做对 | 没有树，只有一条被不断改写的线性数组 |
| UI 和模型看到的东西纠缠 | UI 想显示工具详情，模型却不能塞太多 |
| 无法解释「模型为什么这么说」 | 说不清当时投影规则 |
| 调试 / 回放失真 | 录到的是投影后的结果，不是原始事件 |
| 扩展状态无处安放 | todo、permission、custom state 只能硬塞进 messages |

---

## 4. Pi 参考模型（概念，非格式规范）

Pi Session 不是简单的：

```ts
messages: Message[]
```

而是 JSONL 事件树：

```text
entry {
  id
  parentId
  type: message | compaction | model_change | branch_summary | custom | ...
}
```

然后每次发模型前再投影：

```text
leaf
  → 沿 parentId 走到 root   // 得到当前 branch
  → 遇到 compaction 就折叠旧段
  → 过滤 UI-only / custom state
  → convertToLlm()
  → 发给 provider
```

因此：

- **Branch** 改变的是 leaf，不是复制一份 messages
- **Compaction** 追加一个 compaction entry，不是销毁旧 entry
- **Custom state** 可以存在 session 里，但不一定进模型
- **完整历史**始终可回看

Zene 不必采用 JSONL 或 `parentId` 树，但应保留这层语义：

> Session 是事实；Context 是投影。

---

## 5. 对照 Zene 现状

Zene 今天大致是：

```rust
SessionRecord {
  messages: Vec<Message>,      // 很像「当前上下文」
  compactions: Vec<...>,       // 压缩记录
  todos: ...,
  ...
}
```

外加：

- checkpoint 文件
- compaction segment
- record writer（部分运行事件）
- ACP replay
- Cloud event stream

也就是说：Zene **已经有多个投影 / 旁路记录**，但还没有把「完整事件历史」明确提升为单一事实源。

目前比较容易滑向的状态是：

```text
session.messages ≈ 当前给模型看的上下文
```

而不是：

```text
session.events = 全部发生过的事
session.messages_for_llm = 从 events 投影出来的视图
```

因此下一阶段最有价值的，往往不是再加一个 compaction 算法，而是把「会话真相」从「可变 messages 数组」升级成「稳定事件日志 + 多层投影」。

---

## 6. 具体例子

用户做了这些事：

1. 让 Agent 重构支付模块
2. Agent 读了 12 个文件，改了 3 个文件
3. 用户说「停，方案 A 不行，换方案 B」
4. 发生一次 auto-compact
5. Agent 在方案 B 上继续
6. 用户后来想回到方案 A 分叉前再试一次

### 如果 Session = messages 数组

- compaction 后，早期细节可能已经丢了
- 「换方案」只是后面又追加了几条 user/assistant
- 想回到分叉点，只能靠不完整 checkpoint，或近似 rewind
- UI、模型、回放看到的历史可能都不一致

### 如果 Session = 事件树，Context = 投影

- 方案 A 路径完整保留
- 方案 B 是另一条 branch
- compaction 只影响「当前发给模型的视图」
- 仍可树上游走、fork、对比
- UI 可以显示完整工具过程
- 模型只看 summary + 最近 turn + 关键文件

同一份事实，多种用途。

---

## 7. 同一份 Session 至少可派生这些视图

```text
1) LLM Context
   - system + 保留的历史 + summary + 最近工具结果
   - 目标：便宜、够用、稳定

2) UI Transcript
   - 更完整的工具输出、权限询问、状态提示
   - 目标：人能看懂

3) Replay / Debug
   - step、usage、cancel、permission、model switch
   - 目标：可复盘

4) Branch View
   - 当前 leaf 到 root 的路径
   - 目标：探索与回退

5) Analytics
   - token、成本、工具成功率、压缩次数
   - 目标：产品与模型改进
```

这些都可以不同，也 **应该** 不同。  
但它们必须来自同一套 Session 事实。

---

## 8. 对 Zene 的落地含义

不需要立刻改成 Pi 的 JSONL，但架构上建议固定成：

### A. Session 负责记录

逐步让 Session 成为这些事实的归宿：

- message
- tool call / tool result
- permission decision
- model change
- compaction
- branch / fork / rewind
- checkpoint marker
- custom / extension state

### B. ContextEngine 只负责投影

`prepare_step` / compact / assemble 的输入应是 Session 事实，输出是：

```text
这次 LLM 请求该看的 messages + metadata
```

而不是「偷偷改写唯一历史」。

### C. Compaction 是追加事件，不是销毁历史

压缩后：

- Session 里仍能看到旧内容，或至少能指向 segment / checkpoint
- Context 用 summary 替代旧段
- UI 能解释「压缩了什么、保留了什么」

### D. UI / ACP / Cloud 也不要私自再造一套真相

它们都应订阅或读取同一 session event 流，再各自渲染。

对外实时流优先走统一 **RuntimeEvent**（见
[agent-runtime-optimization.md](./agent-runtime-optimization.md)），
由 Conversation SoT / Execution record **投影** 而来，而不是 ACP / Cloud 各自再记一份对话真相。

### E. Conversation SoT ≠ Execution record ≠ RuntimeEvent

| 名称 | 职责 |
|------|------|
| **Conversation SoT** | message、compaction、fork…（内容真相） |
| **Execution record** | step/tool/approval 进度（运行真相，可恢复） |
| **RuntimeEvent** | 带 sequence 的对外实时流 |

三者 **ID 空间统一**，职责不合并成一个万能日志。控制面（cancel/steer/approval）的单所有者是 **AgentRuntime**，不是 ContextEngine。

---

## 9. 最小心智模型

```text
          ┌──────────────────────────┐
          │   Session Event Log      │  ← 事实：发生过什么
          │   (tree / append-only)   │
          └────────────┬─────────────┘
                       │
       ┌───────────────┼────────────────┐
       ▼               ▼                ▼
  LLM Context     UI Transcript     Replay/Export
  (给模型看)       (给人看)         (给系统看)
       ▲
       │
  Context Engine
  过滤 / 截断 / 总结 / 注入 / 格式转换
```

更直白一点：

> **不要让「模型这一次碰巧看到的那包 messages」变成会话历史。**  
> **会话历史应独立存在；模型看到的内容，只是根据策略从历史里算出来的一个视图。**

---

## 10. 落地顺序（与 Runtime 合并）

数据面不要单独空转；与控制面合并 Wave 见
[agent-runtime-optimization.md §16](./agent-runtime-optimization.md#16-merged-implementation-waves)：

```text
Wave 1  统一 ID + RuntimeEvent 信封
Wave 2  Conversation SoT 双写（本文主场）
Wave 3  Context observe/commit/project
…
```

第一期：双写、兼容旧 load、**不**切换默认读路径。
控制类事实（steer/cancel/approval）预留事件或 execution record 挂钩，避免日后 Runtime actor 无法对齐。

---

## 相关文档

- [ENGINE.md](./ENGINE.md) — turn / steer / compaction 行为
- [context-engine.md](./context-engine.md) — ContextEngine 边界
- [context-engine-projection.md](./context-engine-projection.md) — Context 投影化优化路线
- [agent-runtime-optimization.md](./agent-runtime-optimization.md) — AgentRuntime / Turn / ports（控制面）
- [agent-inference-context.md](./agent-inference-context.md) — 推理上下文装配
- [agent-components.md](./agent-components.md) — 可组装组件栈
- [pi-agent-harness-lessons.md](./pi-agent-harness-lessons.md) — Pi Agent Harness 对照与启发总览
