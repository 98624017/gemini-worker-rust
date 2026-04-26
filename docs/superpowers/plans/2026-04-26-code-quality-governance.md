# Code Quality Governance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Improve code quality through behavior consistency, low-risk request-path cleanup, targeted memory fixes, and stronger tests without large module splits.

**Architecture:** Keep the current file layout. Changes stay inside existing modules, with only small helper extraction inside `router.rs` and focused tests around changed behavior. The plan deliberately avoids splitting `router.rs`, `admin.rs`, OpenAI image logic, or aiapidev logic into new production modules.

**Tech Stack:** Rust 2024, axum 0.8, reqwest 0.12, serde_json, tokio, tower test helpers, cargo test.

---

## File Structure

- Modify `src/http/router.rs`
  - Unify client-side JSON/body-read error classification.
  - Avoid double JSON parse in the Gemini standard path.
  - Keep all helper extraction private to this file.
- Modify `src/response_rewrite.rs`
  - Make `keep_largest_inline_image` rebuild parts by moving `Value`s instead of cloning large inline data.
  - Preserve URL-like inlineData parts while dropping only base64 inline image siblings.
- Modify `src/image_io.rs`
  - Add a no-copy result shape for image optimization when bytes are unchanged.
  - Keep compatibility wrappers where existing callers still expect `OptimizedImage`.
- Modify `src/response_materialize.rs`
  - Consume the no-copy optimization result so unchanged images are not re-encoded or copied.
- Modify `src/lib.rs`
  - Make `test_config()` derive from runtime defaults, then override only test-specific fields.
- Modify `tests/http_forwarding_test.rs`
  - Update invalid JSON expectations from `502` to `400`.
  - Add OpenAI image invalid JSON and request body limit tests.
  - Add an admin-log assertion for structured client errors.
- Modify `tests/response_rewrite_test.rs`
  - Add a regression test for preserving non-image and URL-like inline parts.
- Modify `tests/image_io_test.rs`
  - Add tests for unchanged optimization result.
- Modify `tests/config_test.rs`
  - Add a runtime-default parity test for `test_config()`.
- Modify `tests/admin_test.rs`
  - Add admin auth and log buffer boundary tests.

---

### Task 1: Client Error Statuses And Structured Error Consistency

**Files:**
- Modify: `tests/http_forwarding_test.rs`
- Modify: `src/http/router.rs`

- [ ] **Step 1: Update the Gemini invalid JSON test to expect 400**

In `tests/http_forwarding_test.rs`, change `invalid_request_json_returns_structured_proxy_error` to:

```rust
#[tokio::test]
async fn invalid_request_json_returns_structured_proxy_error() {
    let app = rust_sync_proxy::build_router(rust_sync_proxy::test_config());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"contents":[{"parts":[{"text":"hello"}]}]"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json_body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json_body["error"]["code"], 400);
    assert_eq!(json_body["error"]["message"], "invalid request json body");
    assert_eq!(json_body["error"]["source"], "proxy");
    assert_eq!(json_body["error"]["stage"], "parse_request_json");
    assert_eq!(json_body["error"]["kind"], "invalid_json");
}
```

- [ ] **Step 2: Add an OpenAI image invalid JSON test**

Append this test near the other `/v1/images/generations` tests in `tests/http_forwarding_test.rs`:

```rust
#[tokio::test]
async fn image_generations_invalid_json_returns_bad_request() {
    let app = rust_sync_proxy::build_router(rust_sync_proxy::test_config());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/images/generations")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"model":"gpt-image-2","prompt":"cat""#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json_body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json_body["error"]["code"], 400);
    assert_eq!(json_body["error"]["message"], "invalid request json body");
    assert_eq!(json_body["error"]["source"], "proxy");
    assert_eq!(json_body["error"]["stage"], "parse_request_json");
    assert_eq!(json_body["error"]["kind"], "invalid_json");
}
```

- [ ] **Step 3: Add request body limit tests**

Append these tests to `tests/http_forwarding_test.rs`:

```rust
#[tokio::test]
async fn generate_content_oversized_body_returns_payload_too_large() {
    let app = rust_sync_proxy::build_router(rust_sync_proxy::test_config());
    let oversized = "x".repeat(20 * 1024 * 1024 + 1);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(oversized))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json_body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json_body["error"]["code"], 413);
    assert_eq!(json_body["error"]["message"], "request body too large");
    assert_eq!(json_body["error"]["source"], "proxy");
    assert_eq!(json_body["error"]["stage"], "read_request_body");
    assert_eq!(json_body["error"]["kind"], "request_body_too_large");
}

#[tokio::test]
async fn image_generations_oversized_body_returns_payload_too_large() {
    let app = rust_sync_proxy::build_router(rust_sync_proxy::test_config());
    let oversized = "x".repeat(20 * 1024 * 1024 + 1);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/images/generations")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(oversized))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json_body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json_body["error"]["code"], 413);
    assert_eq!(json_body["error"]["message"], "request body too large");
    assert_eq!(json_body["error"]["source"], "proxy");
    assert_eq!(json_body["error"]["stage"], "read_request_body");
    assert_eq!(json_body["error"]["kind"], "request_body_too_large");
}
```

- [ ] **Step 4: Run the targeted tests and verify they fail**

Run:

```bash
timeout 60s cargo test --test http_forwarding_test -- --nocapture
```

Expected:

```text
invalid_request_json_returns_structured_proxy_error ... FAILED
image_generations_invalid_json_returns_bad_request ... FAILED
generate_content_oversized_body_returns_payload_too_large ... FAILED
image_generations_oversized_body_returns_payload_too_large ... FAILED
```

- [ ] **Step 5: Add request body read classification helpers**

In `src/http/router.rs`, below `proxy_error_response`, add:

```rust
fn request_body_read_error_response(err: &axum::Error) -> (StatusCode, &'static str, &'static str) {
    let detail = err.to_string();
    if detail.contains("length limit")
        || detail.contains("body exceeded")
        || detail.contains("too large")
    {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            "request body too large",
            "request_body_too_large",
        );
    }

    (
        StatusCode::BAD_GATEWAY,
        "failed to read request body",
        "request_body_read_failed",
    )
}

fn invalid_json_response() -> Response {
    proxy_error_response(
        StatusCode::BAD_REQUEST,
        "invalid request json body",
        "parse_request_json",
        "invalid_json",
    )
}
```

- [ ] **Step 6: Update request body read error branches**

In both `image_generations_action` and `model_action`, replace the current read-body error response construction with this pattern:

```rust
Err(err) => {
    let (status, message, kind) = request_body_read_error_response(&err);
    let response = proxy_error_response(status, message, "read_request_body", kind);
    return finalize_admin_response(
        &state,
        response,
        AdminLogEntry {
            created_at,
            method: request_method,
            path: request_path,
            query: request_query,
            remote_addr,
            is_stream: false,
            status_code: status.as_u16(),
            duration_ms: started_at.elapsed().as_millis() as i64,
            request_parse_ms: request_parse_started.elapsed().as_millis() as i64,
            error_source: "proxy".to_string(),
            error_stage: "read_request_body".to_string(),
            error_kind: kind.to_string(),
            error_message: message.to_string(),
            error_detail: err.to_string(),
            ..Default::default()
        },
    )
    .await;
}
```

- [ ] **Step 7: Update JSON parse error branches**

In both `image_generations_action` and `model_action`, change invalid JSON handling to use `StatusCode::BAD_REQUEST` and `invalid_json_response()`:

```rust
let parsed_body: Value = match serde_json::from_slice(&request_body) {
    Ok(body) => body,
    Err(err) => {
        let mut admin_entry = AdminLogEntry {
            created_at,
            method: request_method,
            path: request_path,
            query: request_query,
            remote_addr,
            is_stream: false,
            status_code: StatusCode::BAD_REQUEST.as_u16(),
            duration_ms: started_at.elapsed().as_millis() as i64,
            request_parse_ms: request_parse_started.elapsed().as_millis() as i64,
            error_source: "proxy".to_string(),
            error_stage: "parse_request_json".to_string(),
            error_kind: "invalid_json".to_string(),
            error_message: "invalid request json body".to_string(),
            error_detail: format!("invalid request json body: {err}"),
            ..Default::default()
        };
        request_log.apply_to_entry(&mut admin_entry);
        return finalize_admin_response(&state, invalid_json_response(), admin_entry).await;
    }
};
```

- [ ] **Step 8: Run targeted tests and verify they pass**

Run:

```bash
timeout 60s cargo test --test http_forwarding_test -- --nocapture
```

Expected:

```text
test result: ok.
```

- [ ] **Step 9: Commit Task 1**

Run:

```bash
git add src/http/router.rs tests/http_forwarding_test.rs
git commit -m "fix: classify client request errors"
```

---

### Task 2: Remove Gemini Request Double Parse And Timing Double Count

**Files:**
- Modify: `src/http/router.rs`
- Modify: `tests/http_forwarding_test.rs`

- [ ] **Step 1: Add a regression test for single parse accounting**

In `tests/http_forwarding_test.rs`, append:

```rust
#[tokio::test]
async fn generate_content_invalid_json_is_recorded_once_in_admin_logs() {
    let mut config = rust_sync_proxy::test_config();
    config.admin_password = "pw".to_string();
    let app = rust_sync_proxy::build_router(config);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"contents":["#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let auth = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode("user:pw")
    );
    let logs_response = app
        .oneshot(
            Request::builder()
                .uri("/admin/api/logs")
                .header("authorization", auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(logs_response.status(), StatusCode::OK);
    let body = to_bytes(logs_response.into_body(), usize::MAX).await.unwrap();
    let json_body: Value = serde_json::from_slice(&body).unwrap();
    let item = &json_body["items"][0];
    assert_eq!(item["statusCode"], 400);
    assert_eq!(item["errorStage"], "parse_request_json");
    assert_eq!(item["errorKind"], "invalid_json");
    assert_eq!(item["errorMessage"], "invalid request json body");
    assert!(item["requestParseMs"].as_i64().unwrap_or_default() >= 0);
}
```

At the top of `tests/http_forwarding_test.rs`, add:

```rust
use base64::Engine;
```

- [ ] **Step 2: Run the new test**

Run:

```bash
timeout 60s cargo test --test http_forwarding_test generate_content_invalid_json_is_recorded_once_in_admin_logs -- --nocapture
```

Expected after Task 1:

```text
test result: ok.
```

This test protects the admin-log behavior before changing the request forwarding signature.

- [ ] **Step 3: Change `forward_gemini_request` signature**

In `src/http/router.rs`, replace the function signature:

```rust
async fn forward_gemini_request(
    state: AppState,
    resolved: ResolvedUpstream,
    target_path: String,
    request: Request,
    request_log: RequestLogSnapshot,
) -> Result<(Response, AdminLogEntry), ForwardRequestFailure> {
```

with:

```rust
async fn forward_gemini_request(
    state: AppState,
    resolved: ResolvedUpstream,
    target_path: String,
    parts: axum::http::request::Parts,
    request_body: bytes::Bytes,
    body: Value,
    request_log: RequestLogSnapshot,
) -> Result<(Response, AdminLogEntry), ForwardRequestFailure> {
```

- [ ] **Step 4: Remove body reread and JSON reparse from `forward_gemini_request`**

At the start of `forward_gemini_request`, replace:

```rust
let request_parse_started = Instant::now();
let content_type_header = request.headers().get(CONTENT_TYPE).cloned();
let accept_header = request.headers().get(ACCEPT).cloned();
let request_query = request.uri().query().map(ToOwned::to_owned);
let admin_enabled = state.admin.is_some();
let request_body = to_bytes(request.into_body(), MAX_REQUEST_BODY_BYTES)
    .await
    .map_err(|err| {
        ForwardRequestFailure::new(
            anyhow!("failed to read request body: {err}"),
            request_log.base_entry(),
        )
    })?;
let mut admin_entry = request_log.base_entry();
let body: Value = match serde_json::from_slice(&request_body) {
    Ok(body) => body,
    Err(err) => {
        admin_entry.request_parse_ms = request_parse_started.elapsed().as_millis() as i64;
        return Err(ForwardRequestFailure::new(
            StructuredProxyError::new(
                "invalid request json body",
                "parse_request_json",
                "invalid_json",
                format!("invalid request json body: {err}"),
            ),
            admin_entry,
        ));
    }
};
```

with:

```rust
let content_type_header = parts.headers.get(CONTENT_TYPE).cloned();
let accept_header = parts.headers.get(ACCEPT).cloned();
let request_query = parts.uri.query().map(ToOwned::to_owned);
let admin_enabled = state.admin.is_some();
let mut admin_entry = request_log.base_entry();
```

Keep the existing `output_mode`, materialization, encoding, upstream send, and response handling code below this block.

- [ ] **Step 5: Update `model_action` call site**

In `model_action`, replace:

```rust
let request = Request::from_parts(parts, Body::from(request_body));

match forward_gemini_request(
    state.clone(),
    resolved,
    target_path,
    request,
    request_log.clone(),
)
```

with:

```rust
match forward_gemini_request(
    state.clone(),
    resolved,
    target_path,
    parts,
    request_body,
    parsed_body,
    request_log.clone(),
)
```

- [ ] **Step 6: Stop adding parse time twice**

In `model_action`, after successful `forward_gemini_request`, replace:

```rust
admin_entry.request_parse_ms += request_parse_ms;
```

with:

```rust
admin_entry.request_parse_ms = request_parse_ms;
```

In the error branch, replace:

```rust
admin_entry.request_parse_ms += request_parse_ms;
```

with:

```rust
admin_entry.request_parse_ms = request_parse_ms;
```

- [ ] **Step 7: Run router-related tests**

Run:

```bash
timeout 60s cargo test --test http_forwarding_test -- --nocapture
```

Expected:

```text
test result: ok.
```

- [ ] **Step 8: Commit Task 2**

Run:

```bash
git add src/http/router.rs tests/http_forwarding_test.rs
git commit -m "refactor: parse gemini request once"
```

---

### Task 3: Move-Based `keep_largest_inline_image`

**Files:**
- Modify: `tests/response_rewrite_test.rs`
- Modify: `src/response_rewrite.rs`

- [ ] **Step 1: Add a regression test that preserves text and URL-like inlineData**

In `tests/response_rewrite_test.rs`, after `keeps_only_largest_inline_image_per_candidate`, add:

```rust
#[test]
fn keep_largest_inline_image_preserves_text_and_url_inline_data() {
    let input = json!({
        "candidates": [{
            "content": {"parts": [
                {"text": "before"},
                {"inlineData": {"mimeType": "image/png", "data": "small"}},
                {"inlineData": {"mimeType": "image/png", "data": "https://img.example/kept.png"}},
                {"inlineData": {"mimeType": "image/png", "data": "largest-base64"}},
                {"text": "after"}
            ]}
        }]
    });

    let output = rust_sync_proxy::response_rewrite::keep_largest_inline_image(input);
    let parts = output["candidates"][0]["content"]["parts"]
        .as_array()
        .unwrap();

    assert_eq!(
        parts,
        &vec![
            json!({"text": "before"}),
            json!({"inlineData": {"mimeType": "image/png", "data": "https://img.example/kept.png"}}),
            json!({"inlineData": {"mimeType": "image/png", "data": "largest-base64"}}),
            json!({"text": "after"})
        ]
    );
}
```

- [ ] **Step 2: Run the new test and verify it fails**

Run:

```bash
timeout 60s cargo test --test response_rewrite_test keep_largest_inline_image_preserves_text_and_url_inline_data -- --nocapture
```

Expected:

```text
keep_largest_inline_image_preserves_text_and_url_inline_data ... FAILED
```

- [ ] **Step 3: Add helper functions in `src/response_rewrite.rs`**

Below `keep_largest_inline_image`, add:

```rust
fn inline_data_payload(part: &Value) -> Option<&str> {
    part.get("inlineData")
        .and_then(Value::as_object)
        .and_then(|inline_data| inline_data.get("data"))
        .and_then(Value::as_str)
}

fn is_base64_inline_image_part(part: &Value) -> bool {
    inline_data_payload(part).is_some_and(|data| !is_url_like(data))
}
```

- [ ] **Step 4: Replace clone-based retention with move-based filtering**

In `keep_largest_inline_image`, replace the `if let Some(best_index)` block with:

```rust
if let Some(best_index) = best_index {
    let original_parts = std::mem::take(parts);
    let mut retained = Vec::with_capacity(original_parts.len());

    for (index, part) in original_parts.into_iter().enumerate() {
        if !is_base64_inline_image_part(&part) || index == best_index {
            retained.push(part);
        }
    }

    *parts = retained;
}
```

Also replace the second-loop `is_inline_image` logic entirely; the helper decides whether a part is a base64 inline image.

- [ ] **Step 5: Run response rewrite tests**

Run:

```bash
timeout 60s cargo test --test response_rewrite_test -- --nocapture
```

Expected:

```text
test result: ok.
```

- [ ] **Step 6: Commit Task 3**

Run:

```bash
git add src/response_rewrite.rs tests/response_rewrite_test.rs
git commit -m "perf: avoid cloning inline image parts"
```

---

### Task 4: Avoid Copying Unchanged Optimized Images

**Files:**
- Modify: `src/image_io.rs`
- Modify: `src/response_materialize.rs`
- Modify: `tests/image_io_test.rs`
- Modify: `tests/response_materialize_test.rs`

- [ ] **Step 1: Add tests for unchanged optimization status**

In `tests/image_io_test.rs`, add:

```rust
#[test]
fn compression_result_marks_disabled_optimization_as_unchanged() {
    let original = vec![137, 80, 78, 71, 13, 10, 26, 10];
    let optimized = rust_sync_proxy::image_io::maybe_compress_png_bytes_result(
        &original,
        "image/png",
        false,
        1,
        97,
    )
    .unwrap();

    assert!(optimized.is_unchanged());
    assert_eq!(optimized.mime_type(), "image/png");
}

#[test]
fn compression_result_marks_non_png_as_unchanged() {
    let original = b"jpeg bytes";
    let optimized = rust_sync_proxy::image_io::maybe_compress_png_bytes_result(
        original,
        "image/jpeg",
        true,
        1,
        97,
    )
    .unwrap();

    assert!(optimized.is_unchanged());
    assert_eq!(optimized.mime_type(), "image/jpeg");
}
```

- [ ] **Step 2: Run the new tests and verify they fail**

Run:

```bash
timeout 60s cargo test --test image_io_test -- --nocapture
```

Expected:

```text
compression_result_marks_disabled_optimization_as_unchanged ... FAILED
compression_result_marks_non_png_as_unchanged ... FAILED
```

- [ ] **Step 3: Add `ImageOptimizationResult` to `src/image_io.rs`**

Below `OptimizedImage`, add:

```rust
#[derive(Clone, Debug)]
pub enum ImageOptimizationResult {
    Unchanged { mime_type: String },
    Reencoded { mime_type: String, bytes: Bytes },
}

impl ImageOptimizationResult {
    pub fn is_unchanged(&self) -> bool {
        matches!(self, Self::Unchanged { .. })
    }

    pub fn mime_type(&self) -> &str {
        match self {
            Self::Unchanged { mime_type } | Self::Reencoded { mime_type, .. } => mime_type,
        }
    }

    pub fn into_optimized_image(self, original: &[u8]) -> OptimizedImage {
        match self {
            Self::Unchanged { mime_type } => OptimizedImage {
                mime_type,
                bytes: Bytes::copy_from_slice(original),
            },
            Self::Reencoded { mime_type, bytes } => OptimizedImage { mime_type, bytes },
        }
    }
}
```

- [ ] **Step 4: Add the no-copy compression function**

In `src/image_io.rs`, add this function above `maybe_compress_png_bytes_with_options`:

```rust
pub fn maybe_compress_png_bytes_result(
    bytes: &[u8],
    mime_type: &str,
    enabled: bool,
    threshold_bytes: usize,
    jpeg_quality: u8,
) -> Result<ImageOptimizationResult> {
    let normalized_mime = mime_type.trim().to_ascii_lowercase();
    if !enabled || normalized_mime != "image/png" || bytes.len() <= threshold_bytes {
        return Ok(ImageOptimizationResult::Unchanged {
            mime_type: normalized_mime,
        });
    }

    let dynamic = image::load_from_memory_with_format(bytes, ImageFormat::Png)?;
    let rgb = dynamic.to_rgb8();
    let width = u16::try_from(rgb.width()).map_err(|_| anyhow!("image width too large"))?;
    let height = u16::try_from(rgb.height()).map_err(|_| anyhow!("image height too large"))?;

    let mut encoded = Vec::new();
    let mut encoder = JpegEncoder::new(&mut encoded, jpeg_quality);
    encoder.set_sampling_factor(SamplingFactor::R_4_4_4);
    encoder.encode(rgb.as_raw(), width, height, JpegColorType::Rgb)?;

    if encoded.len() >= bytes.len() {
        return Ok(ImageOptimizationResult::Unchanged {
            mime_type: normalized_mime,
        });
    }

    Ok(ImageOptimizationResult::Reencoded {
        mime_type: "image/jpeg".to_string(),
        bytes: Bytes::from(encoded),
    })
}
```

- [ ] **Step 5: Keep the existing compatibility wrapper**

Replace the body of `maybe_compress_png_bytes_with_options` with:

```rust
pub fn maybe_compress_png_bytes_with_options(
    bytes: &[u8],
    mime_type: &str,
    enabled: bool,
    threshold_bytes: usize,
    jpeg_quality: u8,
) -> Result<OptimizedImage> {
    maybe_compress_png_bytes_result(bytes, mime_type, enabled, threshold_bytes, jpeg_quality)
        .map(|result| result.into_optimized_image(bytes))
}
```

- [ ] **Step 6: Use no-copy semantics in response materialization**

In `src/response_materialize.rs`, inside `optimize_inline_data_images_with_options`, replace:

```rust
let optimized = crate::image_io::maybe_compress_png_bytes_with_options(
    &image_bytes,
    &entry.mime_type,
    enabled,
    threshold_bytes,
    jpeg_quality,
)?;
if optimized.mime_type == entry.mime_type
    && optimized.bytes.as_ref() == image_bytes.as_slice()
{
    continue;
}

replacements.insert(
    entry,
    InlineDataReplacement {
        mime_type: optimized.mime_type,
        data: STANDARD.encode(optimized.bytes),
    },
);
```

with:

```rust
let optimized = crate::image_io::maybe_compress_png_bytes_result(
    &image_bytes,
    &entry.mime_type,
    enabled,
    threshold_bytes,
    jpeg_quality,
)?;
let crate::image_io::ImageOptimizationResult::Reencoded { mime_type, bytes } = optimized else {
    continue;
};

replacements.insert(
    entry,
    InlineDataReplacement {
        mime_type,
        data: STANDARD.encode(bytes),
    },
);
```

- [ ] **Step 7: Run image and response materialize tests**

Run:

```bash
timeout 60s cargo test --test image_io_test --test response_materialize_test -- --nocapture
```

Expected:

```text
test result: ok.
```

- [ ] **Step 8: Commit Task 4**

Run:

```bash
git add src/image_io.rs src/response_materialize.rs tests/image_io_test.rs tests/response_materialize_test.rs
git commit -m "perf: avoid copying unchanged image optimizations"
```

---

### Task 5: Config Defaults And Admin Boundary Tests

**Files:**
- Modify: `src/lib.rs`
- Modify: `tests/config_test.rs`
- Modify: `tests/admin_test.rs`

- [ ] **Step 1: Add a test proving `test_config()` follows runtime defaults**

In `tests/config_test.rs`, after `defaults_match_runtime_expectations`, add:

```rust
#[test]
fn test_config_uses_runtime_defaults_except_test_api_key() {
    let runtime = rust_sync_proxy::config::Config::from_env_map(&HashMap::new()).unwrap();
    let test = rust_sync_proxy::test_config();

    assert_eq!(test.port, runtime.port);
    assert_eq!(test.upstream_base_url, runtime.upstream_base_url);
    assert_eq!(test.upstream_timeout, runtime.upstream_timeout);
    assert_eq!(test.upstream_connect_timeout, runtime.upstream_connect_timeout);
    assert_eq!(test.upstream_tcp_keepalive, runtime.upstream_tcp_keepalive);
    assert_eq!(test.upstream_pool_idle_timeout, runtime.upstream_pool_idle_timeout);
    assert_eq!(test.image_fetch_timeout, runtime.image_fetch_timeout);
    assert_eq!(test.upload_timeout, runtime.upload_timeout);
    assert_eq!(
        test.upload_tls_handshake_timeout,
        runtime.upload_tls_handshake_timeout
    );
    assert_eq!(test.inline_data_url_cache_ttl, runtime.inline_data_url_cache_ttl);
    assert_eq!(
        test.inline_data_url_background_fetch_total_timeout,
        runtime.inline_data_url_background_fetch_total_timeout
    );
    assert_eq!(test.upstream_api_key, "test-upstream-key");
}
```

- [ ] **Step 2: Run the new config test and verify it fails**

Run:

```bash
timeout 60s cargo test --test config_test test_config_uses_runtime_defaults_except_test_api_key -- --nocapture
```

Expected:

```text
test_config_uses_runtime_defaults_except_test_api_key ... FAILED
```

The failure should include `upload_timeout`, because `test_config()` currently uses 10s while runtime default is 20s.

- [ ] **Step 3: Make `test_config()` derive from runtime defaults**

In `src/lib.rs`, replace the whole `test_config()` body with:

```rust
pub fn test_config() -> Config {
    let mut config = Config::from_env_map(&std::collections::HashMap::new())
        .expect("empty config map should produce default test config");
    config.upstream_api_key = "test-upstream-key".to_string();
    config
}
```

Remove the now-unused import:

```rust
use std::time::Duration;
```

Keep the existing imports for `AtomicU64` and `Ordering`.

- [ ] **Step 4: Add admin auth boundary tests**

In `tests/admin_test.rs`, add:

```rust
#[tokio::test]
async fn admin_rejects_invalid_basic_auth_variants() {
    let mut config = rust_sync_proxy::test_config();
    config.admin_password = "pw".to_string();
    let app = rust_sync_proxy::build_router(config);

    let cases = [
        "Basic wrong-password",
        "Basic !!!not-base64!!!",
        "Basic dXNlcm5vY29sb24=",
        "Bearer dXNlcjpwdw==",
    ];

    for auth in cases {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/admin/api/logs")
                    .header("authorization", auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "auth={auth}");
    }
}

#[tokio::test]
async fn admin_routes_return_not_found_when_admin_is_disabled() {
    let app = rust_sync_proxy::build_router(rust_sync_proxy::test_config());

    for uri in ["/admin", "/admin/api/logs", "/admin/api/stats"] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "uri={uri}");
    }
}
```

- [ ] **Step 5: Add admin log capacity and order test**

In `tests/admin_test.rs`, add:

```rust
#[tokio::test]
async fn admin_log_buffer_keeps_latest_hundred_entries() {
    let admin = rust_sync_proxy::admin::AdminState::new("pw".to_string());

    for index in 0..101 {
        admin
            .record(rust_sync_proxy::admin::AdminLogEntry {
                path: format!("/request-{index}"),
                status_code: 200,
                ..Default::default()
            })
            .await;
    }

    let logs = admin.snapshot_logs().await;
    assert_eq!(logs.len(), 100);
    assert_eq!(logs[0].path, "/request-100");
    assert_eq!(logs[99].path, "/request-1");
    assert!(logs.iter().all(|entry| entry.path != "/request-0"));
}
```

- [ ] **Step 6: Run config and admin tests**

Run:

```bash
timeout 60s cargo test --test config_test --test admin_test -- --nocapture
```

Expected:

```text
test result: ok.
```

- [ ] **Step 7: Commit Task 5**

Run:

```bash
git add src/lib.rs tests/config_test.rs tests/admin_test.rs
git commit -m "test: align config defaults and admin boundaries"
```

---

### Task 6: Full Verification And Documentation Check

**Files:**
- Read: `docs/superpowers/specs/2026-04-26-code-quality-governance-design.md`
- Read: `docs/superpowers/plans/2026-04-26-code-quality-governance.md`
- Verify: entire repository

- [ ] **Step 1: Run formatting check**

Run:

```bash
timeout 60s cargo fmt --check
```

Expected:

```text
```

No output means formatting is already correct.

- [ ] **Step 2: Run the full test suite**

Run:

```bash
timeout 60s cargo test
```

Expected:

```text
test result: ok.
```

Multiple test binaries may print separate `test result: ok.` lines.

- [ ] **Step 3: Review changed files**

Run:

```bash
git status --short
git diff --stat HEAD
```

Expected:

```text
```

`git status --short` should show only intentionally uncommitted user files, such as `aiapi专篇(gptimage).md`, because each task commits its own changes.

- [ ] **Step 4: Confirm no forbidden scope expansion happened**

Run:

```bash
git show --stat --oneline HEAD
```

Manually confirm the implementation did not create new production modules for splitting `router.rs`, `admin.rs`, OpenAI image logic, or aiapidev logic.

- [ ] **Step 5: Final summary**

Report:

```text
完成内容：
- 客户端输入错误现在按 400/413 分类。
- 代理自身错误响应结构保持 source/stage/kind。
- Gemini 标准链路只解析一次请求 JSON。
- keep_largest_inline_image 避免克隆大 inlineData，并保留 URL-like inlineData。
- 未变化图片优化路径避免在响应物化中复制整图。
- test_config 与运行时默认值保持一致。
- admin 鉴权和日志容量边界有测试覆盖。

验证：
- timeout 60s cargo fmt --check
- timeout 60s cargo test
```
