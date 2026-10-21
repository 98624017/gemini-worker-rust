# Magic 单参考图 OpenAI 调用

Magic 上游支持 `mj_fast_imagine` 与 `mj_relax_imagine` 的单张参考图请求。向现有 `POST /v1/images/generations` 发送 HTTP(S) 图片 URL 数组；代理负责下载、预处理及 Base64 Data URI 提交，随后等待任务成功并返回 OpenAI 图片 URL 响应。

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

参考图沿用现有别名合并顺序：`reference_images`、`images`、`image`。本阶段合并后仅支持一张，重复 URL 也按多张拒绝。省略字段或传空数组仍是文生图。字段必须为 URL 字符串数组，不接受 `null`、标量、Data URI、原始 Base64 或私有 `base64Array` 字段；生成结果仅支持 `response_format: "url"`。

下载沿用现有图片客户端、网络目标约束和超时，每张最多 20MiB，图片源不会收到 Magic API key。代理按实际可解码字节识别 JPEG、PNG、WebP；损坏或不支持的图片返回 400，下载失败返回 502，准备、提交与轮询共享请求的绝对截止时间，超时返回 504。任一步准备失败都不会创建 Magic 任务。

图片最长边只在超过 2048 时等比缩小，不放大或裁切。尺寸合规的 JPEG/WebP 保留原始字节；超限 JPEG 保留格式并以质量 98、4:4:4 重编码，WebP 使用无损编码并保留透明度。PNG 一律转为质量 98、4:4:4 JPEG，透明像素合成白底。解码的内存分配上限为 128MiB，超出上限的图片需要先缩小后再请求。

MJ 下载绕过可能含 PNG→WebP 变体的共享请求图片缓存，以保证输入格式合同；它不会改写其他渠道的缓存。上游请求体分块编码并遵守 Blob Runtime 存储预算，请求结束、失败或取消后回收临时资源。admin 只保存图片 MIME、大小等摘要。

本页描述的新增 OpenAI 单参考图链路尚未进行真实 Magic 计费验收；本地验证使用图片 HTTP 服务和模拟 Magic。既有原生协议单图成功记录不代表本链路已经实测通过。多参考图属于后续 #4。
