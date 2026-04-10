# Docker + Mock Upstream 压测设计

日期: 2026-04-10
状态: 已批准

## 目标

为 `rust-sync-proxy` 补一套可重复执行的 Docker 压测方案，用于观察：

- 峰值 RSS
- 突刺后 RSS 回落曲线
- `spill_count`
- `spill_bytes_total`
- 请求成功率与延迟分布

本轮压测边界固定为：

- 目标进程运行在 Docker 容器内
- 请求侧使用真实图片 URL
- 上游使用本地 mock
- `output=url`
- 单次请求体包含 3 张约 7MB 图片
- mock 上游每次只返回 1 张约 20MB 的 base64 图片
- 下游上传走真实图床链路

## 非目标

本次不做以下事项：

- 不接真实上游
- 不扩成多档 `MALLOC_CONF` 自动对比套件
- 不把 allocator 指标做进 admin UI 页面
- 不补完整压测报表系统
- 不改现有业务行为语义

## 核心方案

### 方案选择

本次采用“方案 B”：

1. 新增 Docker 压测脚本
2. 在 `BlobRuntime` 增加最小运行时计数
3. 把计数接到 `/admin/api/stats`

最终不选纯脚本方案，是因为只有外部 RSS 和时延还不足以解释：

- 内存回落是否主要来自 `jemalloc`
- 是否大量对象被 spill 到磁盘
- `spill` 与回落曲线之间是否同步

## 链路拓扑

```text
benchmark script
  ├── 启动本地 mock upstream
  ├── 启动 rust-sync-proxy Docker 容器
  │     ├── UPSTREAM_BASE_URL -> host mock upstream
  │     └── IMAGE_HOST_MODE / 图床配置 -> 真实图床
  ├── 并发发送 output=url 请求
  │     └── 请求体含 3 个真实图片 URL
  ├── 周期采样容器内 /proc/1/status 的 VmRSS
  └── 周期拉取 /admin/api/stats
```

### 请求侧

基准请求由脚本动态生成：

- `contents[0].parts` 中包含 3 个 `inlineData.data=http(s)://...`
- URL 来自用户提供的真实大图地址
- 每次请求结构固定，保证可重复比较

### 响应侧

mock upstream 固定返回：

- 1 个 `inlineData`
- `mimeType = image/png`
- `data` 为约 20MB 的 base64 字符串

这样可以稳定压到：

- 响应侧 base64 解码
- `BlobRuntime` inline/spill 决策
- `output=url` 上传链路

## 最小运行时计数

本次只新增两个计数：

- `spill_count`
- `spill_bytes_total`

定义：

- 每个最终落到 `Spilled(path)` 的 blob 计一次 `spill_count`
- 同时把该 blob 的最终落盘字节数累加到 `spill_bytes_total`

覆盖路径：

- `store_bytes`
- `store_stream`

这两个值由 `BlobRuntime` 统一维护，不散落到业务层。

## 管理接口输出

`/admin/api/stats` 新增两个 camelCase 字段：

- `spillCount`
- `spillBytesTotal`

admin 页面前端不要求本轮同步展示，只保证 API 输出稳定，便于脚本采样。

## 压测脚本输出

脚本完成后输出以下产物：

- `summary.json`
  - 总请求数
  - 成功数 / 失败数
  - P50 / P95 / P99
  - 峰值 RSS
  - 最终 `spill_count`
  - 最终 `spill_bytes_total`
- `rss-samples.csv`
  - 时间戳
  - `VmRSS`
- `stats-samples.csv`
  - 时间戳
  - `totalRequests`
  - `errorRequests`
  - `cacheHits`
  - `spillCount`
  - `spillBytesTotal`
- `requests.csv`
  - 单请求耗时
  - 状态码
  - 错误信息

## 配置输入

脚本至少要求输入：

- Docker 镜像名
- 3 个真实图片 URL
- 并发数
- 总请求数或持续时间
- 输出目录

脚本默认透传当前 shell 中的图床相关环境变量到容器。

## 验证标准

### 代码验证

- `spill_count` 与 `spill_bytes_total` 单测通过
- `/admin/api/stats` 新字段测试通过

### 工具验证

- benchmark 脚本帮助信息可执行
- benchmark 辅助函数单测通过

### 运行验证

- 能启动 mock upstream
- 能启动 Docker 容器
- 能成功发送至少一轮请求
- 能采样 RSS 和 admin stats

## 下一步

这轮完成后，就能稳定比较：

- `MALLOC_CONF=500ms`
- `MALLOC_CONF=100ms`

并把差异拆成：

- RSS 回落
- spill 行为
- 延迟代价
