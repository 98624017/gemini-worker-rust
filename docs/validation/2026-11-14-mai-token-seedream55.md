# MAI Token Seedream 5.5 适配验证

日期：2026-11-14（Asia/Shanghai）。本次修改和验证位于本地工作区，目标上游为 `https://api.mai-token.com`。生产入口 `gx.xinbao-ai.com` 未在本次测试中部署或重放请求。测试密钥通过终端隐藏输入载入进程内存，不保存到源码、样例或本记录；结果 URL 的 query 和 fragment 不写入验证记录。

## 客户端合同

AI Work 的请求构造见 `/root/projects/AIwork_Personal/src/shared/xinbao-request-contract.ts:239`，实际提交见 `src/main/services/xinbao-api.ts:1414`。Seedream 5.5 使用精确模型名 `seedream-5.5`，调用 `POST /v1/images/generations`，发送 `prompt`、`response_format:"url"`、`n:1`、`size`、`aspect_ratio`、`watermark:false` 和可选 `image: string[]`。

本次保持这些字段和本层入口兼容，不改 AI Work 代码。调用方使用既有 `https://api.mai-token.com|YOUR_API_KEY` 上游覆盖格式或服务配置选择新渠道。代理最多接受 10 张参考图；AI Work 当前自己的请求校验仍最多接受 6 张，本次没有调整其界面或校验规则。

AI Work 的实际提交地址由 `src/main/services/service-base-urls.ts:2` 定义为 `https://async.xinbao-ai.com`，提交后解析任务 ID，再通过 `/v1/tasks/batch-get` 轮询。其约 80 秒的非幂等提交超时保护的是异步服务受理阶段，不能据此推断整个生成任务只能执行 80 秒。本次未修改客户端超时，也未验证线上异步服务到 `gx` 的内部配置；接入时需在异步服务中选择本层渠道，并确认该段请求超时覆盖实际出图时间。

只有精确上游 host `api.mai-token.com` 和模型 `seedream-5.5` 同时匹配时启用新适配。其他模型、渠道及 Gemini 请求保持原有分发。

## 上游协议与实测修正

最初测试标准图片入口时，`image-v1` 请求携带 `seed:0` 得到 HTTP 503：

```json
{"error":{"message":"field seed is not part of image-v1","type":"invalid_request","param":"","code":"invalid_request"}}
```

该响应没有任务 ID。最终实现统一采用用户文档中的兼容异步图片接口：`POST /v1/videos` 创建，`GET /v1/videos/{taskId}` 查询，完成后读取 `metadata.url`。不将旧参数混入标准 `image-v1` 请求，也不在两种接口之间自动重建任务。

映射规则为 `size=1K/2K` → 固定 `quality:"1440p"`，`aspect_ratio` 保留；参考图别名按 `reference_images`、`images`、`image` 合并为 `image_urls`；整数 `seed` 原样保留，包括 `0`。`4K`、不支持的比例、`n!=1` 及超限参考图在创建前返回 400。

第一次兼容接口创建成功，已返回任务 ID。立即进行的首次 GET 出现未知状态，适配器返回 502 并携带原任务 ID；之后直接查询同一任务可见 `queued` 和 `in_progress`。没有重新创建该任务。最终实现按上游文档在创建后等待 5 秒再首次查询，后续默认每 5 秒查询；未知状态仍报协议错误，并保留实际状态值，不推测任意未知状态为处理中。

## 本地验证

- `rtk cargo test --lib --test mai_token_test --test provider_test`：153 项通过，包含 10 项新渠道 HTTP 集成测试。
- `rtk cargo build`：通过。
- `rtk cargo fmt --check`、`rtk git diff --check`：通过。

HTTP 测试覆盖完整轮询、`progress=100` 仍未完成、参考图合并顺序、`seed:0`、10/11 张边界、精确 host/model 分发、旧 Gemini/OpenAI 行为、创建错误不重试、查询 429 仅重试已有任务、损坏响应、失败任务、已知任务 ID 保留、总 deadline 和输出 URL 的专用代理前缀。

本层成功响应复用既有 `created`、`data:[{url}]`、`usage` 构造函数。`usage` 仍为固定兼容值，不代表上游真实 token 用量或按张计费金额。提交、响应读取、查询和等待受同一 `UPSTREAM_TIMEOUT_MS` 截止时间约束；创建不自动重试。

## 真实生成与产物

真实测试固定 `size:"2K"`、`n:1`、`response_format:"url"`、`watermark:false` 和 `seed:0`。文生图请求比例为 `1:1`；双参考图融合请求比例为 `4:5`，素材为红色运动鞋和黑色耳机的公网 JPEG URL。测试下载文件时不携带 API key。

| 用例 | 本层生成 HTTP | 取得结果的方式 | 下载 HTTP | 原图尺寸 |
| --- | --- | --- | --- | --- |
| 文生图 `1:1` | 初版首次查询返回 502，并保留任务 ID | 查询原任务，最终 `completed`；未重新生成 | 200 | 2048 × 2048，PNG |
| 双参考图 `4:5` | 最终构建返回 200 | 本层提交、等待和轮询，返回统一图片响应 | 200 | 1856 × 2320，PNG |

文生图任务 ID 为 `task_z3t2qPzyTJ25kOeTQSsrUGlPNVIl0jod`，上游完成态的 `created_at` 与 `completed_at` 相差 429 秒。双参考图任务 ID 为 `task_nSoXFPuEHifVcZkw6HOu0j3S0oSG9lok`，本地生成请求耗时 269.50 秒，不含图片下载。此为各一次请求的观察值，不是延迟承诺。

双参考图请求返回且核对了 `created`、单元素 `data[].url` 和既有固定 `usage`，没有向下游暴露上游的 `object:"video"` 任务对象。原图下载未携带鉴权头；两张生成图均由 Pillow 解码确认尺寸，未缩放或裁切。`4:5` 原图宽高比精确为 0.8。

目视检查：文生图呈现浅蓝色陶瓷茶壶与白色大理石台面；双参考图结果同时呈现红色运动鞋、白色钩形标志和黑色耳机，背景变为浅灰与米白展示台。运动鞋左端部分出画，本次未做商品外观严格一致性评估。

本次共发送一次被标准入口拒绝的创建请求，以及两次兼容入口创建请求；两项已受理任务均取得图片，未重新提交已有任务。最终构建的真实完整链路验证使用双参考图请求；文生图复用原任务的完成结果，未为验证最终构建额外生成同一张图片。其他比例通过文档白名单及本地测试验证，没有逐一提交真实计费生成。

本地产物位于权限为 `0700` 的 `/tmp/mai-token-seedream55-UtmXbI/`：

- [文生图原图](/tmp/mai-token-seedream55-UtmXbI/text-1x1.png)
- [双参考图融合原图](/tmp/mai-token-seedream55-UtmXbI/references-4x5.png)
- [最终构建脱敏请求和统一响应](/tmp/mai-token-seedream55-UtmXbI/results.json)
- [文生图原任务查询记录](/tmp/mai-token-seedream55-UtmXbI/recovered-task.json)
- [标准入口字段拒绝记录](/tmp/mai-token-seedream55-UtmXbI/image-v1-seed-rejection.json)
- [初版首次查询错误记录](/tmp/mai-token-seedream55-UtmXbI/initial-poll-rejection.json)

原图 SHA-256：

```text
text-1x1.png       a9342bbd079ebe72c8f48588e3ab7870be03a144cb6b8b449946d8f86a404329
references-4x5.png 9a0b1ddc59d8a3241fa2efd78b9e30bae063e1b7c37d4b3f6d3ac8af19b914e8
```

本次未提交、推送或部署，也未在 AI Work 中运行真实线上 E2E。验证范围为本地构建到真实 MAI 上游及结果文件下载。
