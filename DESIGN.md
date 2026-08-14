# Zene Design System

**Console UI 唯一规范入口**。完整细则见 [`docs/Designs.md`](docs/Designs.md)。对照稿见 [`docs/design/changes-and-checks.png`](docs/design/changes-and-checks.png)。

后端与 Cloud 协议以架构文档为准；勿在本文重复 API 契约。

Zene Cloud Console 是面向开发者的 Agent 工程工作区。视觉参考 VS Code 工作区、GNOME 克制实用、以及 Git / CI 的工程感。不采用 SaaS 营销或 AI 产品风格。

核心体验：知道 Agent 在做什么、为什么这样做，也能在必要时接管。

## Surfaces：Console（Cloud）

- **画布** `#F6F5F4` · **导航** `#EBE9E7` · **面板** `#FFFFFF`
- **Ink** `#2E3436` · **Muted** `#687174`
- **Primary** `#3584E4` — 选中、运行中、主按钮、链接；不铺大面积
- **等待 / 审批** `#E5A50A` / `#FFF5D6`
- **成功** `#287A46` / `#EAF4EC` · **失败** `#C01C28` / `#FCEBEC`（必须配文字）
- **字体**：Inter（UI）；等宽用于路径、命令、日志、Diff
- **密度**：窄工具栏 ~52px；项目导航 220–240px；控件高约 32px
- **圆角**：4 / 6 / 8px；阴影 `0 1px 2px rgba(46, 52, 54, 0.08)`
- **壳层**：窄工具栏 | 项目 / 上下文导航 | 主工作区

Login / Register 可用居中卡片，仍用同一套 token。深色只用于终端 / 长日志块，不是整页主题。

## 布局骨架

```
┌──────┬──────────────┬─────────────────────────────────┐
│ 52px │ 220–240px    │ 主工作区                         │
│ 图标 │ 项目 / 上下文 │ 概览 · 运行工作台 · 变更与检查   │
│ 工具栏│ Agent 导航   │ New Agent 空态：居中 Composer     │
└──────┴──────────────┴─────────────────────────────────┘
```

Zene 页面对应：New Agent = 新建任务；Run 对话 = 运行工作台（意图 / 动作 / 证据）；CodePanel = 变更与检查；Settings = 工具栏设置。

## 反馈与提醒

操作结果用 **Toast**（`components/Toast.tsx` 的 `useToast()`）。禁止为提醒预留固定 layout。任务摘要条、审批区、检查结果是工作区内容，不是 Toast。确认与危险操作用应用内模态，禁止 `alert` / `confirm` / `prompt`。

## 核心原则

1. **过程优先** — 让用户看到 Agent 正在做什么
2. **证据优先** — 结果关联文件、命令和测试
3. **人工可接管** — 批准、拒绝、暂停、重试是正常流程
4. **工程化表达** — 任务、步骤、命令、Diff、检查；不用营销文案
5. **安静克制** — 少品牌色、装饰、阴影和大圆角
6. **状态明确** — 颜色必须配文字（如 `失败 · 命令退出码 1`）
7. **Token reuse** — 与 `globals.css` 共用，禁止页内自造主色
8. **页面稳定** — 过渡 ≤ 150ms，避免 layout shift

## 实现对照

| 能力 | 位置 |
|------|------|
| 设计入口 | 本文 `DESIGN.md` |
| 完整细则 | `docs/Designs.md` |
| Token / 组件类 | `cloud/apps/web/app/globals.css` |
| Cloud Console | `cloud/apps/web/` |
| 对照稿 | `docs/design/changes-and-checks.png` |
