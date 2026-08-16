---
name: archive-agent-notes
description: 管理 Agent Notes 生命周期，归档已实施的历史流水，提炼高频诱人错误为负向护栏
---

# Archive Agent Notes (决策记录生命周期管理)

管理 `.zene/notes/`（或 `docs/notes/`）下的决策记录生命周期。

## 状态与目录规则

- `active/`：现行核心架构公理、模块责任边界、协议语义。
- `negative-guardrails/`：被否决（rejected）的方案中，**仅保留那些“极其诱人、容易反复被提起的严重错误”**，写明失败原因与边界。
- `archived/`：已完成实现且不再具有跨时间指导意义的局部流水账。

## 归档动作

1. 检查 `implemented` 的 note：若属于一次性修复或细节流水，移至 `archived/`。
2. 检查 `rejected` 的 note：若不是高频诱人错误，彻底删除；若是，保留并移动至 `negative-guardrails/`。
