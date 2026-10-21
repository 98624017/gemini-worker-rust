# Magic RELAX OpenAI 代理链路验收

日期：2026-10-22（Asia/Shanghai）。对应 [#2](https://github.com/98624017/gemini-worker-rust/issues/2)。

## 真实 Magic 验证

本次未执行真实生成。当前环境的 `MIDJOURNEY_API_KEY`、`UPSTREAM_API_KEY` 均未设置，项目仅有 `.env.example`，没有可用的真实凭据。

因此，经过新增 `POST /v1/images/generations` 代理链路的中文 v8.1 RELAX 任务终态、实际 `--relax` 且无 `--fast`、任务 ID、端到端耗时及结果 URL 下载可用性均为**未验证**。旧的 Magic 原生接口成功记录不能替代本次代理验收。

补验时使用 [RELAX 调用说明](../magic-relax-openai.md) 的请求，经代理发送中文 `--v 8.1` 提示词；结合 admin 中实际提交的慢速路径、模式、提示词和任务诊断，核对终态并下载返回的每个 URL。凭据通过运行环境注入，不写入报告。

## 本地 HTTP 验证

复用现有真实 TCP 测试边界，把逻辑主机 `magic666.top` 映射到本地 Magic 模拟服务，从下游图片入口观察最终响应并捕获实际提交。

- RELAX 慢速提交路径、`mode=RELAX`、唯一 `--relax`、中文及 `--v 8.1` 保留，提交与通用查询使用相同 Bearer key。
- 四种上游路径后缀、空图片别名、强制 b64 配置隔离和精确 host/model 分发，FAST 及非目标渠道行为保持。
- 非终态等待、最终候选响应、任务失败/取消/MODAL、一次提交、共享绝对期限、输入拒绝，以及实际 admin 请求、模式和密钥脱敏。

验证结果：`rtk cargo test midjourney_tests --lib` 的 36 项测试全部通过；`rtk cargo check --all-targets` 通过。类型检查时只有 #3 尚未接入的编码 helper 存在暂时 `dead_code` 提示。本票修改文件的格式检查及差异检查通过。最终整合后的全量验证另见单参考图验收记录。

## Standards

以任务起点 `9fe5f9bddc0e4dd5a74098746be5d239b76fd6ae` 为基点，对固定实现快照进行独立规范审查。明确违规 0 项，值得报告的 Fowler smell 0 项。模式、路径和参数由 `Mode` 集中管理，RELAX 复用既有生命周期，未引入无关重构。

## Spec

独立按 #2 正文核对：缺失或部分实现 0 项，范围扩张 0 项，错误行为 0 项。慢速路径、实际模式和唯一参数均有 HTTP 捕获验证；非空参考图在 #3 接入前明确拒绝。真实凭据缺失的限制已按 ticket 要求记录。

Standards：0 项；Spec：0 项。
