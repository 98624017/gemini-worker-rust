# Magic FAST OpenAI 同步链路验收记录

测试日期：2026-10-22（Asia/Shanghai）。本文只记录本次代理 FAST OpenAI 路径的验收边界，不把旧的 Magic 原生 `/mj/*` 成功记录当作代理链路证据。

## 结论

真实 Magic 代理链路本次未完成验收。当前环境中 `MIDJOURNEY_API_KEY` 与 `UPSTREAM_API_KEY` 均未设置；项目内没有 `.env`、`.env.local` 或 `.env.production`。因此本次没有真实生成任务，不能填写或推断以下字段：任务 ID、实际模式、任务终态、端到端耗时和结果 URL 下载可用性。

这不是“真实生成失败”，而是缺少真实凭据导致未执行。旧的原生 Magic 成功记录不能替代 `POST /v1/images/generations` 经过本代理的验证。

## 本地模拟 HTTP

新增测试位于 `src/http/router/midjourney_tests.rs` 及其三个子模块，共 31 个测试。它们启动真实下游 TCP 服务，通过已注入的 HTTP 客户端将逻辑域名 `magic666.top` 映射到本地模拟服务；没有添加产品测试路由或全局环境变量。

覆盖范围：

- 精确 host/model 分流、上游路径和端口、请求头覆盖、空参考图别名及强制 b64 配置隔离；非目标 Magic、其他渠道和 Gemini 保持原协议。
- 中文 v8.1 提示词、冲突模式清理、一次提交、立即查询、空/未知状态等待、合法成功终态和候选 URL 严格校验。
- 业务码、鉴权、限流、失败/取消/MODAL、损坏响应、五次连续查询失败与成功清零；提交中断不重提。
- 同一绝对预算覆盖请求体读取、提交、慢查询和轮询等待；缺省 600 秒；读取停滞也会返回 504，且不创建任务。
- 真实 TCP 断连后不再轮询，在途查询释放，admin 保存 499 取消诊断；已知密钥在错误和 admin 中脱敏。
- 候选顺序/去重、主图回退、毫秒时间戳、兼容 usage、直链代理前缀，以及既有违规缓存与普通审核失败的区别。

验证命令及结果：

| 命令 | 结果 |
| --- | --- |
| `rtk cargo check --all-targets` | 通过 |
| `rtk cargo fmt --all -- --check` | 通过 |
| `rtk cargo test` | 当前工作区 273 个测试通过，24 个测试套件；包含以上 31 个新增 HTTP 测试及既有 aiapipro、grsai、Gemini/OpenAI 回归 |
| `rtk proxy git diff --check` | 通过 |

验证保留了工作区已有未提交改动。等待和截止时间采用 Tokio 虚拟时间，真实网络连接用于请求转发与断连；没有真实等待数分钟。

## Standards

审查基点为任务开始前的 `ce801055480768780447a2a0c1be3751b6fb0bab`，只审查本任务提交的改动。首轮发现一项 P2：Magic 网络错误只保存友好分类，admin 缺少底层根因链，不符合诊断日志要求。现已复用 `reqwest_error_detail`，移除请求 URL 并对已知 key 脱敏，仅写入 admin 诊断；`interrupted_submission_is_not_replayed` 通过真实 HTTP 日志接口验证根因保留、下游响应无原始 detail，且仍只提交一次。

未发现需要处理的 Fowler code smell；既有大路由只作局部接入，协议和任务执行分别由新模块负责。

## Spec

按 GitHub [#1](https://github.com/98624017/gemini-worker-rust/issues/1) 核对，未发现 FAST 文生图实现缺口、错误行为或范围扩张。真实 Magic 生成因缺少凭据未执行，按 ticket 要求明确保留验证局限，未引用旧原生协议结果代替。

Standards：1 项 P2 已修复，未解决 0 项；Spec：0 项。

## 待补验收项

| 验收项 | 当前状态 |
| --- | --- |
| 真实中文 `mj_fast_imagine` 代理生成 | 未验证，缺少凭据 |
| 任务 ID、实际 `FAST` 模式、终态、耗时 | 未验证 |
| 返回 URL 实际下载 | 未验证 |
| 非目标 Magic 模型、恶意相似域名、Gemini、aiapipro、grsai 和既有 OpenAI 图片行为不回归 | 本地测试通过 |
| 输入拒绝、Magic 业务错误、查询重试、连续失败五次、超时和下游断连不重提 | 本地测试通过 |
| 明确违规错误保持 `451`/现有缓存，其他审核错误不扩大缓存 | 本地测试通过 |

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
