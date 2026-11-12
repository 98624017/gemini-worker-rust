# 01 — FAST 中文文生图通过 OpenAI 接口同步返回

## What to build

调用方使用 POST /v1/images/generations，提交 mj_fast_imagine、中文 prompt 和 response_format=url，不传参考图即可在同一个 HTTP 请求中等待 Magic 完成，并取得 OpenAI 风格图片 URL 响应。代理承担一次提交、查询、超时、错误转换和诊断记录。

本任务交付完整的 FAST 文生图路径；RELAX 和参考图由后续 tickets 交付。先保留既有未提交修改，按现有模块职责局部扩展，不先做通用 provider 重构。

## Acceptance criteria

- [ ] 仅当解析后的上游 host 精确为 magic666.top 且 model 为 mj_fast_imagine 时启用此路径；相似恶意域名、Magic 的其他模型和其他渠道不误命中。带 /v1 或尾斜杠的上游配置也能构造正确的提交和查询地址，保留现有 scheme、port 和鉴权覆盖语义。
- [ ] model、prompt 为非空字符串；response_format 省略时为 url，仅支持现有大小写兼容的 url。非法类型、null、b64_json 以及未支持的 n、size、quality、stream 在创建任务前返回清晰 400，不受通用强制 b64_json 配置改写。
- [ ] image、images、reference_images 省略或空数组时均可文生图；非空参考图在参考图 ticket 完成前明确拒绝，不丢图执行文生图。非法图片字段不创建任务。
- [ ] 提交 POST /mj/submit/imagine，JSON 仅含 mode=FAST 和 prompt；中文正文、--v 8.1、--ar 等参数保留，不翻译。移除独立的冲突及重复模式参数，最终仅有一个 --fast，不破坏正文中非参数文本。
- [ ] 提交和查询使用现有解析出的 Bearer key。每次下游请求最多创建一个任务，提交失败、超时或连接中断均不自动重新创建；HTTP 成功但业务失败不视为已生成。
- [ ] 仅在提交业务 code 为 1、21 或 22 且 result 是非空字符串任务 ID 时查询 GET /mj/task/{taskId}/fetch。缺少任务 ID、非 JSON 或响应结构损坏明确失败。
- [ ] 创建后立即查询，未完成时默认每 10 秒查询。空状态、NOT_START、SUBMITTED、IN_PROGRESS 和未知非终态继续等待；仅 SUCCESS 且有合法结果 URL 才成功，progress=100% 不作为成功依据。失败、取消和 MODAL 明确终止并保留原因。
- [ ] 全链路从请求开始受现有上游超时配置的绝对截止时间约束，默认 600 秒；每次网络操作和等待都服从剩余预算，不移植 aiapipro 的固定 450 秒上限。超时返回 504，下游取消后停止本请求轮询并释放资源，不承诺取消上游任务。
- [ ] 仅查询的暂时性传输错误和现有可重试 HTTP 状态可重试；连续失败五次停止，成功查询后清零。认证错误、不可重试 HTTP 状态和损坏响应立即停止，不因查询失败重提任务。
- [ ] 成功返回 HTTP 200、application/json 和 created、data[].url、现有兼容 usage。优先返回 imageUrls 全部候选，兼容字符串和含 url 对象，保留顺序并去除完全重复项；列表缺失或为空时才回退 imageUrl，非空但损坏的列表或非法 HTTP(S) URL 返回协议错误，不返回空 data。
- [ ] created 优先使用正整数 finishTime，其次 submitTime，并把毫秒转为 Unix 秒；均不可用时用本层时间。保留现有直链代理前缀策略，不强制上传结果；usage 沿用 input_tokens=1024、output_tokens=1024、total_tokens=2048 及现有细分结构，并在文档说明它是兼容占位统计。
- [ ] 错误沿用既有 error 包装及 source、stage、kind：输入或明确策略拒绝为 400，参考图之外的一般上游协议或任务失败为 502，无可用实例、队列满或分组饱和为 503；保留 401、403、429 语义。错误保留必要的业务 code、failReason、阶段和任务诊断，不泄露 key。
- [ ] admin 记录能追踪实际发送模式、任务 ID、阶段和耗时，密钥脱敏；保留既有明确违规错误分类，不把泛化的“Prompt图片未通过审核”新增为长期缓存规则。
- [ ] 通过真实下游 HTTP 入口和本地模拟 Magic 验证请求、状态推进、最终响应、错误、取消、期限及不重复提交；用受控状态或虚拟时间，避免真实等待数分钟。验证 Magic 非目标模型、Gemini、aiapipro、grsai 和既有 OpenAI 图片行为不回归。
- [ ] 补充调用方可用的 FAST 文生图说明；实现后经新增代理链路进行中文 v8.1 文生图实测，记录终态、实际模式、任务 ID、耗时和结果 URL 可下载性。若无法实测，明确记录未验证，不能用旧原生协议记录代替。

## Blocked by

- None — can start immediately。
