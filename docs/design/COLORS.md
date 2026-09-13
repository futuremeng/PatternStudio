# PatternStudio 颜色规范

本项目的 UI 配色基于**武汉大学品牌色板**，所有颜色通过 CSS 变量（`web/src/styles.css` 中的 `:root`）统一管理，并支持浅色 / 深色两套主题。

## 1. 品牌色板

| 名称 | 色值 | 用途 |
| --- | --- | --- |
| 珞珈蓝 | `#002554` | 主色（浅色主题 `--accent`），品牌标识、主要操作 |
| 珞珈绿 | `#115740` | 次主色（`--accent2`）、成功状态（`--success`） |
| 珞樱粉 | `#f8a3bc` | 辅助色 `--pink` |
| 东湖蓝 | `#41b6e6` | 辅助色 `--blue`；深色主题主色（`--accent`） |
| 秋桂黄 | `#ffa300` | 辅助色 `--yellow`、警告（`--warning`） |
| 春藤紫 | `#33058d` | 辅助色 `--purple` |
| 赏瓦绿 | `#00797c` | 辅助色 `--teal` |
| 霜叶红 | `#e10800` | 辅助色 `--red`、错误（`--error`，浅色主题） |
| 晨雾灰 | `#c1c6c8` | 辅助色 `--gray` |

## 2. 语义化变量（浅色主题）

| 变量 | 色值 | 说明 |
| --- | --- | --- |
| `--bg` | `#f7f4ec` | 页面背景（米白） |
| `--surface` | `#ffffff` | 卡片 / 面板 / 顶栏背景 |
| `--surface2` | `#f1ede1` | 次级表面：按钮、输入框背景 |
| `--border` | `#e0dbcc` | 边框、分隔线 |
| `--text` | `#14213d` | 主文字 |
| `--text2` | `#6f7683` | 次要文字 |
| `--accent` | `#002554` | 主色（珞珈蓝） |
| `--accent-hover` | `#003a7c` | 主色 hover |
| `--accent2` | `#115740` | 次主色（珞珈绿） |
| `--success` | `#115740` | 成功 |
| `--error` | `#e10800` | 错误（霜叶红） |
| `--warning` | `#ffa300` | 警告（秋桂黄） |
| `--on-accent` | `#ffffff` | 主色底上的文字 |
| `--badge-done-bg` | `#e2efe9` | “已完成”徽章底色 |
| `--badge-error-bg` | `#fde7e7` | “失败”徽章底色 |
| `--badge-processing-bg` | `#fff3dc` | “处理中”徽章底色 |
| `--badge-processing-text` | `#b26a00` | “处理中”徽章文字 |
| `--overlay-panel-bg` | `rgba(255,255,255,0.92)` | 悬浮面板背景 |
| `--selected-bg` | `rgba(0,37,84,0.08)` | 选中项高亮 |
| `--profile-badge-bg` | `rgba(17,87,64,0.1)` | 配置徽章底色 |
| `--profile-badge-border` | `rgba(17,87,64,0.35)` | 配置徽章边框 |
| `--save-hover-bg` | `rgba(17,87,64,0.08)` | 保存按钮 hover 底色 |
| `--complete-hover` | `#0d4634` | 完成按钮 hover |
| `--radius` | `8px` | 通用圆角（卡片等） |

## 3. 语义化变量（深色主题）

深色主题在 `:root[data-theme="dark"]` 及 `@media (prefers-color-scheme: dark)` 下覆盖同名变量：

| 变量 | 色值 | 说明 |
| --- | --- | --- |
| `--bg` | `#0f1219` | 页面背景（近黑蓝） |
| `--surface` | `#1a1f2b` | 面板背景 |
| `--surface2` | `#232a3a` | 次级表面 |
| `--border` | `#2e3648` | 边框 |
| `--text` | `#e4e7ee` | 主文字 |
| `--text2` | `#8b92a5` | 次要文字 |
| `--accent` | `#41b6e6` | 主色（东湖蓝，提亮保证对比度） |
| `--accent-hover` | `#6cc6ef` | 主色 hover |
| `--accent2` | `#45a583` | 次主色（提亮珞珈绿） |
| `--success` | `#45a583` | 成功 |
| `--error` | `#ff6b6b` | 错误（提亮霜叶红） |
| `--warning` | `#ffa300` | 警告（不变） |
| `--on-accent` | `#0b1524` | 主色底上的文字（深色） |
| `--badge-done-bg` | `#12352a` | “已完成”徽章底色 |
| `--badge-error-bg` | `#3a1a1a` | “失败”徽章底色 |
| `--badge-processing-bg` | `#3a3218` | “处理中”徽章底色 |
| `--badge-processing-text` | `#ffa300` | “处理中”徽章文字 |
| `--overlay-panel-bg` | `rgba(26,29,39,0.92)` | 悬浮面板背景 |
| `--selected-bg` | `rgba(65,182,230,0.12)` | 选中项高亮 |
| `--profile-badge-bg` | `rgba(69,165,131,0.15)` | 配置徽章底色 |
| `--profile-badge-border` | `rgba(69,165,131,0.4)` | 配置徽章边框 |
| `--save-hover-bg` | `rgba(69,165,131,0.12)` | 保存按钮 hover 底色 |
| `--complete-hover` | `#157a55` | 完成按钮 hover |

## 4. 使用规则

- **一律使用语义变量**（`var(--accent)` 等），不要在组件里写死色值，以保证深浅主题自动切换。
- **主要操作**（保存、打开、确认）：`--accent` 底 + `--on-accent` 文字，hover 用 `--accent-hover`。
- **成功 / 完成**：`--success`（珞珈绿系）；**错误 / 危险**：`--error`；**警告 / 处理中**：`--warning`。
- **中性控件**（按钮、下拉框）：`--surface2` 底 + `--border` 边框 + `--text` 文字，hover 时边框变为 `--accent`。
- **文字层级**：主文字 `--text`，次要信息 `--text2`。

## 5. 顶栏（app-header）控件标准

顶栏内所有按钮 / 下拉框统一尺寸，保证各页面观感一致：

- 高度 **32px**，圆角 **6px**，边框 **1px `--border`**，背景 **`--surface2`**，字号 **0.85rem**；
- 图标按钮（`.theme-toggle`、`.icon-btn`）为 32×32px 方形；
- 返回按钮（`.back-btn`）为幽灵样式（无边框、`--text2` 文字），但同样 32px 高；
- 主按钮（`.open-project-btn`）保留 `--accent` 填充，去掉边框，其余尺寸一致。
