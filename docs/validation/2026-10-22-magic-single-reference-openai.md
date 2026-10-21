# Magic 单参考图 OpenAI 代理链路验收

日期：2026-10-22（Asia/Shanghai）。对应 [#3](https://github.com/98624017/gemini-worker-rust/issues/3)。

## 真实 Magic 验证

本次未执行真实生成。当前环境的 `MIDJOURNEY_API_KEY`、`UPSTREAM_API_KEY` 均未设置，项目仅有 `.env.example`，没有可用的真实凭据。

经过新增 OpenAI 代理链路的 FAST 中文 v8.1 单图任务，以及其输入处理后 MIME/尺寸、任务终态、实际模式、任务 ID、端到端耗时和结果 URL 下载可用性均为**未验证**。本地模拟测试能验证处理合同，但不能证明真实 Magic 接受载荷；旧的原生单图成功记录也不能替代代理验收。

补验时按 [单参考图调用说明](../magic-reference-openai.md) 发起 `mj_fast_imagine` 请求，使用可访问的 HTTP(S) 参考图 URL 和中文 `--v 8.1` 提示词。记录实际预处理结果、admin 任务诊断和下载结果；凭据和完整图片 Data URI 均不写入报告。

## 临时资源回归

通过现有 `BlobRuntime::store_stream` 公共接口验证写入中断和取消。首条回归测试在原实现中观察到部分 spill 文件残留；修复文件创建和写入期间的清理守卫后，以下测试通过：

- `rtk cargo test --test blob_runtime_test interrupted_blob_stream_removes_partial_spill`：流读取失败后无临时文件残留，不计为成功 spill。
- `rtk cargo test --test blob_runtime_test cancelled_blob_stream_removes_partial_spill`：实际临时文件已创建、输入流仍未结束时取消，文件仍被清理。

## 本地 HTTP 验证

通过真实下游 TCP 入口、本地图片源和模拟 Magic 捕获最终请求，首轮新增 16 项单参考图测试，覆盖：

- JPEG、PNG、WebP 按实际字节识别；2047、2048、2049、横竖图及极端比例，最近整数取短边且至少为 1；合规 JPEG/WebP 字节保留，PNG 强制转 JPEG 98 / 4:4:4、透明白底，WebP 无损和透明通道。
- 三种别名与 RELAX 复用，多张合并结果及私有 `base64Array` 拒绝；损坏、不支持格式、源站 404、20MiB 限制及禁止的网络目标均在提交前失败。
- 活跃共享缓存禁用、未命中及预热 PNG→WebP 命中时均重新下载原始 PNG 并提交 JPEG；原缓存仍保留 WebP，其他渠道的共享缓存不被污染。
- 准备、提交和查询共享绝对期限；真实下游断连、失败和取消清理请求 blob；全局四个准备槽及取消释放；MIME/大小摘要日志和 key / Base64 脱敏，暂时审核失败不扩大违规缓存。

首轮 `rtk cargo test --lib http::router::midjourney_tests -- --nocapture`：52 项通过，包括 FAST、RELAX 和以上 16 项参考图测试。`rtk cargo check --all-targets` 通过；admin 新脱敏测试按红→绿通过。通用编码器另补输入 blob 不可用时的错误和部分输出清理回归，纳入最终全量验证。

MJ 首期绕过缺少原始格式标记的共享图片缓存，不缓存其处理变体；解码受 128MiB 分配上限约束。两项选择用于保证格式合同和有界资源，不影响其他渠道既有处理。

## 审查后的回归修复

Standards 首轮发现两项 P2，均增加 HTTP 回归并先观察失败后修复：

- 长中文 prompt 被完整留在 `requestUpstream`，绕过既有 64KiB 截断。恢复使用同一截断函数，保留 MIME/字节摘要；测试同时确认实际提交的 prompt 没有被截断。
- 图片下载错误只保留 reqwest 外层说明，丢失连接根因并携带源站 URL。现在移除 URL 后复用现有错误链记录；本地图片源断连测试确认根因留在 admin、源站查询 token 和已知 key 不泄漏到错误诊断，下游不暴露内部 detail，也不创建 Magic 任务。

这两项增加后，参考图相关 HTTP 测试共 18 项。

首次 `rtk cargo test` 在库测试中出现 111 项通过、1 项失败：既有断连测试在固定次数的 `yield` 后观察到上游活动请求尚未归零。单独运行和 timing 测试组重复未复现，未发现需要修改生产取消逻辑的证据。测试现先在真实的 3 秒期限内确认 admin `499` 与上游请求归零，再推进虚拟时间检查不会继续查询；原有强断言保留。修后单测重复 100 次、timing 组重复 20 轮通过，无临时调试代码。

## Standards

以 #2 提交 `ecb3207965b9176769bd46fff7fc01fe7920f823` 为基点，独立规范审查累计发现以上两项 P2；两项均已修复并由审查代理复核，新增问题 0 项，未解决 0 项，Fowler smell 0 项。

## Spec

按 #3 正文及其共同规格独立审查：遗漏或部分实现 0 项，范围扩张 0 项，错误行为 0 项。日志修复和断连测试同步增量复核仍为 0 项；真实 Magic 验证局限按 ticket 要求保留。

Standards：2 项 P2 已修复，未解决 0 项；Spec：0 项。

## 最终验证

| 命令 | 结果 |
| --- | --- |
| `rtk cargo test` | 300 项通过，24 个测试套件；包括 FAST、RELAX、参考图、通用编码、Blob、admin 和其他渠道回归 |
| `rtk cargo check --all-targets` | 通过 |
| `rtk cargo fmt --all -- --check` | 通过 |
| `rtk proxy git diff --check` | 通过 |

保留任务开始时的六个已修改文件差异及全部既有未跟踪文件；通过精确差异和文件哈希核对。本次只提交 #2、#3 的实现、测试和调用／验收文档。
