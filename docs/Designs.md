# Zene UI Design

**Console UI 完整规范**（根目录 [`DESIGN.md`](../DESIGN.md) 为入口）。对照稿：[`docs/design/`](./design/)（基础语言 / 组件库 / 审查与反馈）。

## Surface：Console

Cloud Console（`cloud/apps/web`）与 Auth 页。浅色文档式 chrome + 可选深色 IDE 样板仅作对照，**产品默认仍为浅色 Console**。

- **主操作色**：`#0090FF`
- **主文字**：`#1C2024`
- **字体**：Inter；代码 / 路径 JetBrains Mono（或 Menlo 回退）
- **密度**：侧栏 272px；顶栏 48px；控件高约 28–32px
- **圆角**：控件 4px；面板 8px
- **壳层**：横向侧栏 + 主区；Run 保持对话左、代码/变更右

## 按钮

- **主操作**：实心 `#0090FF` 白字；hover 略加深（`#0588F0`）；disabled `opacity: .4`
- **次操作**：白底 + `border #DFE3E8`；hover `#F9F9FB`
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

- 宽 **272px**，白底或极浅表面，右边线 `#E6E8EB`
- 顶部 New Agent；中部 Agents 列表；底部账户
- Active / hover：`#E6F4FE` 或 `#F9F9FB`
- 列表行高约 28–32px

## 主区

- 顶栏：白底、底边线；标题 + 状态
- New Agent：居中 composer（约 720px）
- Run：左对话流 + 右 CodePanel（Files / Diff / Review / Commits）
- Settings：居中卡片栈，1px 边框，轻阴影可选

## 对话

- 用户：右对齐，Ink 或 Primary 实心气泡
- 助手：左对齐，清晰层级
- Tool / system：等宽、浅底

## 动画

过渡 ≤ `150ms`。禁止长动画与大面积 blur。

## 禁止事项

- Gold / Manrope 作为 Console 默认品牌语言（已废弃 Atelier 默认）
- 紫色营销渐变、霓虹描边、玻璃拟态
- 每页自造主色
