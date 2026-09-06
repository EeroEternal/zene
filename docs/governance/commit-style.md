# Commit message style

## 七、Commit message 规范

格式：`<type>(<scope>): <subject>`

常用 type：
- `feat` — 新功能
- `fix` — bug 修复
- `refactor` — 重构（不影响行为）
- `perf` — 性能优化
- `test` — 只改测试
- `docs` — 只改文档
- `chore` — 构建、发版、依赖升级等

**subject 必须反映真实内容**。禁止：
- `fix: misc` / `chore: cleanup` 这类无信息 message
- `feat: complete phase N` 但 commit 里有 unrelated 调参
- 用 commit message 隐瞒夹带变更

**长 body 推荐**包含：
- 改了什么（what）
- 为什么改（why）
- 风险点 / rollback 方法（how to revert）

---
