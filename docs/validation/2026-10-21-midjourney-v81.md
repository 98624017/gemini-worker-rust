# Midjourney FAST / v8.1 实测

测试日期：2026-10-21（Asia/Shanghai）。直连 `https://magic666.top`，目标模型 ID 为 `mj_fast_imagine`，提示词统一指定 `--v 8.1`。密钥通过隐藏输入进入进程内存，未写入脚本、请求文件或报告。

## 接口与模型选择

依据以下 API 文档：

- <https://docs.magic666.top/api-reference/images/midjourney/overview>
- <https://docs.magic666.top/api-reference/images/midjourney/submit>
- <https://docs.magic666.top/api-reference/images/midjourney/query>

`GET /v1/models` 返回的模型列表包含 `mj_fast_imagine`。MJ 专用协议没有文档化的 `model` 请求字段；本次按文档使用 `POST /mj/submit/imagine` 和 `mode=FAST` 选择 FAST Imagine 路径，版本写在 `prompt` 的 `--v 8.1` 参数中。

提交返回 `code=1` 后，取 `result` 作为任务 ID，每 10 秒查询 `GET /mj/task/{id}/fetch`。只有查询得到 `status=SUCCESS` 且实际下载成功，才计为通过。任务最终 `promptEn` / `properties.finalPrompt` 保留了 `--v 8.1` 和 `--fast`，这是本次版本及模式参数生效的接口证据。

项目当前路由只提供 Gemini / OpenAI 图片入口，未实现 `/mj/*`。本次验证上游接口，新增可复用探针和报告，未改动 Rust 代理业务代码。

## 已完成实测

| 场景 | 输入 | 任务 ID | 终态 | 提交至下载完成 | 主图像素 |
| --- | --- | --- | --- | --- | --- |
| 文生图 | 纯文本 | `1792589432444193` | `SUCCESS` | 70.31 秒 | 2048 × 2048 |
| URL 垫图 | 提示词前置图片 URL，`--iw 1.5` | `1792589624436647` | `SUCCESS` | 67.29 秒 | 2048 × 2048 |
| Base64 垫图 | 512 × 512 JPEG，`base64Array`，`--iw 1.5` | `1792589706816298` | `SUCCESS` | 59.65 秒 | 2048 × 2048 |
| 风格参考图 | `--sref` 图片 URL | `1792589787101842` | `SUCCESS` | 89.73 秒 | 2048 × 2048 |

耗时包含提交、轮询等待和图片下载。成功主图均为 PNG 四宫格；文生图任务的 `imageUrls` 还提供 4 张单图，第一张实际下载并解码为 1024 × 1024。主图 2048 × 2048 不表示每张单图都是 2K。

文生图生成橙色陶瓷狐狸、蓝围巾、木桌和奶油色背景。后续参考图统一取该任务的第一张单图：

<https://imagine-cdn.youchuan.cn/73ab0052-ec0c-42f7-8da9-f710c21452a8_0_0.png>

目视检查：URL 垫图保留了橙色狐狸、蓝围巾、白色脸颊和陶瓷质感，并将背景改成雨天窗台；Base64 垫图保留相同主要特征，并按提示加入绿色盆栽。脸型、围巾纹理和姿势仍有变化。

`--sref` 使用相同狐狸图引导新主体“陶瓷兔子”：输出为兔子、木桌、陶瓷质感和柔和光线，风格与参考图相似。提示词指定绿色围巾，但部分候选仍呈蓝色，颜色遵循不完全。本次验证了接口接受 `--sref`、保留于最终提示词并完成生成，没有进行量化风格一致性评估。

结果图片：

- [文生图](https://gg-midjourney.oss-cn-shanghai.aliyuncs.com/images/orig_1792589432444193_a43af7c094254d85adf6dd4eb2dce0c0.png)
- [URL 垫图](https://gg-midjourney.oss-cn-shanghai.aliyuncs.com/images/orig_1792589624436647_037058ea472440bc99d743de80b87858.png)
- [Base64 垫图](https://gg-midjourney.oss-cn-shanghai.aliyuncs.com/images/orig_1792589706816298_472cc819ee6f4eaaaa25db537f7f0129.png)
- [风格参考图](https://gg-midjourney.oss-cn-shanghai.aliyuncs.com/images/orig_1792589787101842_c671f94cd57e4c5fa09f063c2c4353e7.png)

上述首轮共提交 5 次 Imagine 请求：4 次成功并下载、解码、目视检查；首次大 PNG Base64 请求失败。初始查询曾返回 `status=""`、`progress="0%"`，继续轮询后进入 `IN_PROGRESS` 和 `SUCCESS`；不能把初始空状态直接判为失败。

## 首次 Base64 提交失败

最初直接将 1024 × 1024、约 1.5MiB 的参考 PNG 放入 `base64Array`，提交接口返回 HTTP 400，没有任务 ID：

```json
{"code":5,"description":"do_request_failed ","type":"upstream_error"}
```

随后将同一输入图缩小为 512 × 512、约 39KiB 的 JPEG，再次使用相同字段和提示词提交，成功生成并下载。由于这次同时改变了体积、尺寸和格式，且两次请求发生在不同时间，不能单凭这两次结果确定失败根因或推导图片大小上限。实际接入可优先使用已验证的公开 URL，或较小的 JPEG Base64 输入。所有输出图片均直接下载，没有缩放原始输出。

## 请求示例

以下请求均发送到 `POST https://magic666.top/mj/submit/imagine`，请求头为 `Authorization: Bearer YOUR_API_KEY` 和 `Content-Type: application/json`。

文生图：

```json
{
  "mode": "FAST",
  "prompt": "A small orange ceramic fox with a blue scarf, sitting on a wooden table, warm studio photography, plain cream background --ar 1:1 --v 8.1"
}
```

URL 垫图：

```json
{
  "mode": "FAST",
  "prompt": "https://imagine-cdn.youchuan.cn/73ab0052-ec0c-42f7-8da9-f710c21452a8_0_0.png An orange ceramic fox with a blue scarf inspired by the reference image, on a windowsill on a rainy day, studio photography --iw 1.5 --ar 1:1 --v 8.1"
}
```

Base64 垫图使用相同接口，提示词不必添加 URL。探针将本地完整图片编码为 `data:image/jpeg;base64,...` 放入 `base64Array`，实际请求保存在测试输出目录的 `request.json`。成功任务的 `promptEn` / `finalPrompt` 显示网关先把该 Base64 图转成公开图片地址，再加入最终提示词。

风格参考图：

```json
{
  "mode": "FAST",
  "prompt": "A ceramic rabbit with a green scarf on a wooden table, studio photography --ar 1:1 --v 8.1 --sref https://imagine-cdn.youchuan.cn/73ab0052-ec0c-42f7-8da9-f710c21452a8_0_0.png"
}
```

## 复用探针与验证

脚本：`scripts/midjourney_smoke.py`，只依赖 Python 标准库。未设置 `MIDJOURNEY_API_KEY` 时使用隐藏输入；每次运行默认会提交一个生成任务。`--task-id` 可直接继续查询已有任务，避免重复提交。

```bash
rtk proxy python3 scripts/midjourney_smoke.py --models-only

rtk proxy python3 scripts/midjourney_smoke.py \
  --case text --output-dir /tmp/mj-v81-new/text

rtk proxy python3 scripts/midjourney_smoke.py \
  --case url \
  --reference https://imagine-cdn.youchuan.cn/73ab0052-ec0c-42f7-8da9-f710c21452a8_0_0.png \
  --output-dir /tmp/mj-v81-new/url

rtk proxy python3 scripts/midjourney_smoke.py \
  --case base64 --reference /path/to/reference.jpg \
  --output-dir /tmp/mj-v81-new/base64

rtk proxy python3 scripts/midjourney_smoke.py \
  --case style \
  --reference https://imagine-cdn.youchuan.cn/73ab0052-ec0c-42f7-8da9-f710c21452a8_0_0.png \
  --output-dir /tmp/mj-v81-new/style
```

本次原始请求、提交响应、最终任务 JSON、图片和摘要保存在 `/tmp/mj-v81-smoke-20261021/`。其中 `base64/` 是首次失败的请求，`base64-small/` 是成功的较小 JPEG 请求。图片预览单独缩小，原始 `output.image` 保持下载字节不变。

已通过脚本自检（包含 Base64 编码、参考图输入校验、提交返回码和任务 ID 校验）、`ruff check`、`ruff format --check`。成功图片均使用 Pillow 实际解码并目视检查。未修改 Rust 业务代码，未运行 Rust 全量测试；本次未验证本地代理的 MJ 接入、放大、变体、多参考图或角色一致性。

## Uguu 原始大图补充实测

同日按补充要求，把原始 PNG 上传到 `POST https://uguu.se/upload`，使用 multipart 字段 `files[]` 获得公开 URL，然后将该 URL 放在提示词前作为垫图，保持 `mode=FAST`、`--iw 1.5 --ar 1:1 --v 8.1`。本次上传不缩小、不重编码图片。

结论：两张原始 PNG 均可经 Uguu URL 完成垫图生成。其中 7.73MiB 图片首次请求成功；1.47MiB 图片首次出现审核错误，保持图片、URL 和请求体不变重试后成功。补充实测共提交 3 次任务，2 次成功、1 次失败。

| 输入图片 | 原始字节数 | Uguu URL | MJ 任务 ID | 结果 |
| --- | --- | --- | --- | --- |
| 1024 × 1024 单图 PNG，与首次 Base64 失败的输入相同 | 1,537,353（1.47MiB） | `https://h.uguu.se/pixEoOrN.png` | `1792590128080727` | `FAILURE`，`Prompt图片未通过审核` |
| 2048 × 2048 四宫格 PNG，首轮文生图原始输出 | 8,109,298（7.73MiB） | `https://n.uguu.se/irRULlGi.png` | `1792590193117100` | `SUCCESS`，68.50 秒完成并下载 |
| 1024 × 1024 单图 PNG，保持原 URL 和请求体重试 | 1,537,353（1.47MiB） | `https://h.uguu.se/pixEoOrN.png` | `1792590294038458` | `SUCCESS`，81.39 秒完成并下载 |

两个 Uguu URL 均实际下载成功，HTTP 200、`image/png`、字节数与原文件一致。重新下载文件的 SHA-256 与各自原文件完全相同：

- 1.47MiB 单图：`1cf18f817124ffc987e8f45c7cfe63eea75bc530cf42c06414685abd917c94eb`
- 7.73MiB 四宫格：`6d1e5201cd4585bd40d0a6336a5b326340f8fd37880bc02f8464b35335dd078d`

7.73MiB 请求的最终提示词保留了 Uguu URL、`--v 8.1`、`--fast` 和 `--iw 1.5`。返回图片实际下载为 9,893,918 字节的 PNG，解码尺寸为 2048 × 2048，目视可见橙色陶瓷狐狸、蓝围巾和雨天窗台。部分候选出现两只狐狸；此次输入本身为四宫格参考图，没有量化其构图影响。

[7.73MiB Uguu 垫图生成结果](https://gg-midjourney.oss-cn-shanghai.aliyuncs.com/images/orig_1792590193117100_b85f0df6493f46209be58890e01118b3.png)

1.47MiB 原图重试的生成结果也已下载、解码和目视检查：10,127,308 字节 PNG，2048 × 2048，四个候选均保留单只橙色狐狸、蓝围巾和雨天窗台。两次请求体文件通过 `cmp` 确认为完全相同。

[1.47MiB Uguu 原图重试生成结果](https://gg-midjourney.oss-cn-shanghai.aliyuncs.com/images/orig_1792590294038458_96591874acd54a0cb98c6bcc680a63f9.png)

首次 1.47MiB 请求虽然提交返回 `code=1` 并获得任务 ID，但约 23 秒后的查询已是失败终态，没有生成结果。同一 URL 和请求体随后重试成功，说明本次失败可以在不修改输入的情况下恢复。失败响应只提供“Prompt图片未通过审核”，其内部原因仍未确定，也不能据此推导尺寸限制。

上传及生成调用示例：

```bash
rtk proxy curl -fSs --max-time 180 \
  -F 'files[]=@/tmp/mj-v81-smoke-20261021/text/output.image;type=image/png;filename=reference-large.png' \
  https://uguu.se/upload

rtk proxy python3 scripts/midjourney_smoke.py \
  --case url --reference https://n.uguu.se/irRULlGi.png \
  --output-dir /tmp/mj-v81-new/uguu-large
```

原始任务证据保存在 `/tmp/mj-v81-smoke-20261021/uguu-original/`、`uguu-large/` 和 `uguu-original-retry/`。本次只补充外部 API 实测和报告，没有修改脚本或 Rust 代码，未重复此前已通过的代码检查。Uguu 的 FAQ 声明文件约 3 小时后删除，以上图床 URL 仅用于本次即时测试。
