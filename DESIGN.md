# Zene Design System

**Console UI 唯一规范入口**。完整细则见 [`docs/Designs.md`](docs/Designs.md)。视觉系统以 [`docs/design/`](docs/design/) 为准（Cursor 风格 AI 工作区）。

后端与 Cloud 协议以架构文档为准；勿在本文重复 API 契约。

## Surfaces

### Console（Cloud）

Cloud Console（`cloud/apps/web`）是唯一控制台界面。

- **Accent / Primary**：`#0090FF`（主操作、焦点、选中指示）
- **Ink**：`#1C2024`（主文字与关键信息）
- **Canvas / Surface**：`#FCFCFD` / `#F9F9FB` / `#FFFFFF`
- **字体**：`Inter`（UI）；`JetBrains Mono`（路径、代码、计数）
- **密度**：顶栏约 48–56px；侧栏 **272px**；列表行高 28–32px；间距 4 / 8 / 12 / 16 / 24
- **圆角**：2 / 4 / 8px（控件与面板）；避免大面积圆角营销卡片
- **层级**：1px 边界优先于阴影；卡片阴影极轻 `0 1px 2px rgba(0,0,0,.05)`
- **Shell**：侧栏 + 顶栏 + 主区；Run 为对话左 + 代码/变更右（保持现有信息架构）

拒绝：渐变壳、玻璃拟态、大面积装饰插画、Gold/Manrope 旧 Atelier 营销语言作为 Console 默认。

### 非 Console 例外

Login / Register 可用居中卡片，仍用同一套 token。

## 实现对照

| 能力 | 位置 |
|------|------|
| 设计入口 | 本文 `DESIGN.md` |
| 完整细则 | `docs/Designs.md` |
| Cloud Console | `cloud/apps/web/` |
| 对照稿 | `docs/design/` |

## 核心原则

1. **边界优先** — 用发丝线分区，而不是重阴影堆叠
2. **蓝色只做动作** — `#0090FF` 用于主按钮、焦点、选中；语义色留给成功/警告/错误软底
3. **紧凑可扫** — 技术信息用等宽字体；列表与工具栏保持 IDE 密度
4. **Token reuse** — Tailwind 语义色 / CSS 变量共用，禁止每页自造主色
5. **页面稳定** — 状态切换不得导致可感知 layout shift；过渡 ≤ 150ms

## 配色速查

| 角色 | Token |
|------|-------|
| Primary | `#0090FF` |
| Accent | `#0588F0` |
| Ink | `#1C2024` |
| Muted | `#60646C` |
| Canvas | `#FCFCFD` |
| Surface | `#F9F9FB` |
| White / panel | `#FFFFFF` |
| Selected / tint | `#E6F4FE` |
| Line | `#E6E8EB` |
| Success soft | `#E0FFE0` |
| Warning soft | `#FEF8E3` |
| Error soft | `#FEE8E7` |
| Info soft | `#E8F0FE` |

## 布局骨架（现有 Console IA）

```
┌─────────────┬──────────────────────────────────────────┐
│ Sidebar     │ Topbar (h-12)                            │
│ 272px       ├──────────────────────────────────────────┤
│ New Agent   │ New: 居中 Composer                       │
│ Agents      │ Run: Chat (左) │ Code / Diff (右)         │
│ Profile     │ Settings: 居中表单卡片                   │
└─────────────┴──────────────────────────────────────────┘
```
