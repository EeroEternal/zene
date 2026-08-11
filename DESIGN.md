# Zene Design System

**Console UI 唯一规范入口**。完整细则见 [`docs/Designs.md`](docs/Designs.md)。对照稿见 [`docs/design/`](docs/design/)（Cursor 风格：基础语言 / 组件库 / 审查流）。

后端与 Cloud 协议以架构文档为准；勿在本文重复 API 契约。

## 产品表面 vs 对照稿

| 层 | 是什么 | 颜色角色 |
|----|--------|----------|
| **Console（产品默认）** | `cloud/apps/web` 浅色壳：侧栏 + 主区 | 中性画布 + 蓝动作色；Ink 只做文字与品牌标 |
| **深色工作台（对照稿）** | `docs/design` 内的 IDE 样板 | 深墨面板；蓝仅服务选中 / 状态 / Diff，**不是**产品默认壳 |

不要把对照稿里的深色编辑器背景当成 Console 主题色，也不要把 Ink 近黑当成第二套「主题蓝」。

## Surfaces：Console（Cloud）

- **Primary**：`#0090FF` — 主操作、焦点环、选中指示（细条 / 勾）；hover 加深 `#0588F0`
- **Selected / tint**：`#E6F4FE` — 列表激活、轻量强调底
- **Hover 表面**：`#F9F9FB`（surface）— 次按钮、行悬停；**不要**用实心 Primary 铺大面积悬停
- **Ink**：`#1C2024` — 主文字、品牌 mark（如侧栏 / Auth 的 `Z` 方块）；**不是**动作主题色，也不再铺用户气泡
- **Canvas / panel**：`#FCFCFD` / `#FFFFFF`；次表面 `#F9F9FB`
- **字体**：`Inter`（UI）；`JetBrains Mono`（路径、代码、计数）
- **密度**：顶栏约 48–56px；侧栏 **272px**；列表行高 28–32px；间距 4 / 8 / 12 / 16 / 24
- **圆角**：2 / 4 / 8px；避免大面积营销圆角卡
- **层级**：1px 边界优先于阴影；卡片阴影极轻 `0 1px 2px rgba(0,0,0,.05)`
- **Shell**：侧栏 + 顶栏 + 主区；Run = 对话左 + CodePanel 右（保持现有 IA）

拒绝：渐变壳、玻璃拟态、大面积装饰插画、Gold/Manrope 旧 Atelier 营销语言作为 Console 默认。

### 非 Console 例外

Login / Register 可用居中卡片，仍用同一套 token（含 Ink 品牌 mark）。

## 全局视觉约定（与当前实现一致）

- **侧栏**：浅表面底（`#F9F9FB`），少用横线分割；靠间距与圆角浅底区分区块
- **侧栏 New Agent / Agent 行**：无描边框；选中白底轻阴影；悬停浅白底，可带 Primary 字色；分组下条目略缩进
- **表单主提交、危险确认后的明确 CTA、Auth 主按钮**：可用实心 `btn-primary`
- **对话**：用户与助手均左对齐；用户浅底圆角气泡（`#F9F9FB` + 细边）；助手无气泡；工具/系统等宽浅底
- **Run Banner**：标题行上下不加横线，用留白分区
- **Composer / 输入框**：浅色圆角输入面；上下无硬分割线；focus 略加深边

## 实现对照

| 能力 | 位置 |
|------|------|
| 设计入口 | 本文 `DESIGN.md` |
| 完整细则 | `docs/Designs.md` |
| Token / 组件类 | `cloud/apps/web/app/globals.css` |
| Cloud Console | `cloud/apps/web/` |
| 对照稿 | `docs/design/`（`page-1-foundations` 等） |

## 核心原则

1. **边界优先** — 发丝线分区，不用重阴影堆叠
2. **蓝色只做动作** — Primary 用于可点击主操作、焦点、选中指示；语义软底留给成功 / 警告 / 错误
3. **Ink 不做主题** — 近黑服务可读性与品牌标，不替代 Primary，也不与深色工作台背景混为一谈
4. **悬停用表面、选中用 tint** — 交互底优先 `#F9F9FB` / `#E6F4FE`，避免整控件长期铺满 `#0090FF`
5. **紧凑可扫** — 技术信息等宽；列表与工具栏保持 IDE 密度
6. **Token reuse** — 与 `globals.css` / Tailwind 语义色共用，禁止页内自造主色
7. **页面稳定** — 状态切换无感 layout shift；过渡 ≤ 150ms

## 配色速查

| 角色 | Token | 用途 |
|------|-------|------|
| Primary | `#0090FF` | 主操作 / 焦点 |
| Primary hover | `#0588F0` | 主按钮悬停 |
| Selected / tint | `#E6F4FE` | 激活行、轻强调 |
| Ink | `#1C2024` | 主文字、品牌 mark、用户气泡 |
| Muted | `#60646C` | 次要文案 |
| Canvas | `#FCFCFD` | 最外层画布 |
| Surface | `#F9F9FB` | 二级表面、悬停底 |
| White / panel | `#FFFFFF` | 面板 |
| Line | `#E6E8EB` | 发丝分割 |
| Success soft | `#E0FFE0` | 已验证 |
| Warning soft | `#FEF8E3` | 待确认 |
| Error soft | `#FEE8E7` | 错误反馈 |
| Info soft | `#E8F0FE` | 信息提示 |

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
