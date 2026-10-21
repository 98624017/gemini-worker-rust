# Magic RELAX OpenAI 文生图调用说明

调用方将 OpenAI 图片请求的 `model` 设为 `mj_relax_imagine`，即可在同一个 HTTP 请求内等待 Magic RELAX 任务完成并取得图片 URL。中文正文无需翻译，版本和宽高比继续由 prompt 中的 MJ 参数控制。

## 请求

```bash
curl --fail-with-body --silent --show-error \
  -X POST 'http://127.0.0.1:8787/v1/images/generations' \
  -H 'Authorization: Bearer https://magic666.top|YOUR_API_KEY' \
  -H 'Content-Type: application/json' \
  --data-raw '{
    "model": "mj_relax_imagine",
    "prompt": "一只橙色陶瓷狐狸坐在木桌上，暖色工作室摄影 --ar 3:2 --v 8.1",
    "response_format": "url"
  }'
```

`YOUR_API_KEY` 是占位符；若服务已配置 `UPSTREAM_BASE_URL=https://magic666.top`，可只传 `Bearer YOUR_API_KEY`。提交和查询均使用解析后的同一 Bearer key。

仅在上游 host 精确为 `magic666.top`，且 `model` 精确为 `mj_relax_imagine` 时采用此适配。配置地址可带 `/v1` 或尾斜杠，代理从 origin 构造固定路径。其他 host、Magic 其他模型和既有 FAST 请求保持现有分发。

`prompt` 必须是非空字符串。`response_format` 可省略，默认 `url`，兼容大小写；`b64_json`、`null`、`n`、`size`、`quality`、`stream` 会在提交前返回 `400`。省略图片字段或传空数组表示文生图，不发送 `base64Array`；`image` / `images` / `reference_images` 合并后支持一张 HTTP(S) URL，处理规则与 FAST 共用，见 [单参考图调用说明](magic-reference-openai.md)。多张参考图会明确返回 `400`。

## 实际慢速链路

模型决定模式：独立的 `--fast`、`--turbo` 和重复 `--relax` 会移除，再追加恰好一个 `--relax`。中文正文、嵌在正文中的同名子串、`--ar`、`--v 8.1` 和其他非模式参数保留。例如：

```text
输入：中文狐狸，正文--fast不改 --fast --turbo --relax --ar 3:2 --v 8.1 --relax
上游：中文狐狸，正文--fast不改 --ar 3:2 --v 8.1 --relax
```

代理发送 `POST /mj-relax/mj/submit/imagine`：

```json
{
  "mode": "RELAX",
  "prompt": "一只橙色陶瓷狐狸坐在木桌上，暖色工作室摄影 --ar 3:2 --v 8.1 --relax"
}
```

慢速提交路径、`mode=RELAX` 和显式 `--relax` 一起选择实际慢速渠道。提交成功后仍用通用的 `GET /mj/task/{taskId}/fetch` 查询，只有任务 `status=SUCCESS` 且结果 URL 合法时才返回成功。

## 生命周期与响应

RELAX 复用 [FAST 调用说明](magic-fast-openai.md) 中的输入校验、一次提交、查询重试、取消、响应和错误合同：

- 从收到请求起共用 `UPSTREAM_TIMEOUT_MS` 绝对截止时间，默认 `600000` 毫秒（600 秒）。等待超时返回 `504`，不沿用 aiapipro 的固定 450 秒等待上限。
- 提交最多一次；创建后立即查询，未完成时每 10 秒继续查询。查询暂时故障连续五次停止，成功查询后清零。断开调用方连接会停止本请求的查询；已创建的 Magic 任务仍可能继续运行。
- 成功返回 OpenAI `created`、`data[].url` 和既有兼容 `usage`。默认保留所有候选的顺序并去重，候选列表缺失或为空时回退主图；`usage` 不代表 Magic 的实际用量或计费。
- 输入错误返回 `400`，鉴权和限流保留 `401` / `403` / `429`，容量不足返回 `503`，任务失败或协议损坏通常返回 `502`。错误保留可用的任务 ID、阶段和失败原因。

Admin 中的 `upstreamMode=RELAX` 和脱敏 `requestUpstream` 来自实际准备的上游请求，同时记录 `upstreamTaskId`、`upstreamStage` 和耗时。仅看到下游 `model=mj_relax_imagine` 不足以证明 Magic 实际执行了 RELAX；真实链路验证还需核对慢速提交路径、最终任务参数、终态和结果可下载性。

本次代理链路实测状态见 [RELAX 验证记录](validation/2026-10-22-magic-relax-openai.md)。
