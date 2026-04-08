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

## 常用环境变量

### 核心

- `PORT`
  默认 `8787`
- `UPSTREAM_BASE_URL`
  默认 `https://magic666.top`
- `UPSTREAM_API_KEY`
  必填；为空时请求返回 `401`

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
  用于把 legacy 图床 URL 包装为 `${PUBLIC_BASE_URL}/proxy/image?url=...`
- `SLOW_LOG_THRESHOLD_MS`
  默认 `100000`；`0` 表示关闭慢请求日志
- `PROXY_SPECIAL_UPSTREAM_URLS`
  默认开启；影响 Markdown 图片结果是否包装为 `/proxy/image?u=...`
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
- `PROXY_STANDARD_OUTPUT_URLS`
  默认开启；仅影响 legacy 图床结果是否继续包装 `/proxy/image`
- `UPLOAD_TIMEOUT_MS`
  默认 `10000`
- `UPLOAD_TLS_HANDSHAKE_TIMEOUT_MS`
  默认 `10000`
- `UPLOAD_INSECURE_SKIP_VERIFY`
  默认关闭；仅用于对齐 Go 原版 TLS 配置

R2 模式还需要：

- `R2_ENDPOINT`
- `R2_BUCKET`
- `R2_ACCESS_KEY_ID`
- `R2_SECRET_ACCESS_KEY`
- `R2_PUBLIC_BASE_URL`
- `R2_OBJECT_PREFIX`
  默认 `images`

行为规则：

- `legacy` 成功后，若 `PROXY_STANDARD_OUTPUT_URLS=1` 且 `PUBLIC_BASE_URL` 非空，则返回 `/proxy/image?url=...`
- `r2` 成功后直出 `R2_PUBLIC_BASE_URL/<objectKey>`
- `r2_then_legacy` 先尝试 R2，失败后回退 legacy
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
export PUBLIC_BASE_URL="https://proxy.example.com"
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

## 验证

跑 Rust 测试：

```bash
~/.cargo/bin/cargo test --tests -- --nocapture
```

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

## Docker

构建镜像：

```bash
docker build -t rust-sync-proxy:local .
```

运行容器：

```bash
docker run --rm -p 8787:8787 \
  -e UPSTREAM_BASE_URL="https://magic666.top" \
  -e UPSTREAM_API_KEY="your-upstream-key" \
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
