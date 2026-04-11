# rust-sync-proxy

一个独立的 Rust 同步 Gemini 兼容代理实现。

目标：

- 可单独构建、运行、测试和容器化
- 优先对齐既有 Go 同步代理的主链路行为
- 方便直接拆分为单独公开仓库

当前状态：

- 已实现同步 `generateContent` 转发
- 已实现请求侧 `inlineData.data=http(s)://...` 拉图转 base64
- 已实现响应侧 `thoughtSignature` 移除
- 已实现多图结果只保留同一 candidate 中 payload 最大的图片
- 已实现 `output=url`
  - `legacy`
  - `r2`
  - `r2_then_legacy`
- 已实现 Go 风格 `admin` 路由与 Basic Auth
- 已实现特殊上游 Markdown 图片结果归一化
- 已实现请求侧内存缓存、磁盘缓存与后台桥接下载
- 已有 Rust 集成测试
- 已有可选的 Go/Rust 对照脚本

## 路由

- `POST /v1beta/models/{model}:generateContent`
- `GET /admin`
- `GET /admin/logs`
- `GET /admin/api/logs`
- `GET /admin/api/stats`

只有 `:generateContent` 会被转发；其余模型路由返回 `404`。

## 快速开始

如果 `cargo` 不在 PATH，可直接使用 `~/.cargo/bin/cargo`。

先复制一份环境变量模板：

```bash
cp .env.example .env
```

```bash
export PORT=8787
export UPSTREAM_BASE_URL="https://magic666.top"
export UPSTREAM_API_KEY="your-upstream-key"

~/.cargo/bin/cargo run
```

本地构建：

```bash
~/.cargo/bin/cargo build
```

发布模式构建：

```bash
~/.cargo/bin/cargo build --release --locked
```

## 默认 allocator

- `Linux + GNU libc` 构建默认使用 `jemalloc`
- 其他平台默认回退系统 allocator
- 当前镜像默认注入：

```bash
MALLOC_CONF=background_thread:true,dirty_decay_ms:500,muzzy_decay_ms:500
```

这组参数的目的，是在保留一小段短时页复用窗口的同时，让大流量突刺后的 RSS
更积极回落。

如果你想覆盖默认值，可以在运行前自行导出：

```bash
export MALLOC_CONF="background_thread:true,dirty_decay_ms:100,muzzy_decay_ms:100"
```

## 常用环境变量

### 核心

- `PORT`
  默认 `8787`
- `UPSTREAM_BASE_URL`
  默认 `https://magic666.top`
- `UPSTREAM_API_KEY`
  必填；为空时请求返回 `401`
- `MALLOC_CONF`
  `jemalloc` 运行时参数；镜像默认
  `background_thread:true,dirty_decay_ms:500,muzzy_decay_ms:500`

### 上游覆盖

每个请求都可以通过 Header 覆盖上游：

- `x-goog-api-key: <apiKey>`
- `x-goog-api-key: <baseUrl>|<apiKey>`
- `Authorization: Bearer <apiKey>`
- `Authorization: Bearer <baseUrl>|<apiKey>`

当前 Rust 版支持单上游覆盖，不支持 Go 版的双上游 `baseUrl1|key1,baseUrl2|key2` 路由格式。

### 图片代理

- `ALLOWED_PROXY_DOMAINS`
  逗号分隔；显式设置后会覆盖默认列表
- `PUBLIC_BASE_URL`
  兼容旧版容器；若 `EXTERNAL_IMAGE_PROXY_PREFIX` 为空，则自动回退为
  `${PUBLIC_BASE_URL}/proxy/image?url=`
- `EXTERNAL_IMAGE_PROXY_PREFIX`
  非空时优先使用；把图片 URL 包装为
  `${EXTERNAL_IMAGE_PROXY_PREFIX}<escaped-url>`
- `PROXY_STANDARD_OUTPUT_URLS`
  默认开启；控制标准链路里 `legacy` / `r2_then_legacy` 回退到 `legacy`
  时是否包装代理前缀
- `SLOW_LOG_THRESHOLD_MS`
  默认 `100000`；`0` 表示关闭慢请求日志
- `PROXY_SPECIAL_UPSTREAM_URLS`
  默认开启；影响 Markdown / `aiapidev` 特殊上游结果是否包装代理前缀
- `ENABLE_IMAGE_COMPRESSION`
  默认关闭；开启后，响应侧 PNG 图片超过 `15MiB` 时会尝试转成
  `4:4:4 / quality=97` 的 JPEG，以降低上传图床 / R2 或返回 base64 的体积
- `IMAGE_COMPRESSION_JPEG_QUALITY`
  默认 `97`；仅在 `ENABLE_IMAGE_COMPRESSION=true` 时生效，范围 `1-100`。
  数值越大，压缩越轻，生成的 JPEG 一般越大；想把 `4.xMB` 往 `7MB+`
  调，建议先试 `100`
- `ADMIN_PASSWORD`
  非空时启用 admin 路由并要求 Basic Auth
- `IMAGE_FETCH_TIMEOUT_MS`
  请求侧图片抓取超时；默认 `20000`
- `IMAGE_TLS_HANDSHAKE_TIMEOUT_MS`
  默认 `15000`
- `IMAGE_FETCH_INSECURE_SKIP_VERIFY`
  默认关闭；仅用于对齐 Go 原版 TLS 配置
- `IMAGE_FETCH_EXTERNAL_PROXY_DOMAINS`
  命中时改走外部代理抓图
  响应侧按 URL 拉图转 base64 时，单张图片默认最大 `35MiB`
- `INLINE_DATA_URL_CACHE_DIR`
  非空时启用请求侧磁盘缓存
- `INLINE_DATA_URL_CACHE_TTL_MS`
  默认 `3600000`
- `INLINE_DATA_URL_CACHE_MAX_BYTES`
  默认 `1073741824`
- `INLINE_DATA_URL_MEMORY_CACHE_MAX_BYTES`
  默认 `104857600`；设为 `off/0/false/disable/none` 可关闭
- `INLINE_DATA_URL_BACKGROUND_FETCH_WAIT_TIMEOUT_MS`
  默认跟随 `IMAGE_FETCH_TIMEOUT_MS`
- `INLINE_DATA_URL_BACKGROUND_FETCH_TOTAL_TIMEOUT_MS`
  默认 `90000`
- `INLINE_DATA_URL_BACKGROUND_FETCH_MAX_INFLIGHT`
  默认 `128`

默认 allowlist：

- `ai.kefan.cn`
- `uguu.se`
- `.uguu.se`
- `.aitohumanize.com`

### `output=url`

- `IMAGE_HOST_MODE`
  - `legacy`
  - `r2`
  - `r2_then_legacy`
- `UPLOAD_TIMEOUT_MS`
  默认 `20000`
- `UPLOAD_TLS_HANDSHAKE_TIMEOUT_MS`
  默认 `10000`
- `UPLOAD_INSECURE_SKIP_VERIFY`
  默认关闭；仅用于对齐 Go 原版 TLS 配置

### BlobRuntime 预算

- `INSTANCE_MEMORY_BYTES`
  用于推导 blob 默认预算；默认 `2147483648`
- `BLOB_INLINE_MAX_BYTES`
  单个 blob 保持内存直通的上限
- `BLOB_REQUEST_HOT_BUDGET_BYTES`
  单请求可占用的热内存预算
- `BLOB_GLOBAL_HOT_BUDGET_BYTES`
  进程级热内存总预算
- `BLOB_SPILL_DIR`
  spill 文件目录；默认 `/tmp/rust-sync-proxy-blobs`

按 `INSTANCE_MEMORY_BYTES` 推导出的默认值：

- `2GiB`: `inline=8MiB`，`request_hot=24MiB`，`global_hot=384MiB`
- `4GiB`: `inline=12MiB`，`request_hot=40MiB`，`global_hot=768MiB`
- `8GiB`: `inline=16MiB`，`request_hot=64MiB`，`global_hot=1536MiB`

R2 模式还需要：

- `R2_ENDPOINT`
- `R2_BUCKET`
- `R2_ACCESS_KEY_ID`
- `R2_SECRET_ACCESS_KEY`
- `R2_PUBLIC_BASE_URL`
- `R2_OBJECT_PREFIX`
  默认 `images`

行为规则：

- 代理前缀解析优先级：
  `EXTERNAL_IMAGE_PROXY_PREFIX` > `${PUBLIC_BASE_URL}/proxy/image?url=`
- 标准链路里，`legacy` 上传结果会在 `PROXY_STANDARD_OUTPUT_URLS=true` 时包装代理前缀
- `r2` 成功后真实 URL 为 `R2_PUBLIC_BASE_URL/<objectKey>`
- 标准链路里，`r2` 成功后永远直接返回
  `R2_PUBLIC_BASE_URL/<objectKey>`，不会再套一层代理
- `r2_then_legacy` 先尝试 R2，失败后回退 legacy
- `aiapidev` / Markdown 特殊上游结果受 `PROXY_SPECIAL_UPSTREAM_URLS` 控制
- 上传失败统一走 fail-open，保留原始 base64

## 示例

### base64 模式

```bash
curl -sS \
  -H 'Content-Type: application/json' \
  -d '{"contents":[{"parts":[{"text":"hello"}]}]}' \
  'http://127.0.0.1:8787/v1beta/models/demo:generateContent'
```

### `output=url` + legacy

```bash
export EXTERNAL_IMAGE_PROXY_PREFIX="https://proxy.example.com/fetch?url="
export IMAGE_HOST_MODE="legacy"

curl -sS \
  -H 'Content-Type: application/json' \
  -d '{"output":"url","contents":[{"parts":[{"text":"hello"}]}]}' \
  'http://127.0.0.1:8787/v1beta/models/demo:generateContent'
```

### `output=url` + R2

```bash
export IMAGE_HOST_MODE="r2"
export R2_ENDPOINT="https://<accountid>.r2.cloudflarestorage.com"
export R2_BUCKET="images"
export R2_ACCESS_KEY_ID="key"
export R2_SECRET_ACCESS_KEY="secret"
export R2_PUBLIC_BASE_URL="https://img.example.com"

curl -sS \
  -H 'Content-Type: application/json' \
  -d '{"output":"url","contents":[{"parts":[{"text":"hello"}]}]}' \
  'http://127.0.0.1:8787/v1beta/models/demo:generateContent'
```

### `aiapidev` 特殊兼容

- 当请求头里的上游地址是 `https://www.aiapidev.com` 或 `https://aiapidev.com` 时，代理会走专用分支：
  - `gemini-3-pro-image-preview -> nanobananapro`
  - `gemini-3.1-flash-image-preview -> nanobanana2`
  - 请求体里的图片 URL 会从 `inlineData` 改写成 `file_data.file_uri`
  - 创建任务后会同步轮询 `/v1beta/tasks/{requestId}`，直到成功、失败或总超时（当前硬编码 `450s`）
  - 轮询遇到网络错误或 `408/425/429/500/502/503/504` 会按 1 秒间隔重试；连续失败 5 次会提前返回最后一次错误
- 成功结果会统一改写回 Gemini 风格响应。
- 若 `output=url` 开启，返回 URL 风格 `inlineData.data`；否则会下载结果图并转成 base64。
- 该上游不提供真实 token 统计，但部分下游网关依赖 `usageMetadata` 触发按次计费，因此这里会返回**稳定占位值**：
  - `promptTokenCount = 1024`
  - `candidatesTokenCount = 1024`
  - `totalTokenCount = 2048`
- 这组值**不是上游真实计费数据**，只用于兼容下游按次计费触发逻辑，不能拿来做 token 对账。

## 验证

跑 Rust 测试：

```bash
~/.cargo/bin/cargo test --tests -- --nocapture
```

看 blob runtime 的基准与调参记录：

```bash
sed -n '1,240p' docs/plans/2026-04-09-generate-content-memory-redesign-benchmark-notes.md
```

跑 Docker 压测脚本：

```bash
python3 scripts/benchmark_docker_mock_upstream.py \
  --image rust-sync-proxy:jemalloc-test \
  --image-url "https://example.com/7mb-a.png" \
  --image-url "https://example.com/7mb-b.png" \
  --image-url "https://example.com/7mb-c.png" \
  --concurrency 2 \
  --total-requests 4 \
  --cooldown-seconds 30
```

这条脚本的边界是：

- Docker 里跑 `rust-sync-proxy`
- 请求侧使用 3 个真实图片 URL
- 上游使用本地 mock
- mock 上游每次返回 1 张约 `20MB` 的 base64 图片
- `output=url` 上传继续走真实图床链路

压测结果会落到 `benchmark-output/<timestamp>/`，包含：

- `summary.json`
- `rss-samples.csv`
- `stats-samples.csv`
- `requests.csv`

跑 Go/Rust 对照：

```bash
GO_IMPL_ROOT=/path/to/go-implementation \
  bash ./scripts/compare_with_go.sh
```

当前对照脚本会验证：

- 非流式 base64 输出
- 非流式 `output=url + r2`
- Markdown 图片归一化
- `admin/api/stats` 可访问且输出一致

当前 `/admin/api/stats` 还会额外输出：

- `spillCount`
- `spillBytesTotal`

## Docker

构建镜像：

```bash
docker build -t rust-sync-proxy:local .
```

一条命令跑 Docker 回归：

```bash
export AIAPIDEV_TEST_KEY="sk_xxx"
python3 scripts/docker_aiapidev_regression.py --image rust-sync-proxy:local
```

这条脚本会依次验证：

- 标准 `output=url` 容器链路
- `aiapidev + output=url`
- `aiapidev + base64`
- `aiapidev` 坏图源失败场景

运行容器：

```bash
docker run --rm -p 8787:8787 \
  -e UPSTREAM_BASE_URL="https://magic666.top" \
  -e UPSTREAM_API_KEY="your-upstream-key" \
  rust-sync-proxy:local
```

如果要覆盖默认 `jemalloc` 参数：

```bash
docker run --rm -p 8787:8787 \
  -e UPSTREAM_BASE_URL="https://magic666.top" \
  -e UPSTREAM_API_KEY="your-upstream-key" \
  -e MALLOC_CONF="background_thread:true,dirty_decay_ms:100,muzzy_decay_ms:100" \
  rust-sync-proxy:local
```

如果要跑 `output=url + r2`，继续补上：

```bash
docker run --rm -p 8787:8787 \
  -e UPSTREAM_BASE_URL="https://magic666.top" \
  -e UPSTREAM_API_KEY="your-upstream-key" \
  -e IMAGE_HOST_MODE="r2" \
  -e R2_ENDPOINT="https://<accountid>.r2.cloudflarestorage.com" \
  -e R2_BUCKET="images" \
  -e R2_ACCESS_KEY_ID="key" \
  -e R2_SECRET_ACCESS_KEY="secret" \
  -e R2_PUBLIC_BASE_URL="https://img.example.com" \
  rust-sync-proxy:local
```

## 当前实现边界

当前仍有少量细节与 Go 版存在实现差异，尤其是更细的预热、慢日志和部分网络细节。
但主链路、admin、Markdown 图片归一化和请求侧缓存/后台桥接已经进入 Rust 版。
