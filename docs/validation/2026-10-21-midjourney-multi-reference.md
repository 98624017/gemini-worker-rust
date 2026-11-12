# Midjourney 多参考图：Imagine 与 Blend

测试日期：2026-10-21（Asia/Shanghai），直连 `https://magic666.top`。延续 [v8.1 单图及 Uguu 验证](2026-10-21-midjourney-v81.md)，使用相同 API key，通过隐藏输入载入进程，未写入文件。

## 选择结论

**多张参考图加文字指令、并明确指定 v8.1，继续使用 `mj_fast_imagine`。** 参考图数量增加不要求改用 Blend。本次两张参考图的 Imagine 已成功生成，并下载、解码、目视检查。

| 需求 | 操作 / 对应模型 | 本次结果 |
| --- | --- | --- |
| 多图参考 + 文字描述 + `--v 8.1` | `/mj/submit/imagine`，`mode=FAST`，对应 `mj_fast_imagine` | 原始 CDN 双图成功 |
| 使用图片直接融图 | `/mj/submit/blend`，`mode=FAST`，对应 `mj_fast_blend` | 未取得任务 ID，返回无可用账号实例 |

根据 [提交文档](https://docs.magic666.top/api-reference/images/midjourney/submit)，Imagine 接收 `prompt` 和多图 `base64Array`；此前已验证公开 URL 可以放在提示词前，本次扩展为两个 URL。Blend 的文档字段为 `mode`、`base64Array`、`dimensions`、`botType`、`notifyHook`、`state`，没有明确的 `prompt` 或版本字段。本次没有创建成功的 Blend 任务，无法验证它实际使用的版本，不能认定为 v8.1。

## 参考图与实际结果

两张图均为前序任务的真实输出单图，PNG，1024 × 1024：

1. 橙色陶瓷狐狸、蓝围巾，1,537,353 字节：<https://imagine-cdn.youchuan.cn/73ab0052-ec0c-42f7-8da9-f710c21452a8_0_0.png>
2. 奶油色陶瓷兔子、绿色围巾，1,471,842 字节：<https://imagine-cdn.youchuan.cn/a90f1315-5800-4bc4-91df-c3dcf8c316a8_0_1.png>

| 请求 | 任务 ID | 结果 |
| --- | --- | --- |
| Imagine，两个 Uguu URL | `1792592344707153` | `FAILURE`，`Prompt图片未通过审核` |
| Imagine，相同 Uguu 请求体重试 | `1792592403976935` | `FAILURE`，相同错误 |
| Imagine，相同图片和描述，改用两个原始 CDN URL | `1792592456977018` | `SUCCESS`，提交至下载完成 67.29 秒 |
| Blend，两个 JPEG Base64，`botType=mj` | 无 | HTTP 400，`Bad Request` |
| Blend，相同输入，省略可选 `botType` | 无 | `code=3`，`No available account instance` |

本次共提交 5 次请求，1 次成功并下载，2 次进入失败终态，2 次在提交阶段未取得任务 ID。

Uguu 链接分别为 `https://h.uguu.se/pixEoOrN.png` 和 `https://h.uguu.se/cmiHSvlq.png`。狐狸图沿用此前上传结果；兔子图重新上传成功，下载文件 SHA-256 与本地原图均为 `3241b08910b70360a0bb8645d186e8b25c2383ab88acbff64a435fdba287bb9e`，未压缩、未重编码。两次失败的 Uguu 请求体通过 `cmp` 确认完全相同。不能把这两个 URL 的失败当作 Imagine 不支持多图；同一组图片换回原始 CDN 后成功，但上游没有提供足够信息来确定 Uguu 链路失败根因。

成功结果下载为 5,735,842 字节 PNG，解码尺寸 **2048 × 1364**，为四宫格。四个候选均可见蓝围巾橙色狐狸和绿围巾浅色兔子，两只陶瓷摆件在木桌上并排出现，背景为暖色摄影棚。最终 `properties.finalPrompt` 保留两个参考图 URL、`--v 8.1`、`--fast` 和 `--iw 1.5`。外观细节仍有变化，没有量化身份一致性或逐张参考图贡献。

[查看两张参考图的 Imagine 生成结果](https://gg-midjourney.oss-cn-shanghai.aliyuncs.com/images/orig_1792592456977018_2805ebd9fa434200917cc4fcc66fee98.png)

## 可复用的 Imagine 请求

发送到 `POST https://magic666.top/mj/submit/imagine`，请求头使用 `Authorization: Bearer YOUR_API_KEY` 和 `Content-Type: application/json`：

```json
{
  "mode": "FAST",
  "prompt": "https://imagine-cdn.youchuan.cn/73ab0052-ec0c-42f7-8da9-f710c21452a8_0_0.png https://imagine-cdn.youchuan.cn/a90f1315-5800-4bc4-91df-c3dcf8c316a8_0_1.png An orange ceramic fox wearing a blue scarf and a cream ceramic rabbit wearing a green scarf, two separate ceramic figurines sitting side by side on a wooden table, both fully visible, warm studio photography, plain cream background --iw 1.5 --ar 3:2 --v 8.1"
}
```

仍按 `result` 里的任务 ID 查询 `GET /mj/task/{id}/fetch`，以实际 `status=SUCCESS` 和图片下载为通过标准。

## Blend 请求与限制

Blend 测试使用上述两张图片各自缩小为 512 × 512、quality=85 的 JPEG，分别约 39KiB 和 35KiB，再编码进 `base64Array`。这是为了控制提交体积；与 Imagine 的 1024 × 1024 URL 输入分开记录。请求不含提示词和版本参数，比例为 `dimensions=SQUARE`。

初次按文档示例传 `botType=mj` 得到 HTTP 400；去掉这一可选字段后，请求返回以下业务响应：

```json
{"code":3,"description":"No available account instance","result":null,"properties":{}}
```

两次请求都没有生成任务或图片。省略字段后错误发生变化，只能确认这两组请求的实际响应，不能单凭它确定 `botType=mj` 在所有渠道都不受支持，或断言 Blend 永久不可用。

## 探针改动与验证

扩展 `scripts/midjourney_smoke.py`：`--reference` 现在接收多个输入；URL / `--sref` 以空格连接，Base64 逐图编码；新增 `--prompt` 和 `--case blend`。既有单图命令保持可用。Blend 使用独立 `/mj/submit/blend` 路径，禁止额外提示词，摘要中的预期路由模型记录为 `mj_fast_blend`。

```bash
rtk proxy python3 scripts/midjourney_smoke.py \
  --case url \
  --reference \
    https://imagine-cdn.youchuan.cn/73ab0052-ec0c-42f7-8da9-f710c21452a8_0_0.png \
    https://imagine-cdn.youchuan.cn/a90f1315-5800-4bc4-91df-c3dcf8c316a8_0_1.png \
  --prompt 'An orange ceramic fox with a blue scarf and a cream ceramic rabbit with a green scarf, sitting side by side on a wooden table --iw 1.5 --ar 3:2 --v 8.1' \
  --output-dir /tmp/mj-multi-new/imagine

rtk proxy python3 scripts/midjourney_smoke.py \
  --case blend --reference /path/to/fox.jpg /path/to/rabbit.jpg \
  --output-dir /tmp/mj-multi-new/blend
```

上述 CLI 示例用于复用，文字稍作简化；完整实测请求以本报告 JSON 及原始 `request.json` 为准。原始证据位于 `/tmp/mj-v81-smoke-20261021/` 下的 `multi-imagine/`、`multi-imagine-retry/`、`multi-imagine-cdn/`、`multi-blend/`、`multi-blend-default-bot/`。

本次验证脚本自检、Ruff 检查及格式检查，并对成功图片实际解码和目视检查。未修改 Rust 业务代码，未测试三张以上参考图或多图 `--sref` / Base64 Imagine 的真实生成，也没有验证本地代理接入。
