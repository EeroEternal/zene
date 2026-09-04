---
name: review
description: 独立上下文的只读代码评审（critic）：新开会话或只读子代理执行，禁止在生成者对话内自评；先跑确定性门禁再读 diff，输出 file:line 级 findings 与 SHIP/REWORK 结论。Use when reviewing a PR, a finished task, or before requesting human approval.
---

# Review（独立只读评审 / critic）

## Symptom / misjudgment
生成者顺带「自查」会给自己打高分：同一上下文里既写代码又评代码，注意力已被实现
细节占满，看不见夹带、幻觉与更简单的做法。本仓库历史上多个问题（设计文档引用
不存在的能力、批量 sed 吃掉闭合括号产生 15 处残缺链接）都是在**独立复核**时才
暴露的，生成当时毫无察觉。

## Rule
1. **独立上下文**：评审必须在新会话或只读子代理（无 Write/Edit）中执行；禁止在
   生成者对话里「顺便看一遍」。子代理只带回结论，不共享思考过程。
2. **只读**：评审者不改文件。发现问题时报告 `file:line` 与建议修法，由生成者
   （或新会话）修复后重新评审。
3. **验证分层**：先用免费的确定性检查，全过再动用读 diff 的人工注意力——
   ```bash
   bash scripts/check_docs_lifecycle.sh   # 文档类改动
   bash scripts/check_release_sync.sh     # 版本/发版相关改动
   cargo fmt -p xrouter --check && cargo clippy -p xrouter --lib --tests -- -D warnings
   ```
   门禁红了先修门禁，不要带着红灯读代码。
4. **对照站立约束逐条问**：夹带无关改动？幻觉能力（有定义无调用 / 文档引用
   不存在的字段）？i18n 双语同步且无中英混杂？弹窗视口边界？排序标方向、
   placeholder 说真话？TODO 挂 issue？
5. **不止看对错，还看多余**：有没有更简单的做法？有没有删掉也能过的代码？
   「能删的行」和「能简化的分支」是评审的一等产出。

## Procedure
1. 确认输入：diff 范围（commit / PR）与声称完成的内容清单。
2. 跑第 3 条的确定性门禁；有红直接返回 REWORK + 红项。
3. 只读走查 diff：先看测试（断言是否真的断言了行为），再看实现，最后看文档。
4. 产出 findings 列表：`file:line [blocker|major|minor] 描述`；无 finding 才可
   给 SHIP。
5. 结论二选一：**SHIP**（可交付/可请求人工批准）或 **REWORK**（附 blockers）。
   不允许「基本可以」。

## Scope
- 本 skill 是评审动作的纪律，不替代 release skill 的 Phase 2 人工批准。
- 生成者修复 REWORK 项后必须重新评审，不允许「改完直接当过」。
