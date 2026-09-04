---
name: pre-push-local-gates
description: Push 前必须在本地跑满与 CI 等效的门禁(Rust fmt/clippy/tests),禁止把 CI 当本地沙盒。Use before every git push touching src/ or tests/.
---

# Pre-push local gates（推送前本地门禁）

## 核心痛点与禁止项 (Symptom / Misjudgment)
严禁把 CI 当本地沙盒：推送后才发现 lint 报错、rustfmt 未对齐、编译报警、测试失败，
形成「推 → 挂 → 本地修 → 再推」的低效循环——不仅污染提交历史、浪费 Runner 配额，
还可能在失败窗口内阻塞团队合并。

## 本地门禁执行标准 (Local Gate Checklist)
在执行 `git push` 或提 PR 之前，以下命令必须**全部在本地通过**（与 CI 等效）：

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
bash scripts/check_ui_stack.sh
bash scripts/check_admin_nav.sh
(cd admin && npx tsc -b --noEmit && npm run lint)
```

UI 规范改动需确保符合 `docs/design.md`；发版与打 Tag 前，转入 skill
[`release`](../release/SKILL.md) 执行完整发版流程（三查 + 人工批准硬停）。

## 适用范围与纪律 (Scope & Discipline)
- 开发过程中的中间 commit 允许临时不跑全量，但 **push 前最后一次提交必须全绿**。
- CI 如果意外挂了：禁止盲猜盲改，必须在本地先复现该失败的等价命令，本地确认修复通过后再推送。
