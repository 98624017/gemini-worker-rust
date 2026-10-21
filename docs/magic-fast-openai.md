# Magic FAST OpenAI 文生图调用说明

本文说明代理的 Magic FAST 中文文生图路径（任务 #1）。调用方继续使用 OpenAI 图片接口；代理在同一个 HTTP 请求内提交 Magic 任务、查询状态并返回最终图片 URL。

## 适用范围

只有同时满足以下条件才会进入本路径：

- 解析后的上游 host 精确为 `magic666.top`；`magic666.top.evil.example` 等主机不会命中。
- 下游 `model` 精确为 `mj_fast_imagine`。

上游配置可以带 `/v1` 或尾斜杠；代理会基于解析后的 origin 构造 Magic 路径，并保留配置中的 scheme、port 和请求头鉴权覆盖语义。Magic 的其他模型、其他 provider 和非目标 OpenAI 图片请求保持原有分发。

本文说明 FAST 文生图；RELAX 用法见 [RELAX 调用说明](magic-relax-openai.md)。单张及多张参考图用法见 [参考图调用说明](magic-reference-openai.md)。

## 请求

```bash
curl --fail-with-body --silent --show-error \
  -X POST 'http://127.0.0.1:8787/v1/images/generations' \
  -H 'Authorization: Bearer https://magic666.top|YOUR_API_KEY' \
  -H 'Content-Type: application/json' \
  --data-raw '{
    "model": "mj_fast_imagine",
    "prompt": "一只橙色陶瓷狐狸坐在木桌上，暖色工作室摄影 --ar 3:2 --v 8.1",
    "response_format": "url"
  }'
```

示例用现有的 `baseUrl|apiKey` 请求头格式选择 Magic 上游，`YOUR_API_KEY` 是占位符。若服务已配置 `UPSTREAM_BASE_URL=https://magic666.top`，也可只传 `Bearer YOUR_API_KEY`。代理使用解析出的 key 同时发送 Magic 的提交和查询请求。

请求字段合同：

| 字段 | 要求 |
| --- | --- |
| `model` | 必填、非空字符串，且必须是 `mj_fast_imagine`。 |
| `prompt` | 必填、非空字符串；中文正文、`--v 8.1`、`--ar` 等参数原样保留。 |
| `response_format` | 可省略，默认 `url`；仅接受大小写兼容的 `url`。 |
| `image` / `images` / `reference_images` | 省略或传空数组表示文生图；合并后支持一至六张 HTTP(S) 参考图 URL，保留顺序及重复项；超过六张返回 `400`。六张为本层上限，并非上游实测支持承诺。 |

本路径不支持 `b64_json`、`n`、`size`、`quality`、`stream`。这些字段会在创建 Magic 任务前返回清晰的 `400`，不会被通用的 `b64_json` 强制配置改写。图片字段的非法类型、`null`、非字符串元素或无效 URL 也会在创建前返回 `400`。

提示词中的独立 `--fast`、`--relax`、`--turbo` 模式 token 会先移除，再追加恰好一个 `--fast`。正文中包含的同类文本不会被全局替换；例如中文描述里的 `--fast` 子串不会被破坏。

## 上游流程

一次下游请求最多创建一个 Magic 任务：

1. 代理校验请求，不通过校验时不调用 Magic。
2. 发送 `POST /mj/submit/imagine`，请求 JSON 只有 `mode` 和规范化后的 `prompt`：

   ```json
   {
     "mode": "FAST",
     "prompt": "一只橙色陶瓷狐狸 --ar 3:2 --v 8.1 --fast"
   }
   ```

3. 仅当提交响应的业务 `code` 为 `1`、`21` 或 `22`，且 `result` 是非空字符串任务 ID 时才开始查询。HTTP 200 不等于任务已生成；缺少任务 ID、非 JSON 或损坏响应会明确失败。
4. 创建后立即发送 `GET /mj/task/{taskId}/fetch`。任务未完成时默认每 10 秒查询一次；空状态、`NOT_START`、`SUBMITTED`、`IN_PROGRESS` 和未知非终态继续等待。
5. 只有 `status=SUCCESS` 且结果包含合法 HTTP(S) 图片 URL 才算成功。`FAILURE`、`FAILED`、`CANCEL`、`CANCELLED` 和 `MODAL` 直接结束；`progress=100%` 不能单独作为成功依据。

整个流程从收到请求开始受 `UPSTREAM_TIMEOUT_MS` 的绝对截止时间约束，默认 `600000` 毫秒（600 秒）。OpenAI 图片入口读取请求体也受此预算约束；读取超时返回 `504`，阶段为 `read_request_body`。查询的暂时性传输错误及现有可重试 HTTP 状态只在查询阶段重试，连续失败五次停止；成功查询后清零。提交失败、超时或连接中断都不会自动重提任务。调用方断开后代理停止本请求的查询并释放资源，但不会向 Magic 发送取消请求，因此上游任务可能继续运行。

## 成功响应

成功返回 HTTP `200`、`Content-Type: application/json`：

```json
{
  "created": 1792596466,
  "data": [
    {"url": "https://imagine-cdn.example/1.png"},
    {"url": "https://imagine-cdn.example/2.png"}
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

`imageUrls` 存在时优先读取其中全部候选，兼容字符串和 `{ "url": "..." }` 两种元素，保留顺序并去除完全重复项。列表缺失或为空时才回退到 `imageUrl`；非空但损坏的列表、缺少 URL 或非 HTTP(S) URL 返回协议错误，不返回空的 `data`。

`created` 优先取上游正数 `finishTime`，其次取 `submitTime`，均按毫秒转换为 Unix 秒；两者都不可用时使用代理当前时间。返回 URL 默认沿用上游直链；配置 `OPENAI_IMAGE_UPSTREAM_URL_PROXY_PREFIX` 后，直链会按现有前缀策略包装，代理不会强制上传结果。

`usage` 是既有兼容占位统计，不是 Magic 的真实 token 或计费数据，不能用于成本对账。

## 错误响应

错误主体保持统一的结构化合同：

```json
{
  "error": {
    "code": 400,
    "message": "response_format 只支持 url，请调整后再试",
    "source": "proxy",
    "stage": "magic_validate_request",
    "kind": "invalid_request"
  }
}
```

常见状态和处理：

| HTTP | 典型原因 |
| --- | --- |
| `400` | 请求字段非法、参考图格式不支持或解码失败、超过六张参考图、Magic 明确拒绝输入，或业务 code `4`/`24`。 |
| `401` / `403` | Magic 鉴权失败；不要重提相同任务。 |
| `429` | Magic 限流。 |
| `502` | 上游协议损坏、一般上游 HTTP/传输失败、任务失败/取消或成功结果缺失。 |
| `503` | 无可用实例、队列已满或分组负载饱和（包括业务 code `3`/`23`/`30`）。 |
| `504` | 从请求开始计算的绝对期限耗尽，或网络操作超时。 |
| `451` | 仅沿用现有明确违规错误的短期缓存语义；普通审核失败、一般任务失败和其他错误不会扩大为新的长期缓存规则。 |

`error.source`、`error.stage`、`error.kind` 用于区分代理校验、提交、查询和结果解析阶段。能从 Magic 响应取得时，错误还会保留 `business_code`、`failReason` 和 `task_id`；这些诊断字段不包含 API key。提交得到任务 ID 后发生查询或任务失败时，不会重新创建任务。

## Admin 诊断

`/admin/api/logs` 和 admin 详情会记录本次实际链路的 `upstreamMode=FAST`、`upstreamTaskId`、`upstreamStage` 及原有请求/上游/总耗时字段。请求体日志只保留脱敏后的上游提交内容，不记录 API key；参考图的 `base64Array` 仅保留 MIME、大小等摘要。
