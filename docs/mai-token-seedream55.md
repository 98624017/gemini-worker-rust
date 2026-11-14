# MAI Seedream 5.5 调用说明

调用方继续向代理的 `POST /v1/images/generations` 发送 OpenAI 风格图片请求。代理通过 MAI 的 `/v1/videos` 兼容接口提交图片任务，在同一 HTTP 请求内轮询并返回图片 URL。

MAI 标准图片入口不支持 `seed`，本适配固定采用其文档中的兼容入口。两个入口均可生成 Seedream 图片，`/v1/videos` 的路径名称不改变结果类型。本适配无需新增配置或选择协议。

## 适用范围

只有同时满足以下条件才启用本适配：

- 解析后的上游 host 精确为 `api.mai-token.com`。
- 请求的 `model` 精确为 `seedream-5.5`。

`mai-token.com`、其他子域及 `api.mai-token.com.evil.example` 均不匹配。其他模型和 Gemini `generateContent` 请求沿用原有转发。上游配置可以包含 `/v1` 或尾斜杠，代理基于解析后的 origin 构造提交和查询路径。

上游地址和 API key 沿用现有环境变量或请求头覆盖格式。下面的请求头以 `baseUrl|apiKey` 选择 MAI，`YOUR_API_KEY` 是占位符。若已配置 `UPSTREAM_BASE_URL=https://api.mai-token.com`，也可只传 `Authorization: Bearer YOUR_API_KEY`。

AI Work 当前向 `async.xinbao-ai.com` 提交任务并查询 `/v1/tasks/batch-get`。接入时将本服务配置为该异步服务的后端渠道，继续由异步服务先返回任务 ID。下方 curl 用于直接验证本层的统一图片响应；部署后还需检查异步服务到本层的请求超时，覆盖上游任务等待时间。

## 请求

```bash
curl --fail-with-body --silent --show-error \
	-X POST 'http://127.0.0.1:8787/v1/images/generations' \
	-H 'Authorization: Bearer https://api.mai-token.com|YOUR_API_KEY' \
	-H 'Content-Type: application/json' \
	--data-raw '{
		"model": "seedream-5.5",
		"prompt": "参考图片中的陶瓷狐狸，柔和自然光，保留主体细节",
		"size": "2K",
		"aspect_ratio": "3:2",
		"image": ["https://images.example/reference.png"],
		"seed": 0,
		"response_format": "url"
	}'
```

文生图省略 `image`，或传空数组。请求字段合同如下：

| 字段 | 合同 |
| --- | --- |
| `model` | 必须为 `seedream-5.5`。 |
| `prompt` | 必填的非空字符串，正文原样保留。 |
| `size` | 可省略；接受大小写兼容的 `1K`、`2K`，均生成 `2K`。`4K` 及其他值返回 `400`。 |
| `aspect_ratio` | 可省略，默认 `1:1`。仅接受下文列出的比例。 |
| `image`、`images`、`reference_images` | 可省略或传空数组；每个字段必须为 HTTP(S) 图片 URL 字符串数组，合计最多 10 张。 |
| `seed` | 可省略；传入时必须为整数，`0` 会保留并发送给上游。 |
| `n` | 可省略；传入时仅接受整数 `1`。 |
| `response_format` | 可省略，默认 `url`；仅接受大小写兼容的 `url`。 |

`aspect_ratio` 允许以下值，其他值返回 `400`：

```text
1:1, 16:9, 9:16, 4:3, 3:4, 5:4, 4:5, 3:2, 2:3, 21:9, auto
```

参考图按 `reference_images`、`images`、`image` 的顺序合并，保留数组内顺序和重复 URL，重复项也占数量。超过 10 张、字段不是数组、元素不是字符串或 URL 非 HTTP(S) 时，代理在提交任务前返回 `400`。代理把参考图 URL 发送给 MAI，不在本地下载参考图。

代理按下表构造兼容接口的请求，客户端继续使用上面的字段：

| 客户端字段 | 上游字段 |
| --- | --- |
| `model` | 保持 `seedream-5.5`。 |
| `prompt` | 原样传入 `prompt`。 |
| `size` | 省略、`1K`、`2K` 均使用固定 `quality="1440p"`。 |
| `aspect_ratio` | 传入同名字段，省略时为 `1:1`。 |
| `reference_images`、`images`、`image` | 合并为 `image_urls` URL 数组。 |
| `seed` | 传入同名字段，包括 `0`；省略时不添加。 |

`quality` 和 `image_urls` 由代理构造，客户端无需改用这些上游字段。

## 提交、查询与超时

每次下游请求最多创建一个上游任务：

1. 校验并转换请求，向 MAI 发送 `POST /v1/videos`。
2. 上游返回已完成结果时，直接构造成功响应。任务未完成时，读取 `id` 或 `task_id`，发送 `GET /v1/videos/{taskId}` 查询。
3. 创建后等待 5 秒再首次查询；`queued`、`in_progress` 和 `processing` 状态继续每 5 秒查询，`completed` 返回结果，`failed` 返回任务错误。

请求体读取、提交、响应读取、查询和等待共享 `UPSTREAM_TIMEOUT_MS` 的绝对截止时间，默认 `600000` 毫秒。预算耗尽返回 `504`，不会为每次查询重新计时。

只有查询阶段重试暂时性传输错误，以及 `408`、`425`、`429`、`500`、`502`、`503`、`504` HTTP 状态。连续失败 5 次停止，成功查询后清零。有效的秒数 `Retry-After` 可以延长等待时间，仍受总超时约束。提交失败、响应损坏、超时或连接中断均不会自动重新创建任务。

提交结果尚未确认时，上游可能已收到任务。查询超时后，上游任务也可能继续执行。调用方应先核对已有任务，避免直接重发相同请求产生重复任务。

API key 仅用于上游 API 的提交和查询。代理把结果图片 URL 返回给调用方，不向图片文件域名发送 API key。

## 成功响应

成功返回 HTTP `200` 和 `Content-Type: application/json`：

```json
{
	"created": 1794614400,
	"data": [
		{"url": "https://images.example/generated.png"}
	],
	"usage": {
		"input_tokens": 1024,
		"input_tokens_details": {"image_tokens": 1000, "text_tokens": 24},
		"output_tokens": 1024,
		"total_tokens": 2048,
		"output_tokens_details": {"image_tokens": 1024, "text_tokens": 0}
	}
}
```

`created` 使用 Unix 秒。代理读取上游已完成结果中的 `metadata.url`，映射为本接口的 `data[].url`。缺少结果、结果为空或包含非法图片 URL 时返回协议错误，不返回空的成功响应。

结果默认使用上游直链，不下载或重新上传图片。配置 `OPENAI_IMAGE_UPSTREAM_URL_PROXY_PREFIX` 后，URL 按既有规则包装代理前缀。

`usage` 沿用本服务的固定兼容值，不是 MAI 的真实 token 用量或计费数据，不能用于成本或 token 对账。

## 错误与任务 ID

代理生成的错误使用现有结构化响应。已取得任务 ID 时，错误包含 `error.upstream_task_id`：

```json
{
	"error": {
		"code": 504,
		"message": "等待上游图片任务超时，请查询已有任务，不要重复创建",
		"source": "proxy",
		"stage": "mai_token_poll",
		"kind": "upstream_timeout",
		"upstream_task_id": "task-example"
	}
}
```

| HTTP | 原因 |
| --- | --- |
| `400` | 请求字段、比例、分辨率或参考图不符合合同。 |
| `502` | 上游任务失败、响应无法解析、缺少任务 ID 或结果 URL，以及传输错误。 |
| `504` | 从收到请求开始计算的总超时预算耗尽。 |

上游直接返回的非 `2xx` HTTP 错误保留原有状态和响应主体，沿用本服务的脱敏及明确违规错误缓存规则。已知任务 ID 通过 `x-upstream-task-id` 响应头返回，调用方不应依赖上游错误主体包含 `error.upstream_task_id`。查询重试耗尽后同样返回最后一次错误，不创建新任务。

需要恢复已知任务时，使用同一上游密钥向 `https://api.mai-token.com/v1/videos/{taskId}` 查询；本层的统一图片入口无需新增客户端轮询流程。

`error.stage` 区分 `mai_token_validate_request`、`mai_token_submit` 和 `mai_token_poll`。Admin 日志记录 `upstreamMode=mai-token`、`upstreamTaskId`、`upstreamStage`、上游状态及耗时，API key 不写入这些诊断内容。
