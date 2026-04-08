# generateContent Memory Redesign Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Rebuild `rust-sync-proxy` around non-stream `generateContent` only, remove local image-proxy responsibilities, and introduce a `BlobHandle` spill runtime so fixed-memory instances can sustain more concurrent large-image requests without sacrificing normal-path throughput.

**Architecture:** Shrink the product surface to `generateContent`, delete `streamGenerateContent` and `/proxy/image`, and separate JSON structure from large binary payloads. Request-side image fetches and response-side image uploads flow through a `BlobHandle` runtime that keeps common images in hot RAM and spills only when object size or memory budget requires it.

**Tech Stack:** Rust 2024, axum 0.8, tokio, reqwest, serde_json, bytes, base64, std::fs, `tokio-util` for streaming IO adapters, `tempfile` for tests

---

### Task 1: 收缩 API 范围到 generateContent

**Files:**
- Modify: `src/http/router.rs`
- Modify: `src/lib.rs`
- Modify: `README.md`
- Modify: `tests/router_test.rs`
- Modify: `tests/http_forwarding_test.rs`
- Delete: `src/stream_rewrite.rs`
- Delete: `src/proxy_image.rs`
- Delete: `tests/stream_rewrite_test.rs`
- Delete: `tests/proxy_image_test.rs`
- Delete: `tests/go_compat_test.rs`

**Step 1: 写失败测试，锁定收缩后的路由边界**

```rust
#[tokio::test]
async fn stream_generate_content_route_is_not_exposed() {
    let app = rust_sync_proxy::build_router(rust_sync_proxy::test_config());
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1beta/models/demo:streamGenerateContent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn proxy_image_route_is_not_exposed() {
    let app = rust_sync_proxy::build_router(rust_sync_proxy::test_config());
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/proxy/image?url=https://example.com/a.png")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
```

**Step 2: 运行测试，确认当前实现仍暴露旧路径**

Run: `~/.cargo/bin/cargo test --test router_test -- --nocapture`

Expected: FAIL，因为当前 `build_router` 还注册了流式路由和 `/proxy/image`。

**Step 3: 做最小实现，删除旧路由和相关模块导出**

```rust
Router::new()
    .route("/admin", get(admin_root))
    .route("/admin/", get(admin_root))
    .route("/admin/logs", get(admin_logs_page))
    .route("/admin/api/logs", get(admin_logs_api))
    .route("/admin/api/stats", get(admin_stats_api))
    .route("/v1beta/models/{*rest}", post(model_action))
    .with_state(state)
```

并把 `model_action` 改成只接受 `:generateContent`。

**Step 4: 运行聚焦测试，确认边界收缩完成**

Run: `~/.cargo/bin/cargo test --test router_test -- --nocapture`

Expected: PASS

**Step 5: 提交**

```bash
git add src/http/router.rs src/lib.rs README.md tests/router_test.rs tests/http_forwarding_test.rs src/stream_rewrite.rs src/proxy_image.rs tests/stream_rewrite_test.rs tests/proxy_image_test.rs tests/go_compat_test.rs
git commit -m "refactor: narrow rust proxy to generateContent only"
```

### Task 2: 建立 BlobRuntime 和 spill 基础设施

**Files:**
- Create: `src/blob_runtime.rs`
- Modify: `src/lib.rs`
- Modify: `src/config.rs`
- Modify: `Cargo.toml`
- Create: `tests/blob_runtime_test.rs`

**Step 1: 先写失败测试，锁定内存/落盘/清理行为**

```rust
#[tokio::test]
async fn blob_runtime_keeps_small_blob_inline() {
    let runtime = BlobRuntime::new(BlobRuntimeConfig {
        inline_max_bytes: 8 * 1024 * 1024,
        request_hot_budget_bytes: 24 * 1024 * 1024,
        global_hot_budget_bytes: 384 * 1024 * 1024,
        spill_dir: tempdir().unwrap().path().to_path_buf(),
    });

    let handle = runtime
        .store_bytes(b"abc".to_vec(), "image/png".into())
        .await
        .unwrap();

    assert!(runtime.is_inline(&handle).await);
}

#[tokio::test]
async fn blob_runtime_spills_large_blob_to_disk() {
    let runtime = test_blob_runtime(1024);
    let handle = runtime
        .store_bytes(vec![7; 4096], "image/png".into())
        .await
        .unwrap();

    assert!(runtime.is_spilled(&handle).await);
}
```

**Step 2: 运行测试，确认 `BlobRuntime` 尚不存在**

Run: `~/.cargo/bin/cargo test --test blob_runtime_test -- --nocapture`

Expected: FAIL with unresolved import / unknown type `BlobRuntime`

**Step 3: 实现最小可用 runtime**

```rust
pub enum BlobStorage {
    Inline(Bytes),
    Spilled(PathBuf),
}

pub struct BlobHandle {
    id: u64,
    meta: BlobMeta,
    storage: BlobStorage,
}

pub struct BlobRuntime {
    cfg: BlobRuntimeConfig,
    next_id: AtomicU64,
    global_hot_bytes: AtomicU64,
}
```

实现：

- `store_bytes`
- `store_stream`
- `open_reader`
- `remove`
- `is_inline`
- `is_spilled`

**Step 4: 跑测试确认基础行为正确**

Run: `~/.cargo/bin/cargo test --test blob_runtime_test -- --nocapture`

Expected: PASS

**Step 5: 提交**

```bash
git add Cargo.toml src/blob_runtime.rs src/lib.rs src/config.rs tests/blob_runtime_test.rs
git commit -m "feat: add blob runtime with spill support"
```

### Task 3: 重写请求侧图片 URL materialize 为 BlobHandle

**Files:**
- Create: `src/request_scan.rs`
- Create: `src/request_materialize.rs`
- Modify: `src/image_io.rs`
- Modify: `src/lib.rs`
- Create: `tests/request_materialize_test.rs`
- Modify: `tests/request_inline_data_flow_test.rs`

**Step 1: 先写失败测试，锁定“扫描 URL + 下载成 BlobHandle”的行为**

```rust
#[tokio::test]
async fn request_materialize_fetches_image_url_into_blob_handle() {
    let (image_url, _server) = spawn_png_server().await;
    let runtime = test_blob_runtime(8 * 1024 * 1024);
    let request = serde_json::json!({
        "contents": [{"parts": [{"inlineData": {"data": image_url}}]}]
    });

    let materialized = materialize_request_images(request, &runtime, &reqwest::Client::new())
        .await
        .unwrap();

    assert_eq!(materialized.replacements.len(), 1);
    assert_eq!(materialized.replacements[0].mime_type, "image/png");
}
```

**Step 2: 运行测试，确认新请求物化层尚未实现**

Run: `~/.cargo/bin/cargo test --test request_materialize_test -- --nocapture`

Expected: FAIL with missing `request_materialize` module or function

**Step 3: 实现扫描和下载层**

```rust
pub struct RequestReplacement {
    pub json_pointer: String,
    pub mime_type: String,
    pub blob: BlobHandle,
}

pub struct MaterializedRequestImages {
    pub request: Value,
    pub replacements: Vec<RequestReplacement>,
}
```

实现规则：

- 只识别 `inlineData.data` 为 `http(s)` 的节点
- 下载结果直接写入 `BlobRuntime`
- 返回轻量结构和句柄，不生成 base64

**Step 4: 跑测试确认请求侧完成“URL -> BlobHandle”**

Run:

```bash
~/.cargo/bin/cargo test --test request_materialize_test -- --nocapture
~/.cargo/bin/cargo test --test request_inline_data_flow_test -- --nocapture
```

Expected: PASS

**Step 5: 提交**

```bash
git add src/request_scan.rs src/request_materialize.rs src/image_io.rs src/lib.rs tests/request_materialize_test.rs tests/request_inline_data_flow_test.rs
git commit -m "feat: materialize request image urls into blob handles"
```

### Task 4: 实现 request_encode，把 BlobHandle 流式编码为上游请求体

**Files:**
- Create: `src/request_encode.rs`
- Modify: `src/http/router.rs`
- Modify: `src/lib.rs`
- Create: `tests/request_encode_test.rs`
- Modify: `tests/http_forwarding_test.rs`

**Step 1: 先写失败测试，锁定不经大内存字符串也能得到正确上游 JSON**

```rust
#[tokio::test]
async fn request_encoder_writes_inline_data_base64_from_blob_handle() {
    let runtime = test_blob_runtime(8 * 1024 * 1024);
    let blob = runtime.store_bytes(vec![1, 2, 3], "image/png".into()).await.unwrap();
    let request = serde_json::json!({
        "contents": [{"parts": [{"inlineData": {"data": "https://example.com/a.png"}}]}]
    });
    let encoded = encode_request_body(request, vec![replacement("/contents/0/parts/0/inlineData", blob)])
        .await
        .unwrap();

    let text = read_blob_to_string(&runtime, &encoded.body_blob).await;
    assert!(text.contains("\"mimeType\":\"image/png\""));
    assert!(text.contains("\"data\":\"AQID\""));
}
```

**Step 2: 运行测试，确认编码器尚未实现**

Run: `~/.cargo/bin/cargo test --test request_encode_test -- --nocapture`

Expected: FAIL with missing `encode_request_body`

**Step 3: 实现基于 BlobHandle 的请求编码**

```rust
pub struct EncodedRequestBody {
    pub body_blob: BlobHandle,
    pub content_length: u64,
}

pub async fn encode_request_body(
    request: Value,
    replacements: Vec<RequestReplacement>,
    runtime: &BlobRuntime,
) -> Result<EncodedRequestBody> {
    // 写入一个新的 output blob；普通字段直接序列化，
    // 图片字段从 BlobHandle reader 读取并边 base64 编码边写入。
}
```

实现要求：

- 去掉 `output` 字段再发上游
- 不构造完整大 base64 `String`
- 结果写入新的 `body_blob`
- `router` 用该 blob 的 reader 构造 reqwest body

**Step 4: 跑聚焦测试和 HTTP 转发测试**

Run:

```bash
~/.cargo/bin/cargo test --test request_encode_test -- --nocapture
~/.cargo/bin/cargo test --test http_forwarding_test -- --nocapture
```

Expected: PASS

**Step 5: 提交**

```bash
git add src/request_encode.rs src/http/router.rs src/lib.rs tests/request_encode_test.rs tests/http_forwarding_test.rs
git commit -m "feat: stream upstream request bodies from blob handles"
```

### Task 5: 重写响应侧 materialize/upload，去掉本地 proxy URL 包装

**Files:**
- Create: `src/response_materialize.rs`
- Modify: `src/response_rewrite.rs`
- Modify: `src/upload.rs`
- Modify: `src/config.rs`
- Modify: `tests/http_forwarding_test.rs`
- Create: `tests/response_materialize_test.rs`
- Modify: `tests/upload_mode_test.rs`

**Step 1: 先写失败测试，锁定“base64 -> BlobHandle -> 上传 -> 外部代理前缀 URL”**

```rust
#[tokio::test]
async fn output_url_response_rewrites_uploaded_url_with_external_proxy_prefix() {
    let runtime = test_blob_runtime(8 * 1024 * 1024);
    let mut body = serde_json::json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "inlineData": {
                        "mimeType": "image/png",
                        "data": "AQID"
                    }
                }]
            }
        }]
    });

    finalize_output_urls(&mut body, &runtime, &fake_uploader("https://img.example.com/a.png"), "https://external-proxy.example/fetch?url=")
        .await
        .unwrap();

    assert_eq!(
        body["candidates"][0]["content"]["parts"][0]["inlineData"]["data"],
        "https://external-proxy.example/fetch?url=https%3A%2F%2Fimg.example.com%2Fa.png"
    );
}
```

**Step 2: 跑测试，确认新响应物化层尚未实现**

Run: `~/.cargo/bin/cargo test --test response_materialize_test -- --nocapture`

Expected: FAIL with missing `finalize_output_urls`

**Step 3: 实现响应 materialize 和上传收尾**

```rust
pub async fn finalize_output_urls(
    body: &mut Value,
    runtime: &BlobRuntime,
    uploader: &Uploader,
    external_proxy_prefix: &str,
) -> Result<()> {
    // 识别 inlineData base64
    // 解码进 BlobHandle
    // 从 BlobHandle reader 直接上传
    // 回填 external_proxy_prefix + escaped(real_url)
}
```

同时更新 `Uploader`：

- 增加从 reader / blob 上传的接口
- 删除本地 `/proxy/image` 包装逻辑

**Step 4: 跑聚焦测试和现有 output=url 回归**

Run:

```bash
~/.cargo/bin/cargo test --test response_materialize_test -- --nocapture
~/.cargo/bin/cargo test --test http_forwarding_test -- --nocapture
```

Expected: PASS，并且测试断言改为外部代理前缀 URL

**Step 5: 提交**

```bash
git add src/response_materialize.rs src/response_rewrite.rs src/upload.rs src/config.rs tests/http_forwarding_test.rs tests/response_materialize_test.rs tests/upload_mode_test.rs
git commit -m "feat: finalize output urls through blob uploads"
```

### Task 6: 去掉默认大 payload admin 脱敏路径，改成轻量观测

**Files:**
- Modify: `src/admin.rs`
- Modify: `src/http/router.rs`
- Modify: `tests/admin_test.rs`
- Create: `tests/config_budget_test.rs`

**Step 1: 先写失败测试，锁定 admin 关闭时不再序列化整份大 body**

```rust
#[tokio::test]
async fn router_skips_admin_body_sanitization_when_admin_is_disabled() {
    let mut config = rust_sync_proxy::test_config();
    config.admin_password = String::new();
    let app = rust_sync_proxy::build_router(config);

    // 发送普通请求，断言不会进入 admin-only body capture path
    // 可通过暴露一个测试计数器或拆分函数后做单元测试。
}
```

**Step 2: 跑测试，确认当前 router 仍无条件做 sanitize**

Run: `~/.cargo/bin/cargo test --test admin_test -- --nocapture`

Expected: FAIL，因为当前 `forward_gemini_request` 默认会做 `sanitize_json_for_log`

**Step 3: 实现轻量观测**

```rust
let admin_enabled = state.admin.is_some();
let request_raw = if admin_enabled {
    Some(admin::sanitize_json_for_log(&request_body))
} else {
    None
};
```

同时新增预算配置测试，确认：

- 2GiB / 4GiB / 8GiB 对应默认值正确

**Step 4: 跑测试确认 admin 与预算逻辑成立**

Run:

```bash
~/.cargo/bin/cargo test --test admin_test -- --nocapture
~/.cargo/bin/cargo test --test config_budget_test -- --nocapture
```

Expected: PASS

**Step 5: 提交**

```bash
git add src/admin.rs src/http/router.rs tests/admin_test.rs tests/config_budget_test.rs
git commit -m "perf: disable heavy admin body capture by default"
```

### Task 7: 清理旧缓存/旧改写路径并跑完整回归

**Files:**
- Modify: `src/cache.rs`
- Modify: `src/request_rewrite.rs`
- Modify: `src/response_rewrite.rs`
- Modify: `README.md`
- Modify: `tests/request_cache_test.rs`
- Modify: `tests/response_rewrite_test.rs`
- Modify: `tests/http_smoke.rs`

**Step 1: 先写或更新失败测试，锁定新边界下仍保留的能力**

```rust
#[tokio::test]
async fn generate_content_output_url_smoke_still_passes_after_blob_runtime_refactor() {
    // 使用新的 generateContent-only app 构造一次完整请求，
    // 断言请求侧 URL 与响应侧 output=url 均正确。
}
```

**Step 2: 跑聚焦测试，确认老路径与新路径仍有冲突**

Run: `~/.cargo/bin/cargo test --test http_smoke -- --nocapture`

Expected: FAIL，直到旧缓存和旧改写路径被清理为止

**Step 3: 清理旧实现残留**

```rust
// 删除只服务 stream/proxy-image 的分支
// 删除只为旧代理 URL 包装服务的辅助函数
// 保留仍有价值的请求侧缓存，但把接口改为 BlobHandle-aware
```

**Step 4: 跑完整测试集**

Run: `timeout 60s ~/.cargo/bin/cargo test --tests -- --nocapture`

Expected: PASS

**Step 5: 提交**

```bash
git add src/cache.rs src/request_rewrite.rs src/response_rewrite.rs README.md tests/request_cache_test.rs tests/response_rewrite_test.rs tests/http_smoke.rs
git commit -m "refactor: complete generateContent blob-runtime migration"
```

### Task 8: 基准与调参文档

**Files:**
- Modify: `README.md`
- Create: `docs/plans/2026-04-09-generate-content-memory-redesign-benchmark-notes.md`

**Step 1: 写失败测试或检查清单，锁定需要观测的指标**

```text
必须记录：
- 峰值 RSS
- spill 次数
- spill 总字节
- 小图 P95
- 大图混合场景吞吐
```

**Step 2: 人工执行基准并记录结果**

Run: `/usr/bin/time -v <your-load-command>`

Expected: 得到 Maximum resident set size、CPU time、elapsed time

**Step 3: 写文档，明确默认预算和调参建议**

```markdown
- 2GiB: inline=8MiB, request_hot=24MiB, global_hot=384MiB
- 4GiB: inline=12MiB, request_hot=40MiB, global_hot=768MiB
- 8GiB: inline=16MiB, request_hot=64MiB, global_hot=1536MiB
```

**Step 4: 提交**

```bash
git add README.md docs/plans/2026-04-09-generate-content-memory-redesign-benchmark-notes.md
git commit -m "docs: add blob runtime benchmark and tuning notes"
```
