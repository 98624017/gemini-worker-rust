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
