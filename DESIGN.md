# Zene Design System

**Console UI 唯一规范入口**（对齐 [XEnsemble DESIGN.md](https://github.com/ParaTensor/XEnsemble/blob/main/DESIGN.md) / ParaRouter Console 面）。`AGENTS.md` 与 Cursor 规则仅指向本文；**完整细则见 [`docs/Designs.md`](docs/Designs.md)**。

后端与网关协议以 `docs/WEB_AGENT_GATEWAY.md` 等架构文档为准；勿在本文重复 API 契约。

## Surfaces

### Console（authenticated / local agent）

本地 Web Agent（`apps/web-agent` → `zene-gateway`）、Cloud Console（`cloud/apps/web`）等控制台界面。

- **Accent**：black / zinc，**不用**紫色营销色、青绿霓虹、大面积 glow
- **Palette**：Morandi 浅色或等价 `zinc-*`（不用 `gray-*` / `blue-*` / `purple-*` 作 Console chrome）
- **字体**：`Inter`（UI）；等宽 `Menlo, Monaco, Consolas…`（代码 / 终端输出）
- **密度**：页面区块 `space-y-6` 量级；顶栏固定 `h-12`（48px）
- **圆角**：控件 `rounded-md`（6–8px）；卡片 / 弹窗 `rounded-lg` / `rounded-xl`（8–12px）
- **Shadow**：卡片 / 弹窗 `shadow-sm`；backdrop `bg-black/50`，**无** blur
- **Shell**：侧栏固定 **272px**（`#F4F5F6`）+ 白主区；中央主内容装在 `rounded-xl border shadow-sm` 卡片内（XEnsemble Sessions 终端卡位；Zene 用对话区替代终端）

### Marketing（公开落地页）

`www/`（及同步的 `web/` / `apps/web/` 副本）。可保留独立信息架构，但**视觉语言仍跟 Console**：浅底、zinc 边框、黑主按钮；**禁止**紫 / 靛营销渐变与深色霓虹终端皮肤。

### 非 Console 例外

Login / Register 等公开认证页可用居中卡片（`max-w-sm`），仍用 Console token，勿引入另一套品牌色。

## 实现对照

| 能力 | 位置 |
|------|------|
| 设计入口 | 本文 `DESIGN.md` |
| 完整细则 | `docs/Designs.md` |
| Local Agent UI | `apps/web-agent/index.html`（零构建，经 gateway `include_str!` 嵌入） |
| Cloud Console | `cloud/apps/web/dist/index.html` |
| Marketing | `www/`（源）；部署副本 `web/`、`apps/web/` 须同步 |
| Morandi token 参考 | XEnsemble `web/src/lib/consoleTheme.js` |

## 核心原则

1. **Content first** — 数据与操作优先于装饰
2. **Token reuse** — 共用 CSS 变量 / 类名语义，禁止每页自造主色
3. **表单** — 大写分区标签 + 输入；密钥 `font-mono`
4. **结构化弹窗** — Header / Body / Footer；固定宽度档位
5. **反馈** — 优先 toast / 顶栏状态点；禁止页面内联大红大绿 banner 作为主反馈
6. **页面稳定性** — 加载与状态切换不得导致可感知 layout shift；预留固定尺寸槽位

## 页面稳定性

Console 在连接、会话切换、面板开关时**不得出现可感知的布局位移**。

- 弹窗：固定档位宽度；Header / Footer 固定分区；Body 单独滚动
- 列表：Status / Actions 列宽固定；行内 loading 用同尺寸 spinner 原位替换
- 动画：过渡 ≤ `150ms`；禁止用动画掩盖布局跳动

## 配色（Console / Marketing chrome）

| 角色 | Token |
|------|-------|
| Canvas | `#FFFFFF` |
| Sidebar / secondary | `#F4F5F6` |
| Tertiary | `#FAFBFC` |
| Active | `#E8EAED` |
| 主文字 | `#202124` |
| 次文字 | `#5F6368` |
| Placeholder | `#9AA0A6` |
| 边线 | `#E8EAED` / `#DADCE0` |
| 主操作 | `#202124`（黑 / zinc-900） |
| 输入 focus | `border` + `ring` 用 `#202124` 或 `#5B8DB8`（二选一，全站统一） |
| Running / OK | `#4A7C59` |
| Danger | `#C06C5D` / `#FDECEA` |

## 布局骨架（Console）

```
┌─────────────┬──────────────────────────────────────────┐
│ Sidebar     │ Topbar (h-12)                            │
│ 272px       ├──────────────────────────────────────────┤
│ New Agent   │ ┌─ Main card (rounded-xl) ─────┐ ┌ Panel┐│
│ Search      │ │ Conversation / Agent stream  │ │ Act. ││
│ Sessions /  │ │ …                            │ │ …    ││
│ Agents      │ │ Composer                     │ └──────┘│
│ Account     │ └──────────────────────────────┘         │
└─────────────┴──────────────────────────────────────────┘
```

中间主卡在 XEnsemble 为 Web Terminal；在 Zene 为 **对话 / Agent 流**。右侧可选 Activity / Files / Overview 面板。
