# Docker Mock Upstream Benchmark Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a Docker-based benchmark flow that drives real request image URLs, a mock upstream returning one 20MB base64 image, and records RSS plus `BlobRuntime` spill metrics.

**Architecture:** Extend `BlobRuntime` with minimal spill counters and surface them through `/admin/api/stats`, then add a Python benchmark tool that starts a local mock upstream, runs the proxy in Docker, drives concurrent `output=url` requests, and writes RSS / stats / latency artifacts to disk.

**Tech Stack:** Rust 2024, axum 0.8, tokio, serde, Python 3 stdlib, Docker CLI

---

### Task 1: 锁定 spill 计数行为

**Files:**
- Modify: `tests/blob_runtime_test.rs`
- Modify: `src/blob_runtime.rs`

**Step 1: 写失败测试**

```rust
#[tokio::test]
async fn blob_runtime_records_spill_count_and_bytes() {
    let runtime = test_blob_runtime(1024);

    let first = runtime
        .store_bytes(vec![7; 4096], "image/png".into())
        .await
        .unwrap();
    let second = runtime
        .store_bytes(vec![8; 2048], "image/png".into())
        .await
        .unwrap();

    assert!(runtime.is_spilled(&first).await);
    assert!(runtime.is_spilled(&second).await);

    let stats = runtime.stats_snapshot();
    assert_eq!(stats.spill_count, 2);
    assert_eq!(stats.spill_bytes_total, 4096 + 2048);
}
```

**Step 2: 跑测试确认失败**

Run: `~/.cargo/bin/cargo test --test blob_runtime_test -- --nocapture`

Expected: FAIL，因为 `stats_snapshot` 和计数字段还不存在。

**Step 3: 做最小实现**

- `BlobRuntimeInner` 增加原子计数器
- `store_bytes` / `store_stream` 在最终 spill 时累加
- 暴露 `stats_snapshot()`

**Step 4: 重跑测试确认通过**

Run: `~/.cargo/bin/cargo test --test blob_runtime_test -- --nocapture`

Expected: PASS

### Task 2: 锁定 admin stats API 新字段

**Files:**
- Modify: `tests/admin_test.rs`
- Modify: `src/admin.rs`
- Modify: `src/http/router.rs`

**Step 1: 写失败测试**

```rust
#[tokio::test]
async fn admin_stats_include_spill_metrics() {
    let admin = rust_sync_proxy::admin::AdminState::new("pw".to_string());
    let snapshot = rust_sync_proxy::blob_runtime::BlobRuntimeStatsSnapshot {
        spill_count: 3,
        spill_bytes_total: 99,
    };

    let response = rust_sync_proxy::admin::admin_stats_response(&admin, snapshot);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["spillCount"], 3);
    assert_eq!(json["spillBytesTotal"], 99);
}
```

**Step 2: 跑测试确认失败**

Run: `~/.cargo/bin/cargo test --test admin_test -- --nocapture`

Expected: FAIL，因为 admin stats 还不接受 runtime snapshot。

**Step 3: 做最小实现**

- `AdminStatsPayload` 增加两个字段
- `admin_stats_response` 接受 `BlobRuntimeStatsSnapshot`
- `router` 在 `/admin/api/stats` 里传入 `state.blob_runtime.stats_snapshot()`

**Step 4: 重跑测试确认通过**

Run: `~/.cargo/bin/cargo test --test admin_test -- --nocapture`

Expected: PASS

### Task 3: 锁定 benchmark 辅助函数

**Files:**
- Create: `scripts/benchmark_docker_mock_upstream.py`
- Create: `scripts/test_benchmark_docker_mock_upstream.py`

**Step 1: 写失败测试**

```python
import base64
import unittest

from scripts.benchmark_docker_mock_upstream import build_base64_payload, build_request_body


class BenchmarkHelperTest(unittest.TestCase):
    def test_build_base64_payload_hits_target_size(self):
        payload = build_base64_payload(20 * 1024 * 1024)
        self.assertEqual(len(payload), 20 * 1024 * 1024)
        self.assertTrue(base64.b64decode(payload, validate=True))

    def test_build_request_body_contains_three_image_urls(self):
        body = build_request_body([
            "https://img.example/1.png",
            "https://img.example/2.png",
            "https://img.example/3.png",
        ])
        self.assertEqual(len(body["contents"][0]["parts"]), 3)
        self.assertEqual(body["output"], "url")
```

**Step 2: 跑测试确认失败**

Run: `python3 -m unittest scripts/test_benchmark_docker_mock_upstream.py`

Expected: FAIL，因为 benchmark 模块还不存在。

**Step 3: 做最小实现**

- 实现 base64 负载生成
- 实现请求体生成
- 实现 mock upstream server
- 实现 Docker benchmark 主流程和 CSV/JSON 落盘

**Step 4: 重跑测试确认通过**

Run: `python3 -m unittest scripts/test_benchmark_docker_mock_upstream.py`

Expected: PASS

### Task 4: 文档和工具验证

**Files:**
- Modify: `README.md`
- Modify: `docs/plans/2026-04-09-generate-content-memory-redesign-benchmark-notes.md`

**Step 1: 更新文档**

- 增加 benchmark 脚本使用说明
- 说明 mock upstream + 真实图床的边界

**Step 2: 做工具验证**

Run: `python3 scripts/benchmark_docker_mock_upstream.py --help`

Expected: PASS

### Task 5: 全量验证

**Files:**
- Modify: `README.md`

**Step 1: 跑 Rust 测试**

Run: `~/.cargo/bin/cargo test --tests -- --nocapture`

Expected: PASS

**Step 2: 跑 Python 单测**

Run: `python3 -m unittest scripts/test_benchmark_docker_mock_upstream.py`

Expected: PASS

**Step 3: 做最小产物验证**

Run: `python3 scripts/benchmark_docker_mock_upstream.py --help`

Expected: PASS

**Step 4: 记录剩余事项**

- 真正的压测执行依赖用户提供 3 个真实图片 URL
- 真实图床凭据仍由运行环境注入
