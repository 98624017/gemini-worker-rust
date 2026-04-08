# Admin UI 全面升级设计

日期: 2026-04-08
状态: 已批准

## 目标

在零服务端额外开销的前提下，全面提升 admin 后台管理界面的视觉质感、数据可视化能力和交互体验。

## 约束

- 仍为单个 `const` HTML 字符串内嵌在 `src/admin.rs`
- Chart.js minimal 内联（~50KB 源码），总 HTML 常量约 ~90KB
- 不新增 API 端点，不增加服务端计算
- 不引入外部字体、CDN 或前端构建工具
- 所有图表数据通过客户端 JS 聚合现有 `/admin/api/logs` 和 `/admin/api/stats` 响应

## 方案选择

选定**方案 A：原地增强**——在现有单文件 HTML 常量上直接改造，保持"零外部依赖单二进制"项目哲学。

## 设计详情

### 1. CSS 变量体系与双主题

所有硬编码颜色抽为 CSS 变量，通过 `<html data-theme="dark|light">` 属性切换。

**暗色主题变量**：
- `--bg-primary: #080e1f`
- `--bg-card: rgba(255,255,255,0.03)`
- `--bg-input: rgba(255,255,255,0.05)`
- `--border: rgba(255,255,255,0.07)`
- `--text-primary: #e2e8f0`
- `--text-secondary: rgba(226,232,240,0.6)`
- `--text-muted: rgba(226,232,240,0.35)`
- accent 系列保持现有值

**亮色主题变量**：
- `--bg-primary: #f8fafc`
- `--bg-card: #ffffff`
- `--bg-input: #f1f5f9`
- `--border: #e2e8f0`
- `--text-primary: #1e293b`
- `--text-secondary: #64748b`
- `--text-muted: #94a3b8`
- accent 系列同色系、饱和度/亮度微调

**切换逻辑**：
- 初始化读取 `localStorage.getItem('theme')`
- 为空则跟随 `prefers-color-scheme`
- header 加 sun/moon SVG 图标按钮
- 切换时设置 `data-theme` + 存 localStorage
- 全局 `transition: background 0.3s, color 0.3s, border-color 0.3s`

### 2. 图表区域

在 stats 卡片和 toolbar 之间插入 2x2 图表网格。

| 图表 | 类型 | 数据源 | 聚合方式 |
|------|------|--------|---------|
| 请求耗时分布 | 柱状图 | `items[].durationMs` | 分桶：0-500ms / 500-1s / 1-3s / 3-5s / 5-10s / 10s+ |
| 模型使用占比 | 环形图 | `items[].path` | 正则提取 model 名，计数 |
| 状态码分布 | 环形图 | `items[].statusCode` | 按 2xx/4xx/5xx 分组计数 |
| 请求时间线 | 折线图 | `items[].createdAt` + `durationMs` | 按时间排序 |

**Chart.js 配置**：
- 图表容器固定高度 200px，响应式宽度
- 读取 CSS 变量设置字体/网格线颜色
- 主题切换时 `chart.update()` 刷新配色
- 每次 refresh 销毁旧实例再创建新的
- `animation: { duration: 400 }`

### 3. 交互增强

**日志展开/折叠动画**：
- `max-height: 0 -> 600px` + `opacity: 0 -> 1`
- `transition: max-height 0.25s ease-out, opacity 0.2s ease-out`
- 展开条目左侧 border 高亮

**键盘导航**：
- `j/↓` 下一条，`k/↑` 上一条
- `Enter/Space` 展开/折叠
- `Esc` 全部折叠
- `/` 聚焦搜索框
- `r` 手动刷新
- 搜索框获焦时禁用快捷键

**微交互**：
- Stats 数值变化时 requestAnimationFrame 数字滚动（300ms）
- 刷新按钮点击旋转 360（0.6s）
- 自动刷新开启时 header 呼吸脉动绿点
- 日志行 hover 背景渐变 + translateX(2px)
- 空状态 SVG 插图

**Header 改进**：
- 左侧 banana SVG icon
- 右侧主题切换 sun/moon 按钮带旋转过渡

### 4. 视觉美化

**卡片**：
- 暗色：`backdrop-filter: blur(8px)` 玻璃拟态
- 亮色：`box-shadow: 0 1px 3px rgba(0,0,0,0.08)`
- 图表卡片与 stats 卡片同风格

**Tag 标签**：
- 新增 `tag-cache`（青色 #06b6d4）
- 新增 `tag-slow`（>5s 自动标记，橙色）
- `backdrop-filter: blur(4px)` 增加层次

**字体**：
- 等宽：`"JetBrains Mono", "Fira Code", ui-monospace, "SF Mono", Menlo, monospace`
- 无衬线：保持现有系统字体栈
- 不加载外部字体

**响应式断点**：
- `>1200px`: Stats 5列, Charts 2列, Detail 2列
- `900-1200px`: Stats 3列, Charts 2列, Detail 2列
- `480-900px`: Stats 2列, Charts 1列, Detail 1列
- `<480px`: 全部 1列

**滚动条**：
- webkit 自定义：6px 宽，透明轨道，border-radius thumb
- 暗/亮主题各自适配
