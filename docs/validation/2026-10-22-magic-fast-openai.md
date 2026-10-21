# Magic FAST OpenAI 同步链路验收记录

测试日期：2026-10-22（Asia/Shanghai）。本文只记录本次代理 FAST OpenAI 路径的验收边界，不把旧的 Magic 原生 `/mj/*` 成功记录当作代理链路证据。

## 结论

真实 Magic 代理链路本次未完成验收。当前环境中 `MIDJOURNEY_API_KEY` 与 `UPSTREAM_API_KEY` 均未设置；项目内没有 `.env`、`.env.local` 或 `.env.production`。因此本次没有真实生成任务，不能填写或推断以下字段：任务 ID、实际模式、任务终态、端到端耗时和结果 URL 下载可用性。

这不是“真实生成失败”，而是缺少真实凭据导致未执行。旧的原生 Magic 成功记录不能替代 `POST /v1/images/generations` 经过本代理的验证。

## 本地模拟 HTTP

实现方已提供首条 happy path 的本地模拟 HTTP 验证：

- 目标匹配使用解析后的 host 精确 `magic666.top` 与 model `mj_fast_imagine`。
- 代理向模拟 Magic 发送一次 `POST /mj/submit/imagine`，JSON 为 `mode=FAST` 和规范化 prompt。
- 提交返回任务 ID 后立即查询 `GET /mj/task/{taskId}/fetch`，没有重复创建任务。
- `imageUrls` 的字符串与 `{ "url": ... }` 候选均可读取，保持顺序并去除重复项。
- 最终响应为 OpenAI 风格 `created`、`data[].url` 和兼容 `usage`。

截至本文编写时，首条 happy tracer 与 `cargo check` 的已知状态为通过；最终交付前由主 agent 补入实际命令、精确测试名称和候选树结果。本文不宣称未运行的完整测试套件、错误/取消/期限用例或真实外部生成已经通过；本文件编辑者未运行共享 `cargo` 命令。

## 待补验收项

| 验收项 | 当前状态 |
| --- | --- |
| 真实中文 `mj_fast_imagine` 代理生成 | 未验证，缺少凭据 |
| 任务 ID、实际 `FAST` 模式、终态、耗时 | 未验证 |
| 返回 URL 实际下载 | 未验证 |
| 非目标 Magic 模型、恶意相似域名、Gemini、aiapipro、grsai 和既有 OpenAI 图片行为不回归 | 需由主 agent 补充精确测试结果 |
| 输入拒绝、Magic 业务错误、查询重试、连续失败五次、超时和下游断连不重提 | 需由主 agent 补充精确测试结果 |
| 明确违规错误保持 `451`/现有缓存，其他审核错误不扩大缓存 | 需由主 agent 补充精确测试结果 |

## 真实验收命令模板

凭据只应通过运行环境注入，不要写入报告或仓库：

```bash
export UPSTREAM_BASE_URL="https://magic666.top"
export UPSTREAM_API_KEY="YOUR_API_KEY"
rtk proxy curl --fail-with-body --silent --show-error \
  -X POST 'http://127.0.0.1:8787/v1/images/generations' \
  -H 'Content-Type: application/json' \
  -H "Authorization: Bearer ${UPSTREAM_API_KEY}" \
  --data-raw '{"model":"mj_fast_imagine","prompt":"中文主体，--v 8.1","response_format":"url"}'
```

实际报告必须以代理响应和下载结果为准，至少记录真实任务 ID、提交/查询返回的模式与状态、从代理入口开始的耗时，以及每个返回 URL 的 HTTP 状态和图片下载结果。超时或调用方断连时只记录代理停止查询；不能声称代理取消了 Magic 上游任务。
