# 02 — RELAX 中文文生图选择正确慢速渠道并同步返回

## What to build

调用方把同一个 OpenAI 文生图请求的 model 改为 mj_relax_imagine，即可实际走 Magic 的 RELAX 渠道，仍在一次请求中取得最终图片 URL。复制来的 --fast 或 --turbo 不会覆盖模型选择。

复用前置任务交付的同步任务生命周期、错误及响应合同；不改变 FAST 的行为，也不重新实现一套轮询器。

## Acceptance criteria

- [ ] 仅对 magic666.top 上的 mj_relax_imagine 采用慢速适配；其他 host、其他模型和既有渠道分发不变，配置带 /v1 或尾斜杠时地址仍正确。
- [ ] POST /mj-relax/mj/submit/imagine，JSON mode=RELAX；不能只修改 mode 而仍走 FAST 提交路径。查询统一使用 GET /mj/task/{taskId}/fetch，提交和查询使用现有 Bearer key。
- [ ] 独立的 --fast、--turbo 及重复 --relax 被规范化为恰好一个 --relax。保留中文正文、--v 8.1、--ar 和其他非模式参数，不翻译、不全局替换破坏正文。
- [ ] 不传参考图或传空数组时，上游不带 base64Array。参考图能力尚未交付时，非空参考图明确拒绝，不静默忽略；本 ticket 不以参考图 ticket 为前提。
- [ ] 输入校验、最多一次提交、状态判定、查询重试、默认 600 秒全链路预算、取消处理、OpenAI 候选 URL 响应及错误映射与已经交付的 FAST 合同一致；慢速等待仍有界，不改变 aiapipro 的超时规则。
- [ ] admin 记录实际发送的 RELAX 模式和任务诊断，key 脱敏。不会把请求中写了 mj_relax_imagine 当作已经证明上游使用 RELAX 的证据。
- [ ] 通过下游 HTTP 入口和本地模拟 Magic 验证慢速路径、mode、唯一模式参数、中文保留、等待完成、失败及期限，并验证 FAST 路径未回归。
- [ ] 补充调用方可用的 RELAX 文生图说明；通过新增代理链路实测中文 v8.1 文生图，核对任务终态、实际 --relax 且无 --fast、任务 ID、耗时和结果 URL 可下载性。若无法实测，明确记录未验证，不用旧原生调用记录替代。

## Blocked by

- [#1 — FAST 中文文生图通过 OpenAI 接口同步返回](https://github.com/98624017/gemini-worker-rust/issues/1)。
