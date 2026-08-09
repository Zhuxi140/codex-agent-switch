# 前端风格指南（Frontend Style Guide）

本文档提取自 cc-switch 前端源码（`src/`、`tailwind.config.cjs`、`src/index.css`），
作为新增/修改 UI 时的风格参考。

## 技术栈

| 类别 | 选型 |
| --- | --- |
| 框架 | React 18 + TypeScript + Vite 7 |
| 样式 | Tailwind CSS 3.4 + shadcn/ui（default 风格、neutral 基色、CSS 变量） |
| UI 原语 | Radix UI（dialog / select / popover / switch / tabs / tooltip 等） |
| 图标 | lucide-react |
| 动效 | framer-motion（页面/视图过渡） |
| 表单 | react-hook-form + zod |
| 提示 | sonner（toast） |
| 拖拽 | @dnd-kit（sortable 排序） |
| 图表 | recharts |
| 编辑 | CodeMirror（JSON/Markdown 配置编辑） |
| 国际化 | i18next / react-i18next，中文为主 + 英文 key |

工具函数：`cn()`（clsx + tailwind-merge，`@/lib/utils`）。

## 设计令牌（tailwind.config.cjs）

### 颜色

- 语义色（background / card / primary / muted / border 等）全部走 CSS 变量
  `hsl(var(--xxx))`，由 `src/index.css` 中 `:root` 与 `.dark` 两套令牌定义，
  暗色模式通过 `.dark` 类切换（`darkMode: ["selector", ".dark"]`）。
- 品牌主色为蓝色系（Apple 风格）：
  - `blue-400 #409CFF` / `blue-500 #0A84FF` / `blue-600 #0060DF`
- 半色调语义灰（混入 iOS 系统灰）：
  - `gray-600 #636366`（iOS systemGray），`gray-700 #48484A`，`gray-800 #3A3A3C`，
    `gray-900 #2C2C2E`，`gray-950 #1C1C1E`
- 状态色：`green-500 #10b981`（当前/健康）、`red-500 #ef4444`（危险）、
  `amber-500 #f59e0b`（警告/提示）。
- 特殊用途色：emerald（故障转移/代理健康，如 `border-emerald-500/60`）、
  sky（"需要路由"等状态标签）、violet/indigo（OMO / Slim 标签）、slate（只读标签）。

### 圆角

比默认更圆润一档：

| token | 值 |
| --- | --- |
| `rounded-sm` | 0.375rem |
| `rounded-md` | 0.5rem |
| `rounded-lg` | 0.75rem |
| `rounded-xl` | 0.875rem |

业务卡片/面板统一用 `rounded-xl`；表单控件用 `rounded-md`。

### 字体

- `font-sans`：系统字体栈（-apple-system / BlinkMacSystemFont / Segoe UI / Roboto...）
- `font-mono`：ui-monospace / SF Mono / Consolas / Liberation Mono / Menlo
- 正文默认 `text-sm`（`body { @apply text-sm }`），行高 1.5，`-webkit-font-smoothing: antialiased`

### 动画

- `fade-in`（0.5s ease-out）、`slide-up` / `slide-down` / `slide-in-right`（0.3s）、
  `pulse-slow`（3s）、`accordion-down/up`（0.2s）
- 交互态一致使用 `transition-all duration-300`（拖拽 `duration-200`）

## 全局样式约定（src/index.css）

- **玻璃拟态是品牌特征**（对话框、统计面板、顶栏之外的面板大量使用）：
  - `.glass`：`rgba(255,255,255,.7)` + `backdrop-filter: blur(10px)` 或暗色下的
    `rgba(255,255,255,.05)` 底叠 1px 白色边框
  - `.glass-card`：blur(20px)，暗色下为 `145deg` 渐变
    `rgba(255,255,255,.05)→.01` + `0 8px 32px` 阴影
  - `.glass-card-active`：蓝色边框高亮（light `rgba(59,130,246,.08)` 底 + `.4` 边框；
    暗色 `.12` 底 + `.3` 边框），用于选中态
  - 用法示例：`className="glass-card rounded-xl overflow-hidden"`
- **滚动条隐藏**：所有滚动条不显示（`scrollbar-width: none` +
  `::-webkit-scrollbar { display: none }`），`overscroll-behavior: none`
- **焦点态**：`*:focus-visible` 统一 `outline-2 outline-blue-500 outline-offset-2`
- **边框工具类**：
  - `border-default`（1px `hsl(var(--border))`）
  - `border-active`（2px，配合 `border-border-active` 主色，用于拖拽/选中强调）
  - `border-border-hover`（主色 40%）、`border-border-dragging`（主色 60%）
- **窗口行为**：`[data-tauri-drag-region]` 支持桌面窗口拖拽；
  `status-heartbeat` 在窗口失活时把状态指示淡化为 `opacity 0.5`
- 容器查询：Usage 日期范围选择器用 `container-type: inline-size` + `@container`
  按弹层自身宽度切换单/双列布局
- 组件层级：`DialogOverlay` 分层 z-index（`base: z-40 / nested: z-50 / alert: z-[60] / top: z-[110]`）

## 常用组件风格

### Button（cva，`src/components/ui/button.tsx`）

基类：`inline-flex items-center justify-center gap-2 rounded-lg text-sm font-medium transition-colors`，
禁用时 `disabled:opacity-50`。

| variant | 样式 |
| --- | --- |
| `default` | 蓝底白字：`bg-blue-500 hover:bg-blue-600`（暗色 `bg-blue-600 hover:bg-blue-700`） |
| `destructive` | 红底白字 |
| `outline` | 白底灰字 + 灰边，hover 加深、边框转主色 40% |
| `secondary` / `ghost` | 灰字灰底，hover 变深 |
| `mcp` | 祖母绿：`bg-emerald-500 hover:bg-emerald-600` |
| `link` | 蓝文字下划线 |

尺寸：`default: h-9 px-4`、`sm: h-8 rounded-md px-3 text-xs`、
`lg: h-10 rounded-md px-8`、`icon: h-9 w-9 p-1.5`。

### Input / Textarea

`h-9 rounded-md border border-border bg-background px-3 text-sm shadow-sm`，
focus 态 `ring-blue-500/20`；强制 `autoComplete off / spellCheck false`。

### Card（`src/components/ui/card.tsx`）

基类 `rounded-lg border bg-card text-card-foreground shadow-sm`；
业务面板通常覆写为 `glass-card rounded-xl`（见 ProviderCard / Usage 面板）。

### Dialog

- `DialogContent`：`max-w-lg max-h-[90vh] rounded-lg` + 蓝色边框，
  入场动画 `zoom-in-95` + `fade-in-0` + 顶部滑入
- **点击遮罩不关闭**（`onInteractOutside` 被 preventDefault）
- Header/Footer：`border-b border-border / border-t` + `bg-muted/20 px-6 py-5`，footer 按钮右对齐
- `zIndex` 参数：`base/nested/alert/top`，嵌套弹层必须提升层级

### 其他常用组件

- **Status 小徽章**（半透明圆角）：`text-[10px] font-semibold rounded-md px-1.5 py-0.5`
  - 路由：`bg-sky-100 text-sky-700 dark:bg-sky-900/40 dark:text-sky-300`
  - 只读/Hermes Managed：`bg-slate-200 text-slate-700 dark:bg-slate-700/60`
  - OMO：`bg-violet-100 text-violet-700 dark:bg-violet-900/40`
  - Slim：`bg-indigo-100 text-indigo-700 dark:bg-indigo-900/40`
- Badge（shadcn Badge）：`rounded-full text-xs font-semibold`
- ToggleRow：`rounded-xl border border-border bg-card/50 p-4` + 图标容器
  `h-8 w-8 rounded-lg bg-background ring-1 ring-border`

## 布局模式

- 顶栏：`fixed inset-x-0 top-0 z-50 h-16 bg-background/80 backdrop-blur-md px-6`，
  标题 `text-lg font-semibold`
- 面板容器：`space-y-*` 纵向排列；统计块 `rounded-xl glass-card overflow-hidden`
- ProviderCard（列表主卡）：
  - 基础：`rounded-xl border border-border p-4 bg-card`（`hover:border-border-active`）
  - 激活态：蓝色/绿色 60% 边框 + `shadow`，顶部同色渐变光（`from-blue-500/10 to-transparent`）
  - 拖拽中：`cursor-grabbing border-primary shadow-lg scale-105`
  - 操作按钮组：hover 时淡入（`opacity-0 group-hover:opacity-100`）
- 响应式：Tailwind 断点到 md，弹层内依赖容器查询
- 圆角、间距统一使用 Tailwind 工具类，不引入内联样式（`style=` 仅用于平台特定行为如拖拽区）

## 交互与反馈

- 覆盖全站的 hover / focus / active 三态齐全；拖拽元素 `cursor-grab active:cursor-grabbing`
- 操作反馈：sonner toast（成功/错误通过 `toast.success/error`）
- 异步状态：按钮加载时显示 `Loader2` 旋转
- 长列表/卡片视觉统一：玻璃拟态 + 渐变高光 + 拖拽排序

## 暗色模式

- 通过 `<html class="dark">` 切换（`darkMode: ["selector", ".dark"]`）
- 所有颜色遵守两套 token；玻璃拟态在暗色下自动降透明度并加阴影
- 新增 UI 时务必同时验证 light / dark 两种观感

## 排版布局

### 整体骨架

- **单窗口全屏布局**：根容器 `h-screen flex flex-col`，顶部固定头 + 滚动内容区
- **顶栏** 64px：`fixed bg-background/80 backdrop-blur-md px-6`，三段式 flex：
  - 左：品牌 Logo（`text-xl font-semibold` 蓝/绿）或子页标题 + 返回按钮
  - 中：`flex-1 min-w-0 justify-end` 的 AppSwitcher（空间不足自动收纳）
  - 右：`shrink-0` 的操作区（代理开关、ProfileSwitcher、+ 添加按钮），固定不被挤出
- **内容区**：`main flex-1 min-h-0 overflow-y-auto`，水平留白统一 `px-6`

### 视图/页面层

- 视图切换用 framer-motion `AnimatePresence` 淡入（0.15s），设置页块级 `opacity+y:10` 上浮 0.3s
- **供应商主页**：单列卡片列表 `space-y-3`（拖拽排序），卡片 = `rounded-xl border p-4 bg-card`，
  激活态蓝/绿边框 + 渐变光条；hover 淡入操作按钮
- **设置页**：顶部玻璃底 6 格 Tabs（`grid-cols-6 rounded-lg`），下方 `space-y-4/6` 分块堆叠
- **统计页（UsageDashboard）**：`space-y-8` 大间距；页头为"大标题 + 说明 + 右上角筛选控件"三段
- **双栏模式**（会话/详情等）：`flex h-full min-w-0` + 右侧 `w-64 border-l` 目录栏（`xl:` 才显示）

### 排版规则

- **字号阶梯**（正文 14px 起）：
  - `text-sm` 正文/说明（muted-foreground）、`text-xs` 辅助标签、`text-[10px]` 卡内徽章
  - `text-lg font-semibold` 子页标题/弹层标题；`text-xl font-semibold` 品牌名；
    `text-2xl font-bold tracking-tight` 页面大标题；统计大数字
    `text-2xl md:text-3xl font-bold tabular-nums`（数字用等宽数字）
  - `font-mono` 专属显示代码/路径/密钥/请求 ID
- 次级文本统一 `text-sm/text-xs text-muted-foreground`；栏目标题惯用
  `text-xs font-medium text-muted-foreground uppercase tracking-wide`
- 长文本统一 `truncate` + `min-w-0` 约束（URL、文件名、日志等）

### 间距与栅格

- 水平留白：主内容 `px-6`，卡片 `p-4`，表单面板 `p-6`
- 纵向块距：`space-y-3`（卡片列表）/ `space-y-4`（设置项）/ `space-y-6`（表单）/ `space-y-8`（统计页）
- 图标按钮等小元素用 `gap-1.5/gap-2`；flex 布局大量 `flex-1 min-w-0` 防溢出 + `min-h-0` 滚动约束