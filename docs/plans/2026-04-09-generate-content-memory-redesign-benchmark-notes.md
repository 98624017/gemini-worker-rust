# generateContent BlobRuntime Benchmark Notes

日期: 2026-04-09
状态: 已更新

## 观测清单

本轮计划要求至少关注以下指标：

- 峰值 RSS
- spill 次数
- spill 总字节
- 小图请求 P95
- 大图混合场景吞吐

## 本次实际执行的命令

```bash
timeout 60s /usr/bin/time -v ~/.cargo/bin/cargo test --test http_forwarding_test -- --nocapture
```

说明：

- 这是一条本地端到端冒烟链路
- 会经过 `generateContent -> 上游转发 -> output=url 上传回填`
- 适合先看重构后链路是否稳定、RSS 是否离谱
- 不是正式压测，不能替代持续负载下的 P95/吞吐结论

## 本次记录结果

- `Maximum resident set size`: `62016 KB`
- `User time`: `0.06s`
- `System time`: `0.04s`
- `Elapsed time`: `0.11s`
- `Exit status`: `0`

## 当前进展

- **spill 次数 / spill 总字节**：已补 `blob_runtime` 级别计数，并通过 `/admin/api/stats` 输出 `spillCount`、`spillBytesTotal`
- **持续压测脚本**：已补 `scripts/benchmark_docker_mock_upstream.py`
- **真实链路边界**：脚本当前采用“请求侧真实图片 URL + mock 上游 + 真实图床上传”

当前仍未补：

- **小图 P95**：需要用真实图片 URL 集合实际跑一轮
- **大图混合场景吞吐**：同上，当前只是工具已就位

## 默认预算

默认预算由 `INSTANCE_MEMORY_BYTES` 推导：

- `2GiB`: `inline=8MiB`，`request_hot=24MiB`，`global_hot=384MiB`
- `4GiB`: `inline=12MiB`，`request_hot=40MiB`，`global_hot=768MiB`
- `8GiB`: `inline=16MiB`，`request_hot=64MiB`，`global_hot=1536MiB`

直接覆盖时可用这些环境变量：

- `BLOB_INLINE_MAX_BYTES`
- `BLOB_REQUEST_HOT_BUDGET_BYTES`
- `BLOB_GLOBAL_HOT_BUDGET_BYTES`
- `BLOB_SPILL_DIR`

## 调参建议

### 2GiB 实例

- 先保持默认值
- `BLOB_SPILL_DIR` 放本地 SSD，不要放慢网络盘
- 如果 RSS 仍容易顶满，优先下调 `BLOB_GLOBAL_HOT_BUDGET_BYTES`

### 4GiB 实例

- 默认值适合作为通用档位
- 如果请求多为中小图，优先提高 `BLOB_REQUEST_HOT_BUDGET_BYTES`
- 如果已经频繁 spill 且磁盘不是瓶颈，再考虑提高 `BLOB_GLOBAL_HOT_BUDGET_BYTES`

### 8GiB 实例

- 可以让大部分常见图片留在热内存
- 不建议盲目继续抬高 `inline`，否则单请求峰值会上升太快
- 更稳妥的做法是先增加 `global_hot`，保住并发时的整体命中率

## 推荐复测顺序

1. 先跑一次当前的 `time -v` 冒烟，确认链路与 RSS 基线正常
2. 再用 `scripts/benchmark_docker_mock_upstream.py` 跑一条真实图片 URL 压测，记录 P95
3. 最后切换不同 `MALLOC_CONF` 档位，比较 RSS 回落和 `spill` 行为

## 2026-04-15 Proxy Extra Overhead Smoke

本轮不是正式的重图片基线，只是先验证“直连 vs 经过代理”的四组场景都能真实出数。

Smoke 条件：

- 请求图源：`https://httpbin.org/image/png|jpeg|webp`
- 并发：`1`
- 请求数：`1`
- mock 上游响应：`20 MiB` base64 图片
- `cooldownSeconds=0`

说明：

- 这批数据主要用来确认 benchmark 脚本、阶段观测、`direct/proxy` 对照字段和 `hit/miss + base64/url` 场景组合都已经打通
- 因为请求图本身很小，而且每组只跑了 `1` 次，所以这些数字**不能**当作最终的重图片性能基线

四组 smoke 结果：

- `miss/base64`
  - `direct_total_ms=96.019`
  - `proxy_total_ms=7646.471`
  - `proxy_overhead_ms=7550.452`
  - `peakRssKb=33080`
  - `spillCount=0`
  - `spillBytesTotal=0`
- `hit/base64`
  - `direct_total_ms=94.524`
  - `proxy_total_ms=375.655`
  - `proxy_overhead_ms=281.131`
  - `peakRssKb=83548`
  - `spillCount=0`
  - `spillBytesTotal=0`
- `miss/url`
  - `direct_total_ms=105.222`
  - `proxy_total_ms=8946.736`
  - `proxy_overhead_ms=8841.514`
  - `peakRssKb=94324`
  - `spillCount=1`
  - `spillBytesTotal=15728640`
- `hit/url`
  - `direct_total_ms=99.962`
  - `proxy_total_ms=7645.804`
  - `proxy_overhead_ms=7545.842`
  - `peakRssKb=106904`
  - `spillCount=2`
  - `spillBytesTotal=31457280`

当前可得出的仅限结论：

- 请求侧缓存命中对 `base64` 路径收益非常明显，smoke 下从约 `7.6s` 降到约 `0.38s`
- `output=url` 路径即便请求侧命中缓存，整体仍明显慢于 `base64`，说明上传链路大概率还是首轮优化重点
- `output=url` 场景已经能在 benchmark summary 里稳定观测到 `spillCount / spillBytesTotal`
- benchmark summary 现在已经原生包含 `direct/proxy` 的 `P50/P95` 和对应 `proxy_overhead_p50_ms / proxy_overhead_p95_ms`

下一步仍需要：

- 换成真正的大请求图片样本
- 每组至少跑出 `P50/P95`
- 再用这组真实基线指导后续快路径裁剪和上传前减重

## 2026-04-15 正式大图基线（修正后）

正式基线条件：

- 请求图源：
  - `https://www.fileexamples.com/api/sample-file?format=jpg&size=10485760`
  - `https://www.fileexamples.com/api/sample-file?format=png&size=10485760`
  - `https://www.fileexamples.com/api/sample-file?format=webp&size=10485760`
- 每张请求图约 `10 MiB`
- mock 上游响应：`20 MiB` base64 图片
- 并发：`1`
- 每组请求数：`3`
- `miss` 场景已修正为每请求自动附加 cache buster，避免同一 run 内被缓存污染

四组正式结果：

- `miss/base64`
  - `proxy_overhead_ms=4624.309`
  - `proxy_overhead_p50_ms=4392.043`
  - `proxy_overhead_p95_ms=5128.457`
  - 平均阶段耗时：
    - `requestImagePrepareMs=3905.5`
    - `upstreamBuildMs=600.0`
    - `responseProcessMs=146.0`
    - `uploadMs=0.0`
  - `peakRssKb=177772`
- `hit/base64`
  - `proxy_overhead_ms=1099.214`
  - `proxy_overhead_p50_ms=1094.223`
  - `proxy_overhead_p95_ms=1132.61`
  - 平均阶段耗时：
    - `requestImagePrepareMs=1652.667`
    - `upstreamBuildMs=600.667`
    - `responseProcessMs=178.0`
    - `uploadMs=0.0`
  - `peakRssKb=184376`
- `miss/url`
  - `proxy_overhead_ms=12318.063`
  - `proxy_overhead_p50_ms=12130.559`
  - `proxy_overhead_p95_ms=13143.576`
  - 平均阶段耗时：
    - `requestImagePrepareMs=4544.5`
    - `upstreamBuildMs=618.0`
    - `responseProcessMs=109.0`
    - `uploadMs=7460.0`
  - `peakRssKb=211096`
- `hit/url`
  - `proxy_overhead_ms=10903.06`
  - `proxy_overhead_p50_ms=8272.255`
  - `proxy_overhead_p95_ms=16261.841`
  - 平均阶段耗时：
    - `requestImagePrepareMs=1645.667`
    - `upstreamBuildMs=599.333`
    - `responseProcessMs=98.667`
    - `uploadMs=10114.0`
  - `peakRssKb=156840`

从正式基线得出的结论：

- `base64` 路径的主要额外耗时在请求侧图片准备
- `output=url` 路径的主要额外耗时在上传阶段，且占比远高于其他阶段
- 即使缓存命中，请求侧图片准备仍有约 `1.6s`，说明命中路径仍存在可继续削减的额外复制 / 落盘 / 读回

## 2026-04-15 命中路径首轮优化结果

已落地优化：

- `BlobRuntime` 新增 `store_shared_bytes(Bytes, ...)`
- 请求缓存命中路径不再把 `Bytes` 先 `to_vec()` 再写入 `BlobRuntime`
- `image_io::fetch_image_into_blob` 也走同一条零额外 `Vec` 复制入口

焦点回归：

- `blob_runtime_can_store_shared_bytes_without_vec_copy`
- `request_materialize_reuses_fetch_service_cache_between_calls`

命中路径回跑结果（`hit/base64`）：

- 优化前：
  - `proxy_overhead_ms=1099.214`
  - `proxy_overhead_p50_ms=1094.223`
  - 平均阶段耗时：
    - `requestImagePrepareMs=1652.667`
    - `upstreamBuildMs=600.667`
    - `responseProcessMs=178.0`
- 优化后：
  - `proxy_overhead_ms=773.961`
  - `proxy_overhead_p50_ms=763.89`
  - 平均阶段耗时：
    - `requestImagePrepareMs=1560.667`
    - `upstreamBuildMs=322.667`
    - `responseProcessMs=153.0`

这一刀的实际收益：

- `proxy_overhead_ms` 约下降 `29.6%`
- `proxy_overhead_p50_ms` 约下降 `30.2%`
- 在不抬高设计复杂度的前提下，先拿掉了一次大对象复制

## 2026-04-15 命中路径第二轮优化结果

已落地优化：

- `BlobRuntime` 新增“外部共享内存 blob”入口 `store_external_shared_bytes(Bytes, ...)`
- 仅对 **请求侧 fetch cache 命中** 的图片启用该路径
- 命中缓存的大图不再因为 `BlobRuntime` 热预算限制被再次落盘
- 编码阶段继续复用同一份共享 `Bytes`，避免“命中缓存 -> 写盘 -> 读回 -> base64”这条中转链

焦点回归：

- `blob_runtime_can_store_external_shared_bytes_without_spill`
- `request_materialize_keeps_large_cached_bytes_in_memory_without_spill`
- `request_encoder_reads_large_external_shared_blob_without_spill`
- `response_materialize_test`

命中路径再次回跑结果（`hit/base64`）：

- 上一版：
  - `proxy_overhead_ms=688.096`
  - `proxy_overhead_p50_ms=690.866`
  - `proxy_overhead_p95_ms=684.204`
- 本版：
  - `proxy_overhead_ms=516.865`
  - `proxy_overhead_p50_ms=500.856`
  - `proxy_overhead_p95_ms=555.721`

这一刀的实际收益：

- 相比上一版，`proxy_overhead_ms` 再下降约 `24.9%`
- 相比上一版，`proxy_overhead_p50_ms` 再下降约 `27.5%`
- 相比最初 `1099.214ms` 基线，命中路径总额外耗时已降到约 `516.865ms`

这次收益的本质不是“再少一次复制”，而是：

- 对已经在内存缓存里的大图，停止重复付出磁盘 spill / read-back 的代价
- 且只在 cache hit 路径启用，避免把 miss 路径的大图常驻内存，控制峰值内存反弹风险

## 2026-04-15 `output=url` 上传前减重结果

已落地优化：

- `response_materialize` 不再把响应侧 base64 先完整 decode 成 `Vec<u8>` 再写入 `BlobRuntime`
- `Uploader` 新增直接消费 base64 inlineData 的上传入口
- legacy 图床改成“base64 按块解码 -> 流式 multipart 上传”
- R2 改成“base64 按块解码计算 SHA256 -> 再按块流式 PUT”
- 因此去掉了响应侧 `decode -> spill -> read-back -> upload` 这一整段本地中转链

焦点回归：

- `output_url_response_streams_large_base64_without_runtime_spill`
- `upload::tests::uploader_r2_base64_path_streams_decoded_bytes`
- `upload_mode_test`

回跑结果（`hit/url`）：

- 优化前：
  - `proxy_overhead_ms=10903.06`
  - `proxy_overhead_p50_ms=8272.255`
  - `proxy_overhead_p95_ms=16261.841`
  - `peakRssKb=156840`
  - `finalSpillCount=20`
  - `finalSpillBytesTotal=356516584`
- 优化后：
  - `proxy_overhead_ms=7653.763`
  - `proxy_overhead_p50_ms=7585.913`
  - `proxy_overhead_p95_ms=7804.065`
  - `peakRssKb=157076`
  - `finalSpillCount=7`
  - `finalSpillBytesTotal=199230184`

这一刀的实际收益：

- `proxy_overhead_ms` 下降约 `29.8%`
- `proxy_overhead_p95_ms` 下降约 `52.0%`
- 峰值 RSS 基本持平，没有出现明显内存反弹
- 最关键的是 spill 明显下降：
  - `finalSpillCount` 从 `20` 降到 `7`
  - `finalSpillBytesTotal` 从 `356516584` 降到 `199230184`

这说明 `output=url` 路径里，除了真实外部上传时间之外，本地上传前中转本身就是一个实打实的热点。  
把这段中转拿掉后，虽然无法消除外部图床网络时间，但依然能把本层额外耗时砍掉一大截。

## 2026-04-15 spilled blob 编码热路径优化结果

已落地优化：

- `request_encode::write_blob_as_base64` 的 spilled blob 路径改成固定输入/输出缓冲复用
- 去掉了每轮 chunk 编码时反复分配输出 `Vec`
- 去掉了原来 `pending.drain(..complete_len)` 带来的大块 memmove
- 余数只保留最多 `2` 字节，整体改成“小尾巴前移 + 大块复用编码”

焦点回归：

- `request_encoder_writes_spilled_blob_as_base64`

回跑结果（`miss/base64`）：

- 优化前：
  - `proxy_overhead_ms=4624.309`
  - `proxy_overhead_p50_ms=4392.043`
  - `proxy_overhead_p95_ms=5128.457`
  - `peakRssKb=177772`
  - `finalSpillCount=12`
  - `finalSpillBytesTotal=220201518`
- 优化后：
  - `proxy_overhead_ms=4099.605`
  - `proxy_overhead_p50_ms=3863.006`
  - `proxy_overhead_p95_ms=4727.0`
  - `peakRssKb=158596`
  - `finalSpillCount=11`
  - `finalSpillBytesTotal=178258292`

这一刀的实际收益：

- `proxy_overhead_ms` 下降约 `11.3%`
- `proxy_overhead_p50_ms` 下降约 `12.0%`
- `proxy_overhead_p95_ms` 下降约 `7.8%`

这次收益主要来自请求侧构造阶段自身的 CPU / 内存管理开销下降。  
也就是说，miss 场景里除了真实图片抓取之外，spilled blob 的 base64 编码本身也确实还有可观的纯本地成本。

## 2026-04-15 `requestImagePrepareMs` 拆分结果

已落地观测增强：

- admin / stats / benchmark 新增：
  - `requestImageMaterializeMs`
  - `requestEncodeMs`
- 保留原有 `requestImagePrepareMs`，避免历史对比失真

焦点回归：

- `admin_logs_and_stats_include_stage_duration_fields`
- `scripts/test_benchmark_docker_mock_upstream.py`

真实大图样本现状判断：

- `hit/base64`
  - 样本显示 `requestImageMaterializeMs` 明显大于 `requestEncodeMs`
  - 当前 `requestEncodeMs` 只占很小一部分，已经不再是主要热点
- `miss/base64`
  - 同样是 `requestImageMaterializeMs` 远高于 `requestEncodeMs`
  - 说明 miss 场景剩余大头已经集中到请求侧图片 materialize，而不是编码本身

当前阶段结论：

- `request_encode` 还能做的小优化已经进入边际区
- 下一步更值得继续拆的是 `materialize`
- 尤其要继续区分：
  - 真实下载 / 缓存等待
  - 本地转换 / 物化

## 2026-04-15 benchmark 采样口径修正 + `fetch/store work` 真值

口径修正：

- 之前直接平均 `stats-samples.csv` 的做法是错的
- 因为 `/admin/api/stats` 是**累计计数器快照**，不是逐请求样本
- 直接平均这些行，会把历史累计值误当成单次请求阶段耗时
- benchmark 脚本现已改为直接抓 `/admin/api/logs`，并输出：
  - `admin-log-stage-rows.csv`
  - `admin-logs.json`
  - summary 内的 `avg*` 阶段字段

焦点回归：

- `scripts/test_benchmark_docker_mock_upstream.py`

修正口径后的真实大图结果：

- `hit/base64`
  - `proxy_overhead_ms=494.689`
  - `avgRequestImagePrepareMs=68.333`
  - `avgRequestImageMaterializeMs=0.0`
  - `avgRequestImageFetchWorkMs=0.0`
  - `avgRequestImageStoreWorkMs=0.0`
  - `avgRequestEncodeMs=68.333`
  - `avgUpstreamBuildMs=248.0`
  - `avgResponseProcessMs=111.667`
- `miss/base64`
  - `proxy_overhead_ms=4099.009`
  - `avgRequestImagePrepareMs=3660.0`
  - `avgRequestImageMaterializeMs=3589.0`
  - `avgRequestImageFetchWorkMs=9811.333`
  - `avgRequestImageStoreWorkMs=34.333`
  - `avgRequestEncodeMs=70.333`
  - `avgUpstreamBuildMs=267.0`
  - `avgResponseProcessMs=107.0`

修正后的阶段结论：

- **命中路径** 其实已经很好了
  - 请求侧图片物化基本可忽略
  - 命中后额外耗时主要只剩 base64 编码、上游发送和响应处理
- **未命中路径** 的大头几乎全在 `fetch`
  - `requestImageStoreWorkMs` 只有约 `34ms`
  - `requestEncodeMs` 只有约 `70ms`
  - 说明本地“存储 / 物化 / 编码”已经不是主要矛盾
- `requestImageFetchWorkMs` 远高于 `requestImageMaterializeMs`
  - 这是多张图并发抓取时的 worker work 总和，符合并发语义
  - 从墙钟看，miss 场景当前主要成本仍是首轮真实图片下载，而不是本地处理

下一步判断：

- 如果目标继续压 `miss/base64`，更值得继续查的是 `fetch` 内部等待构成
- 但从当前数据看，继续打 `store/encode` 的收益大概率已经很小
