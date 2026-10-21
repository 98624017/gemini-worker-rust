# Magic FAST / RELAX 文生图与参考图调用

代理为 `mj_fast_imagine` 与 `mj_relax_imagine` 提供中文文生图、单参考图与多参考图的同步接口。向现有 `POST /v1/images/generations` 发送 HTTP(S) 图片 URL 数组；代理负责下载、预处理及 Base64 Data URI 提交，随后等待任务成功并返回 OpenAI 图片 URL 响应。

下面三个请求体均可用于两个模型；把 `model` 换为 `mj_relax_imagine` 即选择 RELAX。文生图省略参考图字段：

```json
{"model":"mj_fast_imagine","prompt":"一只橙色陶瓷狐狸，暖色工作室摄影 --ar 3:2 --v 8.1","response_format":"url"}
```

单图请求：

```bash
curl https://proxy.example/v1/images/generations \
  -H 'Authorization: Bearer YOUR_API_KEY' \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "mj_fast_imagine",
    "prompt": "参考图片中的陶瓷狐狸，柔和自然光 --ar 3:2 --v 8.1",
    "image": ["https://images.example/reference.png"],
    "response_format": "url"
  }'
```

使用 `mj_relax_imagine` 即走 RELAX 提交路径和模式。中文与版本等 prompt 参数原样保留；模型决定最终的 `--fast` 或 `--relax` 参数。

双图请求体（沿用上例的 URL 和请求头）：

```json
{
  "model": "mj_relax_imagine",
  "prompt": "融合第一张图的主体和第二张图的配色，柔和自然光 --ar 3:2 --v 8.1",
  "image": [
    "https://img.cdn1.vip/i/69a3fd7a9d39a_1772354938.webp",
    "https://img.cdn1.vip/i/69e9c4bc4e0db_1776927932.webp"
  ],
  "response_format": "url"
}
```

参考图沿用现有别名合并顺序：`reference_images`、`images`、`image`；数组内顺序和重复项均保留。合并后最多六张，重复 URL 也占数量；六张是代理的输入上限，不代表 Magic 已实测支持六张。省略字段或传空数组仍是文生图。字段必须为 URL 字符串数组，不接受 `null`、标量、Data URI、原始 Base64 或私有 `base64Array` 字段；生成结果仅支持 `response_format: "url"`。超过六张或任一项非法会在下载、创建任务前返回 400。

下载沿用现有图片客户端、网络目标约束和超时，每张最多 20MiB，图片源不会收到 Magic API key。代理按实际可解码字节识别 JPEG、PNG、WebP；损坏或不支持的图片返回 400，下载失败返回 502，错误注明图片序号。准备、提交与轮询共享从请求开始计算的绝对截止时间，`UPSTREAM_TIMEOUT_MS` 默认 600000 毫秒（600 秒），超时返回 504。全部图片成功后才提交一次；任一步准备失败都不会创建 Magic 任务，不会丢图或降级文生图。

图片最长边只在超过 2048 时等比缩小，不放大或裁切。尺寸合规的 JPEG/WebP 保留原始字节；超限 JPEG 保留格式并以质量 98、4:4:4 重编码，WebP 使用无损编码并保留透明度。PNG 一律转为质量 98、4:4:4 JPEG，透明像素合成白底。解码的内存分配上限为 128MiB，超出上限的图片需要先缩小后再请求。

MJ 下载绕过可能含 PNG→WebP 变体的共享请求图片缓存，以保证输入格式合同；它不会改写其他渠道的缓存。每个请求逐张准备图片，复用既有全局并发槽及 Blob Runtime 存储预算，再分块编码全部 Data URI，不在内存中构造多份完整 Base64 和 JSON。请求结束、失败或取消后回收临时资源。admin 逐项只保存图片 MIME、大小等摘要。

结果默认返回全部候选单图 `data[].url`，没有候选列表时回退主图，主图可能为四宫格；不强制上传图床。`created` 使用 Unix 秒，`usage` 沿用兼容占位值 `input_tokens=1024`、`output_tokens=1024`、`total_tokens=2048`，并非 Magic 的真实用量或计费数据。本适配不宣称全面兼容 OpenAI 生图参数：`n`、`size`、`quality`、`stream` 返回 400，生成控制继续使用 prompt 中的 MJ 参数。

新增 OpenAI 链路的真实 Magic 计费验收状态见 [多参考图验收记录](validation/2026-10-22-magic-multiple-references-openai.md)。本地模拟成功、既有双 URL 任务和原生单图成功，均不能替代两项 Base64 Data URI 的真实双图任务验收。
