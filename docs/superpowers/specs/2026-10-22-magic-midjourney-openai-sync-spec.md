# Magic Midjourney 的 OpenAI Images 同步适配

日期：2026-10-22。状态：已据此批准拆票并发布；功能实现和验收尚未完成。

Issue tracker：GitHub，98624017/gemini-worker-rust。Triage label：ready-for-agent。本文作为拆票依据保留，已发布四个实现 issues，详见末尾发布状态；未另建 parent issue。

## Problem Statement

下游使用 OpenAI 风格的图片生成接口，希望只提交一次请求，等待后直接取得图片 URL。magic 的 Midjourney 则要求调用方构造私有请求体、使用 Base64 参考图、创建任务，再持续查询任务状态。当前 Rust 代理没有承接这套转换，导致调用方需要分别实现图片下载、处理、编码、模型模式选择和异步轮询。

现有 aiapipro 渠道已经在代理层完成异步转同步。新增 MJ 能力应沿用这一使用方式：下游继续调用 POST /v1/images/generations，由本层承担 magic 的异步协议，并将最终结果归一化为 OpenAI 图片响应。

前序实测还发现：只设置 mode=RELAX 仍可能执行 FAST；Uguu 多图 URL 曾连续失败；mj_fast_upscale 的选图成功并不代表图片分辨率增加。下游不应承担这些渠道差异。

## Solution

在现有 OpenAI 图片入口增加 magic MJ 适配，支持 mj_fast_imagine 和 mj_relax_imagine。请求使用 model、prompt、response_format 和可选的 image URL 数组。省略 image 或传空数组时执行文生图，传入图片时执行参考图生图。

代理下载参考图，按最长边 2048 像素及格式规则处理，编码为带 MIME 前缀的 Base64 Data URI，再提交至 magic。代理在同一次下游 HTTP 请求内查询任务，成功后返回 created、data[].url 和项目已有的兼容 usage 字段；失败则返回现有错误结构。response_format 仅支持 url。

本版草案默认返回 MJ 提供的全部候选单图，实测通常为四张；没有候选单图列表时才回退主图。下游无需取得任务 ID 后自行轮询，也无需把参考图先上传至 Uguu。

## User Stories

1. As an API consumer, I want to call POST /v1/images/generations for MJ images, so that I can reuse my existing image client.
2. As an API consumer, I want to select mj_fast_imagine, so that I can request FAST generation explicitly.
3. As an API consumer, I want to select mj_relax_imagine, so that I can request RELAX generation explicitly.
4. As an API consumer, I want to submit Chinese prompts directly, so that I do not need a translation step.
5. As an API consumer, I want to omit image for text-to-image requests, so that a reference image is optional.
6. As an API consumer, I want to supply one reference URL, so that the generated image can follow a visual reference.
7. As an API consumer, I want to supply multiple reference URLs, so that one generation can use several references.
8. As an API consumer, I want reference order to be preserved, so that the proxy does not change my input ordering.
9. As an API consumer, I want the proxy to download and encode reference images, so that I do not manage Base64 payloads or temporary public uploads.
10. As an API consumer, I want oversized reference images resized proportionally, so that input dimensions are bounded without distorting their subjects.
11. As an API consumer, I want JPEG and WebP formats preserved when possible, so that the proxy avoids unnecessary format conversion.
12. As an API consumer, I want PNG references converted with the agreed JPEG quality, so that preprocessing is consistent.
13. As an API consumer, I want my selected model to determine FAST or RELAX mode, so that a copied conflicting prompt flag does not select the wrong mode.
14. As an API consumer, I want my aspect ratio and version parameters preserved, so that the adapter respects my generation instructions.
15. As an API consumer, I want one HTTP request to wait for the completed generation, so that I do not implement MJ polling.
16. As an API consumer, I want final candidate URLs in data[].url, so that I can display or download the results normally.
17. As an API consumer, I want malformed requests rejected before task creation, so that avoidable errors do not create paid tasks.
18. As an API consumer, I want reference download failures reported before task creation, so that a missing image does not silently become text-to-image generation.
19. As an API consumer, I want task failures and timeouts reported clearly, so that a successful submission is not mistaken for a completed image.
20. As an operator, I want task submission to happen at most once per downstream request, so that polling failures do not create duplicate generations.
21. As an operator, I want bounded polling, image work and temporary resource usage, so that MJ requests do not exhaust the proxy.
22. As an operator, I want useful stage and task diagnostics without keys or image Base64 in logs, so that I can troubleshoot safely.
23. As an existing client, I want other magic models and other providers to retain their behavior, so that introducing MJ does not change unrelated requests.
24. As a maintainer, I want tests through the existing HTTP integration boundary, so that the feature is verified from the downstream client's perspective.

## Implementation Decisions

### 1. 适用范围与 provider 选择

- 仅在解析后的上游 host 为 magic666.top，且下游 model 精确为 mj_fast_imagine 或 mj_relax_imagine 时进入本适配。
- 扩展现有 provider 的 OpenAI 图片能力选择，使其能结合上游 host 和模型识别 MJ。magic 的其他模型、Gemini 入口、其他 provider 和未知上游保持现有分发语义。
- 不依赖整个 URL 字符串做包含匹配；magic666.top.evil.example 等主机不能命中。保留现有上游地址解析的 scheme、host、port、鉴权覆盖规则。
- 基于解析后的上游 origin 构造确定的 MJ 路径，避免配置地址带 /v1 或尾斜杠时拼出重复路径。支持真实 magic 逻辑主机经现有测试 HTTP 代理映射到本地模拟服务。

### 2. 下游请求合同

| 字段 | 合同 |
| --- | --- |
| model | 必填、非空字符串；本适配支持两个已列明的 Imagine 模型 |
| prompt | 必填、非空字符串；中文直接发送，不增加翻译或语义改写步骤 |
| response_format | 仅 url；省略时沿用现有默认 url，大小写兼容沿用现有行为；b64_json、null 和其他类型返回 400 |
| image | 可选的 HTTP(S) 图片 URL 字符串数组；省略或空数组为文生图；null、标量、数组中的非字符串或无效 URL 返回 400 |
| images、reference_images | 保留现有参考图别名与合并顺序，合并后统一进入本适配 |

- 沿用现有 OpenAI 图片入口最多六张参考图的本层限制，按别名合并后的数量检查。该限制不代表 magic 已实测支持六张。
- image 中不接受 Data URI 或原始 Base64；本层负责从 URL 下载并编码。上游私有 base64Array 不作为新下游字段开放。
- 本期生成参数由 prompt 内的 MJ 参数控制，不增加 n、size、quality、stream 等字段到 MJ 的映射。MJ 请求携带这些未支持的生成控制字段时返回明确 400，不静默宣称已生效。
- 本适配的 response_format=url 不受通用“对部分上游强制 b64_json”的配置改写影响。必须在这一改写前识别并校验 MJ 请求。

### 3. 模型与 magic 请求映射

| 下游模型 | 上游提交路径 | mode | 最终模式参数 |
| --- | --- | --- | --- |
| mj_fast_imagine | POST /mj/submit/imagine | FAST | --fast |
| mj_relax_imagine | POST /mj-relax/mj/submit/imagine | RELAX | --relax |

- 下游 model 是模式选择的唯一责任方。识别独立的 --fast、--relax、--turbo 参数，移除冲突和重复的模式参数，再追加恰好一个与 model 匹配的参数。
- 不通过全局字符串替换破坏提示词正文。保留中文描述、图片权重、宽高比和版本参数；下游传入的 --v 8.1 保持有效。版本继续由 prompt 控制，本期不引入独立 version 字段或擅自更换用户指定版本。
- 上游 JSON 仅构造 mode、规范化后的 prompt，以及有参考图时的 base64Array。文生图省略 base64Array。不转发 model、response_format、image 别名等下游包装字段。
- 所有上游提交和查询均使用现有解析得到的 API key，按 Bearer 认证发送。参考图下载不携带该上游 key。
- 查询使用 GET /mj/task/{taskId}/fetch；RELAX 也可用这一通用查询入口。
- 不仅依赖 mode=RELAX 判断慢速已生效；本次慢速路径与显式 --relax 的组合已有真实成功样例。

### 4. 参考图下载和预处理

原始格式按实际可解码图片字节识别，不以 URL 后缀作为唯一判断依据。首期复用现有 JPEG、PNG、WebP 编解码能力；其他无法解码或不支持的格式明确报错，不引入额外格式依赖。

| 原始格式 | 最长边不超过 2048 | 最长边超过 2048 |
| --- | --- | --- |
| JPEG / JPG | 保留原始字节和格式，不进行额外重编码 | 等比缩小至最长边 2048，仍编码为 JPEG；需要重新编码时使用质量 98、4:4:4 |
| WebP | 保留原始字节和格式，不进行额外重编码 | 等比缩小至最长边 2048，仍为 WebP，使用现有无损 WebP 编码能力，保留透明通道 |
| PNG | 转为 JPEG，质量 98、4:4:4 | 等比缩小至最长边 2048，再转为 JPEG，质量 98、4:4:4 |

- 不放大小图，不裁切、不拉伸；取整后任意一边至少为 1，最长边不得超过 2048。横图、竖图和极端宽高比遵循同一规则。
- 用户所写“jpg，0.98 质量443色彩”在本草案中解释为 JPEG quality=98、YCbCr 4:4:4。4:4:3 不作为本次目标采样格式。
- JPEG / WebP“仅改尺寸”指保留原格式，尺寸已合规时保留原始字节；实际缩小时需要重新编码，不能承诺字节级保留。PNG 转 JPEG 时，透明区域按本版默认合成白色背景。
- 长边、JPEG 质量和采样方式归 MJ 图片准备模块的命名常量统一管理，不散落在路由代码中，也不受通用响应压缩开关影响。
- 下载沿用现有图片客户端、网络目标约束、超时和每张 20MiB 下载限制；保留实际 MIME、尺寸和处理失败阶段的信息。
- 在全部参考图准备成功后才创建上游任务。任意一张失败则整次请求失败，禁止丢图继续生成。
- 可对重复 URL 去重下载，但按下游归一化后的顺序保留每个数组项及重复项；不能因并发完成先后改变参考图顺序。
- 逐图编码成完整 Data URI，使用最终实际 MIME，如 image/jpeg 或 image/webp；上游 base64Array 不包含图床 URL。

### 5. 共享下载缓存与大载荷

- 复用现有下载、并发控制和 Blob Runtime 能力。图片变换放入阻塞任务执行边界，不能在异步运行时线程上执行长时间编解码。
- 现有请求图片缓存可能已经把大 PNG 转成 WebP，不能直接把这种缓存值当作未经处理的原始格式，从而跳过本次 PNG 转 JPEG 规则。
- MJ 必须取得原始载荷，或使用具备明确处理版本标记的专用缓存变体。若现有缓存无法保证这一点，首期可绕过不兼容的缓存变体；不得污染其他渠道的 URL 缓存。缓存命中与禁用时必须得到相同的 MJ 格式和尺寸合同。
- Base64 编码与请求体构造沿用大载荷预算和临时存储原则，避免同时保留多份完整原始图、Base64 和巨大 JSON。现有 Gemini 编码路径只识别其专用图片结构，不能假定直接接受 MJ 字符串数组；仅扩展必要的编码能力。
- 本次强制尺寸和格式要求不受“重编码后更大就返回原图”的既有压缩策略影响。请求结束、失败或取消时清理请求级临时资源。

### 6. 异步转同步与重试

1. 校验下游请求并准备全部参考图。
2. 提交一次 Imagine 请求，同时检查 HTTP 和 MJ 业务 code。
3. 仅在 code 为 1、21 或 22，且 result 为非空字符串任务 ID 时进入查询。
4. 先查询一次任务；尚未结束时按 10 秒间隔继续查询。
5. 只有 status=SUCCESS 并有合法结果 URL 时返回 OpenAI 图片成功响应。

| MJ 状态 | 本层处理 |
| --- | --- |
| 空字符串、NOT_START、SUBMITTED、IN_PROGRESS | 继续查询 |
| SUCCESS | 提取并校验结果，构造下游响应 |
| FAILURE、FAILED、CANCEL、CANCELLED | 终止查询，返回任务失败信息 |
| MODAL | 返回当前同步能力不支持额外交互的明确错误，不挂起等待用户补充 |
| 其他非终态值 | 保留原始状态并受统一等待期限约束，不能按成功处理 |

- progress=100% 和提交 code=1 均不能替代成功终态判断。
- 从本次请求进入开始使用绝对截止时间，图片准备、提交和轮询共同受现有上游超时配置约束，默认 600 秒；每个网络操作受剩余预算约束，不允许一次轮询把总耗时再延长一个完整上游超时。
- 不把 aiapipro 的固定 450 秒等待上限直接移植到 MJ；不改变现有 aiapipro 的轮询行为。
- 仅对查询操作的暂时性传输失败及现有可重试 HTTP 状态重试，连续失败上限沿用五次，成功查询后清零；认证错误、不可重试响应和结构损坏不盲目重试。
- 创建任务最多尝试一次；取得任务 ID 后不因查询错误重新创建。提交超时或连接中断但无法确认是否创建成功时，也不自动重提。
- 下游取消或截止时间到达后停止该请求的轮询，回收请求级资源；不承诺取消已创建的 magic 任务，也不影响共享下载服务自身的生命周期。

### 7. OpenAI 成功响应

- 返回 HTTP 200、application/json。顶层使用 created、data 和项目现有 OpenAI 图片兼容 usage；每个结果为 data[] 中含 url 的对象。
- 优先读取 imageUrls，兼容字符串 URL 和含 url 字段的对象；保留候选顺序并去除完全重复的 URL。默认返回全部候选，实测通常四张，不引入额外 n 控制。
- imageUrls 缺失或为空时，回退合法的 imageUrl。主图可能是四宫格，回退时数量为一；不能把它说成高清候选单图。存在非空但格式损坏的候选列表时返回协议错误，避免静默丢失候选。
- URL 必须是合法 HTTP(S) 图片地址；没有合法结果则按上游协议错误返回，不能成功返回空 data。
- 直接返回上游结果 URL，并沿用现有 OpenAI 直链的代理前缀策略；不强制下载结果再上传 Uguu 或 R2。现有非 MJ 输出处理不变。
- created 优先由正整数 finishTime 的毫秒时间戳转为 Unix 秒，缺失时取 submitTime，仍不可用时回退本层时间。不能把毫秒原值直接暴露为秒。
- usage 沿用项目当前固定兼容值及细分结构，包括 input_tokens=1024、output_tokens=1024、total_tokens=2048；它是既有兼容占位统计，不是 magic 提供的真实 token 使用量或计费依据。
- 不将任务 ID、status、progress、buttons 或上游 Base64 作为成功响应的顶层公开字段。

### 8. 错误、日志和违规响应缓存

- 错误继续使用项目现有 error 包装及 error.message，复用 source、stage、kind 等现有诊断字段，不把 MJ 私有业务响应直接伪装成 OpenAI 成功响应。
- 下游字段校验、参考图格式不支持和解码失败返回明确 400；参考图网络失败、上游协议损坏和一般任务失败按现有代理错误规则返回 502；全链路等待超时返回 504。
- 上游认证失败和 HTTP 限流保留相应 401 / 403 / 429 语义；业务上的无可用实例、队列满、分组饱和归一化为可识别的 503。明确的输入或策略拒绝按现有规则映射为 400。
- 保留上游任务 ID、原始业务 code、failReason、HTTP 状态、阶段及必要耗时供排障；用户错误信息不得只有“失败”而丢失可操作原因。
- 使用现有 admin 记录链路记录下游输入、实际发送的模式和脱敏后上游请求。base64Array 的每一项必须省略图片载荷，只记录 MIME、大小等摘要；现有脱敏只覆盖部分旧图片结构，需要补齐该私有字段。
- 不在日志中记录 API key，不打印完整 Base64、大图片体或无关响应载荷。
- 保持既有上游违规响应缓存机制；仅复用已经明确分类的违规错误。不能把所有“Prompt图片未通过审核”直接新增为长时间缓存命中，因为前序实测存在同图重试成功和更换图源成功的情况。

### 9. 模块职责与兼容范围

- OpenAI 图片入口负责现有输入解析、上游解析、适配分发及统一响应交付。
- MJ 协议模块集中负责模型模式、路径、请求结构、提交响应、任务状态及结果 URL 解析。
- 请求图片准备模块负责下载后的 MJ 尺寸和格式规则；通用图片 I/O 和编码能力继续复用。
- 任务执行模块组织一次提交、受期限约束的查询与错误转换。沿用 aiapipro 的实现经验，不为此强行重构所有异步 provider。
- OpenAI 图片响应、admin、违规缓存和 Blob Runtime 通过现有职责边界接入；不把整个业务协议继续堆进已有大路由函数。
- 保留未提交的既有修改及其他渠道行为。本 spec 仅要求实现本功能所需的局部接口扩展。

## Testing Decisions

### 主测试 seam：现有 OpenAI 图片 HTTP 请求／响应边界

以向 POST /v1/images/generations 发送 HTTP 请求并观察最终响应为唯一主验收边界。图片源与 magic 提交／查询由本地模拟 HTTP 服务承接，使用项目已有的客户端注入或 HTTP 代理测试方式保留真实逻辑主机，避免新增下游专用测试字段、全局环境变量或测试路由。

测试通过模拟服务捕获实际请求、解码上游 Data URI 并检查可见图片属性；不以私有函数调用次数、具体内部类型或实现结构作为通过标准。采用虚拟时间或受控任务状态推进验证轮询，不在测试中真实等待数分钟。

| 场景组 | 必须观察的外部行为 |
| --- | --- |
| FAST 文生图 | 中文描述和 --v 8.1 保留；路径、mode 与单一 --fast 正确；没有 base64Array；未完成前不返回成功 |
| RELAX 文生图 | 慢速路径、RELAX 与单一 --relax 正确；输入含 --fast 或 --turbo 时由 model 规范化为慢速 |
| 单张参考图 | 图片确由本层下载，magic 收到可解码 Data URI，prompt 无需额外加入参考图 URL |
| 多张参考图 | 顺序和重复项保留；任意一张失败时不创建任务；最多六张按现有入口限制校验 |
| 尺寸边界 | 2047、2048、2049 边界及横／竖图、极端比例；不放大小图，最长边不越界，不拉伸 |
| 格式合同 | JPEG / WebP 合规尺寸时字节不变；超限时保持格式；PNG 转 JPEG；JPEG 输出采样为 4:4:4，透明 PNG 白底；输出 MIME 与字节一致 |
| 缓存与大载荷 | 命中、未命中和禁用缓存的图片合同一致；通用 PNG→WebP 优化不绕过 MJ 规则；大载荷和失败路径释放请求级临时资源 |
| 输入校验 | 缺失／空 prompt、非法 response_format、非数组图片、非法 URL、超限数量、未支持生成参数均在提交前明确失败 |
| 提交处理 | code=1/21/22 且有任务 ID 才查询；HTTP 200 业务失败、缺 ID、无可用实例和非 JSON 不能伪装成功 |
| 查询状态 | 初始空状态、处理中、SUCCESS、失败／取消、MODAL、未知状态、100% 失败及无结果 URL 均按合同处理 |
| 重试与期限 | 查询暂时失败可恢复；五次连续失败停止；认证失败立即停止；图片准备及慢网络查询均不能突破总预算；不重复提交 |
| OpenAI 输出 | 字符串／对象候选列表、候选去重与顺序、四宫格回退、毫秒转秒、固定兼容 usage 和直链代理前缀正确 |
| 日志与失败缓存 | Base64 和 key 不泄漏；能追踪任务及失败阶段；既有违规规则工作，临时抓图／审核错误不被扩大缓存 |
| 兼容性 | magic 非 MJ 模型、未知上游、Gemini、aiapipro、grsai 和现有 OpenAI 图片请求保持行为 |

会涉及的测试模块为 OpenAI 图片 HTTP 转发、provider 选择、图片下载／变换及 admin 脱敏。必要的图片格式断言直接检查捕获的最终载荷；仅在 HTTP seam 难以定位编码边界时，补充针对输出字节的窄格式测试，不新增公共产品接口。

Prior art：仓库已有的 aiapipro OpenAI 异步转同步测试、grsai 通过本地 HTTP 代理验证逻辑域名的测试、HappyAPI 参考图下载与缓存测试，以及 JPEG 4:4:4 编码和 OpenAI 响应归一化测试。

实现完成后的真实 magic 验收至少覆盖两个模型各自的中文文生图、中文单参考图和中文双参考图；用用户给出的两张 WebP URL 检查可访问性与处理后 MIME／尺寸，再完成双图任务验证。验证 task 终态、实际模式和返回图片可下载性，记录任务 ID 和耗时。既有原生协议单图成功记录不能替代新增 OpenAI 适配链路验收；多图 Base64 在前序会话中尚未验证成功，必须单独验证或如实记录上游限制。

本次已完成规格整理及拆票发布，未进行功能实现或新增代理链路的计费实测。上述 HTTP 主测试边界和相关验收条件已纳入获批准的四个 tickets，仍须在实现时验证。

## Out of Scope

- Gemini 原生格式接入 MJ、透传公开 /mj 任务接口、SSE 和后台任务队列。
- 下游 b64_json、Data URI 输入、multipart 图片编辑，以及 Base64 与 URL 的下游混合字段。
- Blend、选图、Upscale、高清 2x / 4x、变体、视频、风格参考图的独立处理流程。
- 从 prompt 中解析 URL 或 --sref 并自动下载转换；本期只物化显式参考图数组。
- OpenAI 全量生图参数映射，包括 n、size、quality 到 MJ 的转换。
- 自动将输入上传 Uguu；强制把生成结果下载后上传 R2 或其他图床。
- 新增图片格式依赖、通用 provider 重写、大规模路由拆分、数据库／配置系统迁移。
- magic 内部渠道、审核、任务取消、长期结果 URL 有效期或真实 token 用量保证。

## Further Notes

### 已有实测依据

- FAST v8.1 中文文生图任务 1792596466169744、中文 Base64 参考图任务 1792596565132407 均成功。中文直接受支持，不要求翻译成英文。
- RELAX v8.1 中文文生图任务 1792600328666710、中文 Base64 参考图任务 1792600464002633 均成功；成功组合为慢速入口、RELAX 和显式 --relax。
- 两张 CDN URL 的 Imagine 任务 1792592456977018 成功，但两个 Uguu URL 连续失败。它们验证的是 URL 输入，不能作为双图 Base64 已通过的依据。
- 前序 Base64 单图成功输入为约 39KiB 的 JPEG；约 1.47MiB PNG 曾提交失败。此次 2048 长边和 JPEG 98 预处理是新合同，须经新增链路验证，不能仅凭原生接口已成功而免验。

### 本草案采用的默认行为

这些默认用于补全可实现的合同，仍可在规格评审时调整：

1. 将“443 色彩”解释为 4:4:4，JPEG quality=98；PNG 的透明区域合成白底。
2. 返回全部候选单图，实测通常四张；缺少列表时回退主图，不增加 n 参数。
3. 结果使用上游直链和现有 URL 前缀策略，不强制重传图床；usage 沿用既有固定兼容统计。
4. 单次同步请求默认使用现有 600 秒全链路预算和 10 秒查询间隔；不沿用 aiapipro 固定 450 秒上限。

### 发布状态

用户已批准四项拆分。以下 GitHub Issues 已发布，均为 open 且带 ready-for-agent 标签；正文与本地 ticket 快照一致，依赖同时写入正文并建立 GitHub native issue dependencies。

1. [#1 — FAST 中文文生图](https://github.com/98624017/gemini-worker-rust/issues/1)：无 blockers，可立即开始。
2. [#2 — RELAX 中文文生图](https://github.com/98624017/gemini-worker-rust/issues/2)：blocked by #1。
3. [#3 — 单参考图 URL 下载、预处理和 Base64 提交](https://github.com/98624017/gemini-worker-rust/issues/3)：blocked by #1。
4. [#4 — FAST 与 RELAX 多参考图](https://github.com/98624017/gemini-worker-rust/issues/4)：blocked by #2、#3。

本次发布的 tracker 与标签约定分别记录于 docs/agents/issue-tracker.md 和 docs/agents/triage-labels.md；仅补建实际需要的 ready-for-agent 标签，完整工程技能初始化并未执行。后续领取前以 GitHub 当前状态与 open blockers 为准。
