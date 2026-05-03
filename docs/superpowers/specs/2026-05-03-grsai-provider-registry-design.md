# grsai Provider Registry 大一统设计

日期：2026-05-03
状态：已确认，待实现

## 背景

Rust 同步代理当前已经支持 Gemini `generateContent` 透明转发、OpenAI
`/v1/images/generations` 转发，以及 `aiapidev` 这类异步任务上游的特殊链路。
但渠道判断主要散落在路由处理函数里，例如根据 base URL 判断是否走
`aiapidev` 特例。

Go 版 `go-banana-proxy` 已经有更明确的上游抽象：

- `grsai`：`api.grsai.com`，同步单次请求。
- `aiapidev`：`www.aiapidev.com`，异步创建任务并轮询。

这次目标是把 Go 版 `grsai` 同步链路复刻到 Rust 项目，同时引入统一
Provider Registry，避免继续把新渠道做成路由层特例。

## 目标

1. 新增统一 Provider Registry，根据 `resolved.base_url` 选择上游渠道。
2. 支持 `grsai.com` 和 `*.grsai.com` 的同步单次请求 provider。
3. `grsai` 同时覆盖 Gemini 和 OpenAI image 两个入口。
4. 保留现有 `aiapidev` 行为，但通过 registry 入口分发。
5. 未匹配 provider 的未知上游继续走现有透明转发，避免破坏当前部署。
6. 下游错误响应继续沿用 Rust 现有格式，不引入 Go 版 `BananaError` schema。
7. 为 provider 匹配、请求构造、响应解析和回归路径补齐测试。

## 非目标

1. 不一次性重写 admin UI。
2. 不引入 Go 版完整 `BananaError` 对外结构。
3. 不移除未知上游透明转发兼容路径。
4. 不改变现有上游鉴权解析规则。
5. 不改变 `aiapidev` 已确认的轮询间隔、超时和输出归一化语义。

## 已确认决策

### 1. Provider 分发策略

采用统一 registry：

```text
resolved.base_url
  ├── grsai.com / *.grsai.com -> grsai provider
  ├── aiapidev.com / www.aiapidev.com -> aiapidev provider
  └── 其他 -> transparent provider
```

`transparent provider` 不是新上游协议，只是现有 Rust 透明转发逻辑的命名边界。

### 2. 未知上游兼容语义

未匹配任何 provider 时继续走 Rust 现有透明转发，不回退到 `grsai`。

原因：

1. 当前 Rust 项目默认上游是 `https://magic666.top`，不能因为引入 `grsai`
   改变已有部署行为。
2. 透明转发仍是通用 Gemini/OpenAI 兼容能力。
3. 严格拒绝未知上游会扩大本次迁移风险。

### 3. `grsai` 输出语义

Gemini：

- `output=url` 时，返回 Gemini 结构，`inlineData.data` 放图片 URL。
- 默认或非 `url` 输出时，由代理下载上游图片并转成 base64 后返回。

OpenAI：

- 返回 OpenAI image response 的 `data[].url`。
- URL 包装沿用现有 Rust 配置和 helper，例如
  `OPENAI_IMAGE_UPSTREAM_URL_PROXY_PREFIX`、`EXTERNAL_IMAGE_PROXY_PREFIX`。

### 4. `grsai` 错误响应

对外错误保持 Rust 现有格式：

- 用户侧看见中文可读 message 和合理 HTTP status。
- 不新增 Go 版 `provider/error_source/request_stage/retryable` 等稳定 schema。
- 上游状态、body、解析细节尽量进入 admin 日志和内部 detail。

## 推荐方案

采用“Provider Registry + 执行策略”的保守迁移。

设计上把渠道统一为 provider，但实现时不强行重写所有现有函数：

1. 新增 provider 匹配和能力边界。
2. `grsai` 新增完整同步 provider。
3. `aiapidev` 先把现有异步函数挂到 registry 分发入口下。
4. 未知上游走 transparent provider，复用现有标准转发。

这个方案比单纯在 `router.rs` 加 `grsai` 分支更适合后续扩展；也比一次性大重构更稳。

## 组件设计

### `ProviderRegistry`

职责：

1. 保存 provider 列表。
2. 按 base URL 解析 host 并选择 provider。
3. 未匹配时返回 transparent provider。

匹配规则必须避免后缀误判：

- `https://api.grsai.com` 命中 `grsai`。
- `https://grsai.com` 命中 `grsai`。
- `https://sub.api.grsai.com` 命中 `grsai`。
- `https://evilgrsai.com` 不命中。
- `https://grsai.com.evil.com` 不命中。

### Provider 能力边界

Provider 负责渠道差异：

1. `name`
2. `match_base_url`
3. Gemini 模型映射
4. OpenAI 模型映射
5. 上游路径
6. 上游请求体构造
7. 上游响应解析
8. 是否需要异步任务执行

路由层继续负责通用事项：

1. 读请求体。
2. 上游鉴权解析。
3. 请求图片物化、缓存、编码。
4. admin 日志。
5. 下游错误响应格式。
6. 图片下载、上传或 URL 包装。

### `grsai` provider

同步单次请求 provider。

匹配域名：

```text
grsai.com
*.grsai.com
```

上游路径：

```text
/v1/draw/nano-banana
```

Gemini 模型映射：

| Gemini 模型名 | grsai 模型名 |
| --- | --- |
| `gemini-3-pro-image-preview` | `nano-banana-pro` |
| `gemini-2.5-flash-image` | `nano-banana-fast` |
| `gemini-3.1-flash-image-preview` | `nano-banana-2` |
| 空模型 | `nano-banana-fast` |
| 其他 | 原样传递 |

OpenAI 模型映射：

- 空模型默认 `nano-banana-fast`。
- 非空模型原样传递。

请求体：

```json
{
  "model": "<mapped model>",
  "prompt": "<prompt>",
  "urls": ["<reference image url>"],
  "aspectRatio": "<aspect ratio>",
  "imageSize": "<image size>",
  "shutProgress": true
}
```

`shutProgress=true` 是 grsai 上游协议字段，对齐 Go 版固定传参。按字段名和
Go 版兼容 SSE 的解析逻辑推断，它用于减少或关闭中间进度返回；代理仍同时兼容
普通 JSON 和 SSE `data:` 最终结果。

响应解析：

1. body 为空视为解析失败。
2. body 以 `{` 或 `[` 开头时按 JSON 解析。
3. 否则按 SSE 文本解析，提取最后一条非 `[DONE]` 的 `data:` payload。
4. `code != 0` 视为上游业务错误。
5. `code == 0` 或没有 `code` 时，提取 `data` envelope。
6. 从最终 data 中读取 `status`、`failure_reason`、`error`、`start_time`、
   `end_time` 和 `results[].url`。
7. 成功但没有可用 URL 时，按上游响应异常处理。

### `aiapidev` provider

`aiapidev` 行为保持不变：

- Gemini 使用现有请求改写、创建任务、轮询、结果归一化。
- OpenAI image 使用现有创建任务、轮询、OpenAI response 构造。
- 模型路径映射沿用当前 `rewrite_aiapidev_model_path`。

本次只调整入口分发边界，不主动改变轮询策略或响应结构。

### `transparent` provider

用于未知上游。

职责是保留当前 Rust 标准链路：

- Gemini 透明转发到 `/v1beta/models/{model}:generateContent`。
- OpenAI image 透明转发到 `/v1/images/generations`，并沿用现有图片结果处理。

## 数据流设计

### Gemini `generateContent`

```text
HTTP request
  └── 读取 body
      └── resolve upstream
          └── registry resolve provider
              ├── grsai -> 同步 provider flow
              ├── aiapidev -> 现有异步 flow
              └── transparent -> 现有标准转发 flow
```

`grsai` flow：

1. 从路径模型名和请求体提取参数。
2. 应用 Gemini 模型映射。
3. 抽取 prompt、参考图 URL、`aspectRatio/aspect_ratio`、
   `imageSize/image_size`。
4. POST 到 `resolved.base_url + /v1/draw/nano-banana`。
5. 解析 JSON 或 SSE 最终 payload。
6. 提取图片 URL。
7. 根据 `output` 决定返回 URL 还是下载后转 base64。
8. 构造 Gemini 兼容响应。

### OpenAI `/v1/images/generations`

```text
HTTP request
  └── 读取 body
      └── normalize OpenAI request
          └── resolve upstream
              └── registry resolve provider
                  ├── grsai -> 同步 provider flow
                  ├── aiapidev -> 现有 OpenAI async flow
                  └── transparent -> 现有 OpenAI 转发 flow
```

`grsai` flow：

1. 从 `model/model_name`、`prompt`、`urls/images`、
   `aspect_ratio/aspectRatio`、`image_size/imageSize` 抽参数。
2. 应用 OpenAI 模型默认值。
3. POST 到 `resolved.base_url + /v1/draw/nano-banana`。
4. 解析响应并提取图片 URL。
5. 按现有 URL 包装配置生成 OpenAI `data[].url` 响应。

## 错误处理设计

`grsai` provider 内部需要区分以下错误来源，但对外响应仍走 Rust 现有格式。

### 请求错误

- prompt 为空：返回现有 invalid request 风格。
- JSON 无法解析：沿用当前路由层错误。
- 上游 URL 配置非法：沿用当前上游地址配置错误。

### 上游传输错误

`reqwest` timeout、connect、body、request 等错误继续映射到现有中文错误消息：

- 上游服务响应超时。
- 连接上游服务失败。
- 发送请求内容到上游服务时失败。
- 请求上游服务失败。
- 上游服务通信异常。

### 上游 HTTP 错误

HTTP 非 2xx：

1. 尝试按 grsai JSON/SSE 解析 body。
2. 尝试提取 `code/msg/message`。
3. 下游响应仍保持 Rust 现有错误 schema。
4. admin 日志记录上游 status、body 和解析 detail。

### 上游业务错误

`code != 0`：

1. 视为上游失败。
2. `msg/message/failure_reason` 可进入 admin detail。
3. HTTP status 可参考上游 code 和当前 Rust 错误映射规则，但不新增对外字段。

### 响应格式错误

以下情况返回当前 Rust 风格的解析失败或缺少图片错误：

- body 为空。
- JSON/SSE 解析失败。
- payload 不是 object。
- 缺少 `status`。
- 成功结果缺少 `results[].url`。

## Admin 与观测

保持现有 admin 日志结构，不要求新增 UI。

`grsai` flow 应尽量填充：

1. 原始请求摘要。
2. 上游请求体。
3. 上游响应摘要。
4. 上游 status code。
5. 请求和上游耗时。
6. 请求图片 URL 和响应图片 URL。
7. 错误 detail。

如果低风险，可以在现有字段中补充 provider 名称；但不把 provider 名称作为本次
对外 API schema。

## 测试计划

### 单元测试

Provider registry：

1. `api.grsai.com` 命中 `grsai`。
2. `grsai.com` 命中 `grsai`。
3. `sub.api.grsai.com` 命中 `grsai`。
4. `evilgrsai.com` 不命中。
5. `grsai.com.evil.com` 不命中。
6. `aiapidev.com` 和 `www.aiapidev.com` 命中 `aiapidev`。
7. 未知域返回 transparent。

`grsai` 请求构造：

1. Gemini 模型映射完整覆盖三组已知模型。
2. Gemini 空模型默认 `nano-banana-fast`。
3. OpenAI 空模型默认 `nano-banana-fast`。
4. OpenAI 非空模型原样传递。
5. `prompt`、`urls/images`、`aspectRatio/aspect_ratio`、
   `imageSize/image_size` 正确进入上游请求体。
6. `shutProgress` 固定为 `true`。

`grsai` 响应解析：

1. 普通 JSON 成功响应。
2. SSE `data:` 成功响应。
3. `[DONE]` 被忽略。
4. HTTP 非 2xx 响应。
5. `code != 0` 业务错误。
6. 缺少 `status`。
7. 缺少 `results`。
8. `results` 中没有 URL。
9. 非 JSON 且无 `data:` 行。

### 集成测试

Gemini：

1. base URL 为 mock `grsai` 时，请求打到 `/v1/draw/nano-banana`。
2. `output=url` 返回 URL 型 Gemini inlineData。
3. 默认输出下载 mock 图片并返回 base64 inlineData。
4. grsai 解析错误返回 Rust 现有错误格式。

OpenAI：

1. base URL 为 mock `grsai` 时，请求打到 `/v1/draw/nano-banana`。
2. 返回 OpenAI `data[].url`。
3. URL 包装配置仍生效。
4. 上游缺少图片 URL 时返回现有缺图错误。

回归：

1. `aiapidev` Gemini 异步链路现有测试继续通过。
2. `aiapidev` OpenAI image 异步链路现有测试继续通过。
3. 未知上游透明 Gemini 转发继续通过。
4. 未知上游透明 OpenAI image 转发继续通过。

## 验证命令

实现完成后至少运行：

```bash
timeout 60s cargo fmt -- --check
timeout 60s cargo test
```

如果全量测试超过 60s，应拆分运行相关测试，并说明未跑全量的原因。

