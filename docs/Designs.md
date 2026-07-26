# Zene UI Design

**Console UI 完整规范**（根目录 [`DESIGN.md`](../DESIGN.md) 为入口）。对照稿：[`docs/design/`](./design/)（基础语言 / 组件库 / 审查与反馈）。

## Surface：Console

Cloud Console（`cloud/apps/web`）与 Auth 页。浅色文档式 chrome；[`docs/design/`](./design/) 深色 IDE 样板仅作对照，**产品默认仍为浅色 Console**。

- **主操作色**：`#0090FF`（动作 / 焦点；非大面积壳色）
- **主文字 / 品牌 mark / 用户气泡**：`#1C2024`（Ink；不是第二主题色）
- **交互底**：hover `#F9F9FB`；选中 `#E6F4FE`
- **字体**：Inter；代码 / 路径 JetBrains Mono（或 Menlo 回退）
- **密度**：侧栏 272px；顶栏 48px；控件高约 28–32px
- **圆角**：控件 4px；面板 8px
- **壳层**：横向侧栏 + 主区；Run 保持对话左、代码/变更右

## 按钮

- **主操作**：实心 `#0090FF` 白字；hover 略加深（`#0588F0`）；disabled `opacity: .4`（表单提交、Auth 主按钮等明确 CTA）
- **次操作**：白底 + `border #DFE3E8`；默认 hover `#F9F9FB`
- **侧栏 New Agent**：无描边；默认 Ink 字；悬停才体现主题色（底 `#E6F4FE` + 字 Primary），默认不铺实心蓝
- **危险**：软底 `#FEE8E7` + 可读错误文案
- **图标按钮**：约 28–32px；hover 浅表面

## Toast / 反馈

- 短 toast；成功 / 警告 / 错误用软底色 + 文字，勿整页 banner
- 状态必须同时提供文字与位置关系，不以颜色为唯一信号

## 表单

- 标签：小写或 sentence case、次要色 `#60646C`
- 输入：白底、`border #DFE3E8`、`rounded` 4–8px；focus 用 `#0090FF` 轮廓或 `#E6F4FE` ring
- 密钥：`font-mono`

## 弹窗

| 档位 | 宽度 | 用途 |
|------|------|------|
| sm | ~384px | 确认 |
| md | 480px | 设置 |
| lg | 560–800px | 多分区 |

Backdrop `rgba(0,0,0,.5)`，无 blur。Header / Body / Footer 分区。

## 侧栏

- 宽 **272px**，浅表面底 `#F9F9FB`，右边线 `#E6E8EB`；**少用**顶/底横线，用间距分区
- 品牌 mark：Ink 方块 + 白字 `Z`（与 Auth 一致）
- New Agent：无描边导航行 + Plus 图标；当前页白底轻阴影；悬停浅白底
- Agents 列表：行间距 4–8px、圆角浅底；分组标题下条目略缩进；选中白底，hover 浅白
- 底部账户：无顶部分割线，与列表用留白隔开
- 列表行高约 28–32px

## 主区

- 顶栏：白底、底边线；标题 + 状态
- New Agent：居中 composer（约 720px）
- Run：左对话流 + 右 CodePanel（Files / Diff / Review / Commits）
- Settings：居中卡片栈，1px 边框，轻阴影可选

## 对话

- 用户与助手均左对齐（非聊天气泡右贴）
- 用户：浅底 `#F9F9FB` + 细边 `#E6E8EB`、大圆角；Ink 字
- 助手：无气泡，直接落在画布；清晰层级
- Run Banner：上下无横线，靠留白分区
- Composer：浅底圆角输入面；上下无硬分割线；focus 略加深边
- Tool / system：等宽、浅底

## 动画

过渡 ≤ `150ms`。禁止长动画与大面积 blur。

## 禁止事项

- Gold / Manrope 作为 Console 默认品牌语言（已废弃 Atelier 默认）
- 紫色营销渐变、霓虹描边、玻璃拟态
- 每页自造主色
