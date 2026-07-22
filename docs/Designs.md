# Zene UI Design

**Console UI 完整规范**（根目录 [`DESIGN.md`](../DESIGN.md) 为 Agent/规则入口；本文为细则正文）。视觉与布局对齐 [XEnsemble DESIGN.md](https://github.com/ParaTensor/XEnsemble/blob/main/DESIGN.md) 与 XEnsemble `docs/Designs.md` 的 Console 面；产品语义按 Zene（本地 Agent / Cloud / 营销站）裁剪。

## Surface：Console

本地 Web Agent、Cloud Console、Workspace Settings 弹窗等均属 Console。

- **主色**：黑 / zinc（`#202124`），不用紫色营销色或青绿霓虹。
- **字体**：Inter；代码区 Menlo / Monaco / Consolas。
- **密度**：顶栏 48px；侧栏 272px；主区卡片内边距约 16px。
- **圆角**：控件 `6–8px`；卡片 / 弹窗 `8–12px`。
- **壳层**：`h-screen` 横向两栏；主区纵向顶栏 + 内容；中央内容装圆角边框卡片。

## Surface：Marketing

落地页（`www/`）用同一套 zinc / 黑主操作语言：浅底、细边框、黑按钮、Inter。可用居中窄栏（约 660–720px）承载文档型内容；**禁止**紫渐变、暗色霓虹 mock、大圆角营销卡片堆叠。

中间「产品示意」区展示 **Zene 对话卡**（对齐 Console 主卡），不要复刻旧版紫色 TUI 截图皮肤。

## 实现对照

| 能力 | 位置 |
|------|------|
| 规范入口 | `DESIGN.md` |
| Local Agent | `apps/web-agent/index.html` |
| Cloud | `cloud/apps/web/dist/index.html` |
| Marketing | `www/index.html` + `www/styles.css`（同步到 `web/`、`apps/web/`） |

## 按钮

- **主操作**（Connect、Send、Start Agent、Sign in）：实心黑底白字；disabled `opacity: .4`。
- **次操作**：白底 + `border #E8EAED`；hover `#F4F5F6`。
- **危险**：文字 / 边框 `#C06C5D`，软底 `#FDECEA`。
- **图标按钮**：仅图标，`title` + `aria-label`；尺寸工具栏约 32×32，图标 16×16；hover 浅灰底。
- **禁止**图标与长文案并排塞进工具栏图标槽。

## Toast / 反馈

- Cloud / 多步操作：顶部或角部短 toast，约 4s；成功 / 失败用边框色区分，勿整页 banner。
- Local Agent：连接态用顶栏状态点（`#4A7C59` / `#C06C5D` / `#9AA0A6`）；详细文案可放 Settings 内 `status` 行。
- **禁止**用紫色 / 霓虹强调成功态。

## 表单

- 分区标签：`11–12px`、`uppercase`、`tracking-wider`、`#5F6368`。
- 输入：白底、`border #DADCE0`、`rounded-md`；focus 黑或统一蓝灰描边 + 1px ring。
- 密钥 / token：`font-mono`。
- 人类可读标签；不在 UI 上直接暴露裸 env 名作主标签。

## 弹窗（Settings / Dialog）

对齐 XEnsemble 结构化弹窗：

| 档位 | 宽度 | 用途 |
|------|------|------|
| sm | ~384px | 确认 |
| md | 480px | Workspace / 登录后设置 |
| lg | 560–800px | 多分区设置（若需要） |

- Backdrop：`rgba(0,0,0,.5)`，无 blur。
- Header / Body / Footer 分区；Footer 浅灰底 `#FAFBFC`，右对齐 Cancel + 主操作。
- Esc / backdrop 关闭；关闭按钮不得导致标题行跳动。

## 侧栏

- 宽 **272px**，背景 `#F4F5F6`，右边线 `#E8EAED`。
- 顶部：New Agent + Search。
- 中部：Sessions（Local）或 Agents（Cloud），可滚动。
- 底部：账户 / Workspace 入口。
- 列表项：`13px`，active `#E8EAED`，hover `#FAFBFC`。

## 主区与中央卡片

- 顶栏：`h-12`、白底、底边线；左标题 + 模式 / 状态；右图标操作。
- 中央：`p-4` 内 `rounded-xl border #E8EAED shadow-sm` 卡片。
  - Local / Cloud：**对话流 + Composer**（替代 XEnsemble 的 xterm）。
  - 空态：居中图标槽 + 一行标题 + 一句说明（`#9AA0A6`）。
- 右侧面板（可选）：Activity / Files / Overview；可用宽度约 280–360px；顶栏同高切换按钮。

## 对话与消息

- 用户气泡：右对齐，黑底白字，圆角偏右下收束。
- 助手：左对齐，左侧 `Z` 方标（`#F4F5F6` 底）。
- Thought：左边线 + 次要斜体字。
- Tool / system：等宽、浅底卡片或居中 meta，避免花哨色块。

## 表格与列表（Marketing commands、Cloud 列表）

- 一列一项；超出 `truncate` + `title`。
- 边框 `#E8EAED`；表头次要色；行 hover `#FAFBFC`。
- 状态切换不得撑开列宽。

## 动画

- 过渡 ≤ `150ms`（`color` / `background` / `border`）。
- 禁止长动画、视差、大面积 blur 掩盖位移。

## 配色速查

| 角色 | 值 |
|------|-----|
| Canvas | `#FFFFFF` |
| Secondary | `#F4F5F6` |
| Tertiary | `#FAFBFC` |
| Active | `#E8EAED` |
| Ink | `#202124` |
| Muted | `#5F6368` |
| Placeholder | `#9AA0A6` |
| Line | `#E8EAED` |
| Line strong | `#DADCE0` |
| OK | `#4A7C59` |
| Danger | `#C06C5D` |
| Warn soft | `#FFF8E8` / `#E8B339` |

## 禁止事项

- 紫色 / 靛色营销 accent（含 focus ring）
- 深色渐变壳 + 霓虹描边作为 Console 默认主题
- Marketing 大圆角英雄卡片、统计条、贴纸式 badge 堆叠进首屏（Console 更是禁止）
- 每页面各自发明一套 CSS 变量主色
