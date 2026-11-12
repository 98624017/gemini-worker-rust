# mj_fast_imagine API 调用文档

本文整理 2026-10-21 的真实调用结果，覆盖 **v8.1 文生图**和**参考图生图**，包括单图、多图和风格参考图。

## 1. 接口与认证

| 项目 | 配置 |
| --- | --- |
| 上游地址 | `https://magic666.top` |
| 模型 ID | `mj_fast_imagine` |
| 提交接口 | `POST /mj/submit/imagine` |
| 调用模式 | 请求体 `mode="FAST"` |
| 模型版本 | 在 `prompt` 中添加 `--v 8.1` |
| 查询接口 | `GET /mj/task/{taskId}/fetch` |
| 认证 | `Authorization: Bearer YOUR_API_KEY` |
| 提交内容类型 | `Content-Type: application/json` |

这套接口通过 Imagine 路径和 FAST 模式选择目标能力，**请求体不需要传 `model` 字段**。文生图、单图参考、多图参考使用相同的提交和查询接口。

**接入建议：文生图传 `prompt`；参考图生图默认使用 `base64Array`，单图和多图采用同一字段。** 这与官方示例一致，可以减少对自行上传图床、临时 URL 有效期和外部图床访问条件的依赖。公开 URL 作为补充输入方式，相关实测记录保留在下文。

本次已验证较小的单图 Base64 成功；多图 Base64 的端到端稳定性仍待实测。网关实际会将 Base64 图转成公开地址后使用，不能认为此方案消除了上游内部抓图环节。

下文均直连上游。本仓库目前没有 `/mj/*` 代理路由，不能把示例地址直接换成本地 Rust 服务地址。

示例从环境变量 `MIDJOURNEY_API_KEY` 读取密钥；调用前在自己的运行环境中设置该变量，不要将真实密钥写入请求文件或版本库。

## 2. 文生图

### 请求

```bash
curl --fail-with-body --silent --show-error \
  https://magic666.top/mj/submit/imagine \
  -H "Authorization: Bearer ${MIDJOURNEY_API_KEY}" \
  -H 'Content-Type: application/json' \
  --data-raw '{
    "mode": "FAST",
    "prompt": "A small orange ceramic fox with a blue scarf, sitting on a wooden table, warm studio photography, plain cream background --ar 1:1 --v 8.1"
  }'
```

提示词描述主体、场景和画面要求，并在末尾指定比例及版本。以上请求已实测成功，任务 ID 为 `1792589432444193`。

### 提交响应

```json
{
  "code": 1,
  "description": "Submit success",
  "result": "1792589432444193",
  "properties": {
    "discordInstanceId": "1785c79933f847b3b90125108a85232b"
  }
}
```

`result` 是新任务 ID。提交成功后继续查询任务，不能直接将 `code=1` 当作图片已经生成。`properties` 是附加信息，调用方无需依赖其中的实例 ID。

## 3. 参考图生图

### 3.1 Base64 单图与多图（默认方案）

图片编码为完整 Data URI 后放入 `base64Array`，提示词只包含文字描述和 MJ 参数。将下列 JSON 作为 `POST /mj/submit/imagine` 的请求体，认证头与文生图相同。下面是结构示意，`<完整图片Base64>` 需要替换为实际编码内容：

```json
{
  "mode": "FAST",
  "prompt": "An orange ceramic fox with a blue scarf inspired by the reference image, sitting beside a tiny green plant, bright studio photography --iw 1.5 --ar 1:1 --v 8.1",
  "base64Array": [
    "data:image/jpeg;base64,<完整图片Base64>"
  ]
}
```

JPEG 使用 `data:image/jpeg;base64,` 前缀，PNG 使用 `data:image/png;base64,`。数组中的每一项对应一张图片；多图在同一数组中依次添加各图的完整 Data URI：

```json
{
  "mode": "FAST",
  "prompt": "An orange ceramic fox wearing a blue scarf and a cream ceramic rabbit wearing a green scarf, sitting side by side on a wooden table, warm studio photography --iw 1.5 --ar 3:2 --v 8.1",
  "base64Array": [
    "data:image/jpeg;base64,<第一张图片的完整Base64>",
    "data:image/jpeg;base64,<第二张图片的完整Base64>"
  ]
}
```

本次一张 512 × 512、约 39KiB 的 JPEG Base64 输入已成功，任务 ID 为 `1792589706816298`。网关将输入图转成公开地址后加入最终提示词。多张 Base64 是上游文档支持的形式，但本次未验证其真实生成结果。

较大的 1024 × 1024、约 1.47MiB PNG Base64 曾在提交阶段返回 HTTP 400、`do_request_failed`；缩小并转为 JPEG 后成功。这两次同时改变了格式和尺寸，不能据此推导图片大小上限。

### 3.2 单张公开 URL（补充方式）

已有可供上游直接下载的公开图源时，可以将图片 URL 放在提示词最前面，用空格与文字描述分隔。图源不应依赖浏览器登录态或额外认证头。

```json
{
  "mode": "FAST",
  "prompt": "https://imagine-cdn.youchuan.cn/73ab0052-ec0c-42f7-8da9-f710c21452a8_0_0.png An orange ceramic fox with a blue scarf inspired by the reference image, on a windowsill on a rainy day, studio photography --iw 1.5 --ar 1:1 --v 8.1"
}
```

以上请求已成功生成橙色狐狸、蓝围巾和雨天窗台，任务 ID 为 `1792589624436647`。

### 3.3 多张公开 URL（补充方式）

依次把多个图片 URL 放在提示词最前面，彼此用空格分隔，再接文字描述和参数。本次已验证两张参考图，继续使用 `mj_fast_imagine`。

```json
{
  "mode": "FAST",
  "prompt": "https://imagine-cdn.youchuan.cn/73ab0052-ec0c-42f7-8da9-f710c21452a8_0_0.png https://imagine-cdn.youchuan.cn/a90f1315-5800-4bc4-91df-c3dcf8c316a8_0_1.png An orange ceramic fox wearing a blue scarf and a cream ceramic rabbit wearing a green scarf, two separate ceramic figurines sitting side by side on a wooden table, both fully visible, warm studio photography, plain cream background --iw 1.5 --ar 3:2 --v 8.1"
}
```

以上请求已成功，任务 ID 为 `1792592456977018`。四个候选均出现蓝围巾狐狸与绿围巾兔子。三张以上参考图及图片数量上限未实测。

### 3.4 风格参考图

需要借用参考图风格时，可以在提示词中使用 `--sref 图片URL`，接口仍为 Imagine：

```json
{
  "mode": "FAST",
  "prompt": "A ceramic rabbit with a green scarf on a wooden table, studio photography --ar 1:1 --v 8.1 --sref https://imagine-cdn.youchuan.cn/73ab0052-ec0c-42f7-8da9-f710c21452a8_0_0.png"
}
```

本次已成功，任务 ID 为 `1792589787101842`，输出具有相似陶瓷质感和摄影风格的兔子。部分候选围巾颜色仍偏蓝，不能保证参考特征或文字要求精确一致。多张 `--sref` 未实测。

### 3.5 本地大图先上传 Uguu（URL 补充方式）

按 Uguu 的 multipart 接口上传图片：

```bash
curl --fail-with-body --silent --show-error \
  -F 'files[]=@reference.png;type=image/png' \
  https://uguu.se/upload
```

确认返回 `success=true`，从 `files[0].url` 取真实公开地址，再按 3.2 的方式放到 Imagine 提示词最前面。上传到 Uguu 不需要携带 magic API key。

实测记录：

| 输入与方式 | 结果 |
| --- | --- |
| 1.47MiB PNG，上传 Uguu 后单图 URL 垫图 | 首次审核失败，同图、同 URL、同请求体重试后成功 |
| 7.73MiB PNG，上传 Uguu 后单图 URL 垫图 | 首次成功，约 69 秒完成并下载 |
| 两张 Uguu URL 同时垫图 | 连续两次失败，`Prompt图片未通过审核` |
| 相同两张图改用原始 CDN URL | 成功，约 67 秒完成并下载 |

两张大 PNG 上传前后的 SHA-256 一致，确认无需压缩即可完成已成功的单图 URL 流程。7.73MiB 是已验证样本大小，不代表接口大小上限。默认接入按 3.1 使用 Base64；采用 URL 补充方式时，优先选择已验证可访问的 CDN 图源。目前不能承诺 Uguu 双图流程可用，也不能仅凭审核错误确定失败根因。

Uguu FAQ 声明文件约 3 小时后删除，因此应在上传后及时提交和完成任务，不要将临时地址作为长期素材链接。

## 4. 请求字段与提示词参数

| 参数 | 位置 | 用法 |
| --- | --- | --- |
| `mode` | JSON 字段 | 本目标固定使用 `FAST` |
| `prompt` | JSON 字段 | 文字描述和 MJ 参数；使用 URL 补充方式时在前部添加图源 URL |
| `base64Array` | JSON 字段 | 参考图生图默认使用的 Data URI 数组；文生图省略 |
| `--v 8.1` | `prompt` 内 | 指定本次目标版本 |
| `--ar 1:1` / `--ar 3:2` | `prompt` 内 | 本次已验证的目标宽高比 |
| `--iw 1.5` | `prompt` 内 | 本次普通参考图生成使用的图片权重值；未测试其他值或范围 |
| `--sref URL` | `prompt` 内 | 指定风格参考图 |

单图和多图 URL 请求都将完整 URL 写入 `prompt`，不要把它们放进仅用于 Data URI 的 `base64Array`。本次没有验证 URL、Base64、`--sref` 混合输入。

## 5. 查询任务与获取图片

### 查询请求

```bash
curl --fail-with-body --silent --show-error \
  https://magic666.top/mj/task/1792589432444193/fetch \
  -H "Authorization: Bearer ${MIDJOURNEY_API_KEY}"
```

历史任务 ID 仅用于展示响应格式；自己的调用应使用刚才提交响应的 `result`。本次探针每 10 秒查询一次，等待上限为 600 秒；这些是客户端策略，不是服务端完成时限承诺。

### 状态处理

| status | 处理方式 |
| --- | --- |
| `""`、`NOT_START`、`SUBMITTED`、`IN_PROGRESS` | 继续等待；初始空状态已在实测中出现 |
| `SUCCESS` | 读取结果 URL，并实际下载确认图片可用 |
| `FAILURE`、`CANCEL` | 结束等待，向调用方返回 `failReason` 和任务 ID |
| `MODAL` | 需要额外交互，保留任务 ID 并明确返回该状态；本流程不自动续提 |
| 其他未知状态 | 保留原始状态；不能按成功处理，并受客户端等待上限约束 |

达到客户端等待上限时，保留任务 ID，后续继续查询这个任务；超时本身不证明上游任务失败。已取得任务 ID 时，不要因为轮询或图片下载出错就重新提交生成任务。

### 成功响应关键字段

以下为真实成功响应的字段节选：

```json
{
  "id": "1792589432444193",
  "action": "IMAGINE",
  "status": "SUCCESS",
  "progress": "100%",
  "failReason": "",
  "imageUrl": "https://gg-midjourney.oss-cn-shanghai.aliyuncs.com/images/orig_1792589432444193_a43af7c094254d85adf6dd4eb2dce0c0.png",
  "imageUrls": [
    {"url": "https://imagine-cdn.youchuan.cn/73ab0052-ec0c-42f7-8da9-f710c21452a8_0_0.png"},
    {"url": "https://imagine-cdn.youchuan.cn/73ab0052-ec0c-42f7-8da9-f710c21452a8_0_1.png"},
    {"url": "https://imagine-cdn.youchuan.cn/73ab0052-ec0c-42f7-8da9-f710c21452a8_0_2.png"},
    {"url": "https://imagine-cdn.youchuan.cn/73ab0052-ec0c-42f7-8da9-f710c21452a8_0_3.png"}
  ]
}
```

`imageUrl` 在本次 Imagine 成功任务中是四宫格主图，`imageUrls` 提供候选单图地址；该列表按上游文档也可能直接包含字符串 URL，读取时兼容字符串和 `{ "url": "..." }` 两种形式。

文生图主图实际为 2048 × 2048，第一张候选单图为 1024 × 1024。双图参考的 `--ar 3:2` 结果主图为 2048 × 1364。不要把四宫格主图的尺寸当作每张单图的尺寸，也不要仅凭 `--ar` 推断固定像素大小。

下载时不向第三方图片域名发送 magic API key。例如获取第一张单图：

```bash
curl --fail --location --silent --show-error \
  https://imagine-cdn.youchuan.cn/73ab0052-ec0c-42f7-8da9-f710c21452a8_0_0.png \
  --output candidate-1.png
```

## 6. 提交错误与失败判定

HTTP 成功也可能包含业务错误。提交时同时检查 HTTP 状态、业务 `code` 和 `result`：

| code | 含义与处理 |
| --- | --- |
| `1` | 通常为提交成功；取得非空字符串任务 ID 后进入轮询 |
| `21` / `22` | 文档约定为任务已存在 / 排队中；有任务 ID 才进入查询，后续可能仍需交互 |
| `23` / `30` | 文档约定为队列已满 / 分组负载饱和；向调用方返回繁忙状态 |
| `24` | 文档约定为提示词可能含敏感词；检查输入并返回明确错误 |
| `4` | 参数校验或网关处理错误；检查请求字段和 `description` |
| `5` | 本次曾与 `do_request_failed` 一起出现；按实际 HTTP 和错误信息记录上游失败 |
| 其他值，或没有合法任务 ID | 保留响应并返回提交失败，不能假装已经生成 |

`description` 不固定，本次成功提交还可能由网关处理排队状态；调用方不应通过文案字符串判断成功。

轮询中的失败以 `status` 和 `failReason` 为准。本次多图 Uguu 任务即使提交返回 `code=1`，仍在后续查询时以 `FAILURE / Prompt图片未通过审核` 结束。`progress=100%` 也不能单独作为成功依据。

## 7. 已验证范围与复用工具

| 能力 | 实测情况 |
| --- | --- |
| 纯文本、v8.1、FAST | 成功，约 70 秒完成并下载 |
| 单张较小 JPEG Base64 | 默认方案，已成功，约 60 秒 |
| 多张 Base64 | 官方文档支持；本次尚未验证真实生成结果 |
| 单图 CDN URL、`--iw 1.5` | 成功，约 67 秒 |
| 两张 CDN URL、`--iw 1.5` | 成功，约 67 秒 |
| 单张 URL 风格参考 `--sref` | 成功，约 90 秒 |
| Uguu 单图 URL、原始大 PNG | 存在成功样例，也出现过审核失败 |
| Uguu 双图 URL | 两次失败，尚未验证通过 |

以上耗时包含轮询等待和结果下载，不是性能保证。参考图用于引导生成，本次仍观察到脸型、姿势、颜色和纹理变化，没有验证精确身份还原或逐像素编辑能力。

仓库提供 `scripts/midjourney_smoke.py` 自动完成编码、提交、轮询和下载。以下命令使用相同的 `MIDJOURNEY_API_KEY` 环境变量；未设置时通过隐藏输入读取：

```bash
python3 scripts/midjourney_smoke.py \
  --case text --output-dir /tmp/mj-imagine-demo/text

python3 scripts/midjourney_smoke.py \
  --case base64 --reference reference.jpg \
  --output-dir /tmp/mj-imagine-demo/reference

python3 scripts/midjourney_smoke.py \
  --case base64 --reference fox.jpg rabbit.jpg \
  --prompt 'An orange ceramic fox with a blue scarf and a cream ceramic rabbit with a green scarf, sitting side by side on a wooden table --iw 1.5 --ar 3:2 --v 8.1' \
  --output-dir /tmp/mj-imagine-demo/multiple-references
```

以上探针命令会提交生成任务；多图 Base64 命令用于展示接入形式，本次尚未验证其生成结果。已成功的多图 CDN 请求见 3.3。本文整理已有证据，没有新增生成请求。

实测证据：[单图、Base64、风格参考和 Uguu](validation/2026-10-21-midjourney-v81.md)；[多参考图](validation/2026-10-21-midjourney-multi-reference.md)。

官方文档：[概览](https://docs.magic666.top/api-reference/images/midjourney/overview)、[提交](https://docs.magic666.top/api-reference/images/midjourney/submit)、[查询](https://docs.magic666.top/api-reference/images/midjourney/query)、[Uguu API](https://uguu.se/api.html)、[Uguu FAQ](https://uguu.se/faq.html)。
