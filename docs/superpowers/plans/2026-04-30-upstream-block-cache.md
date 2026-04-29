# Upstream Block Cache Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a 5-minute in-process cache that short-circuits repeated requests already rejected by upstream moderation or unsafe-content checks.

**Architecture:** Create a focused `upstream_block_cache` module for canonical request keys, TTL/LRU storage, and blockable-error classification. Wire it into `AppState` so each request checks the cache after JSON parsing and upstream resolution, and each upstream error path records blockable `400/502` responses before returning them. Cached hits reconstruct the original status/content-type/body and add `upstream_block_cache_hit:<reason>` to admin `errorDetail`.

**Tech Stack:** Rust 2024, axum 0.8, reqwest 0.12, tokio, serde_json with preserve_order, sha2, lru.

---

## File Structure

- Create `src/upstream_block_cache.rs`
  - Owns `UpstreamBlockCache`, `UpstreamBlockCacheKey`, `CachedBlockResponse`, `BlockCacheHit`, canonical JSON hashing, keyword classification, TTL and LRU.
- Modify `src/lib.rs`
  - Export the new module for tests and router use.
- Modify `src/config.rs`
  - Add `upstream_block_cache_ttl` and `upstream_block_cache_max_entries`.
  - Parse `UPSTREAM_BLOCK_CACHE_TTL_MS` and `UPSTREAM_BLOCK_CACHE_MAX_ENTRIES`.
- Modify `src/http/router.rs`
  - Store `Option<Arc<UpstreamBlockCache>>` in `AppState`.
  - Generate keys after upstream resolution.
  - Return cached responses before upstream work.
  - Store blockable upstream errors from standard, OpenAI image, and aiapidev paths.
- Modify `tests/config_test.rs`
  - Cover defaults, env override, and disabled settings.
- Modify `tests/http_forwarding_test.rs`
  - Cover standard Gemini and OpenAI image cache hits/misses.
  - Cover disabled cache and non-blockable error behavior.
  - Cover API-key-independent keying.

---

### Task 1: Config Fields And Defaults

**Files:**
- Modify: `src/config.rs`
- Test: `tests/config_test.rs`

- [ ] **Step 1: Write failing config tests**

Add these tests to `tests/config_test.rs` near the other env parsing tests:

```rust
#[test]
fn upstream_block_cache_defaults_to_five_minutes_and_1024_entries() {
    let cfg = rust_sync_proxy::config::Config::from_env_map(&HashMap::new()).unwrap();

    assert_eq!(
        cfg.upstream_block_cache_ttl,
        Duration::from_millis(300_000)
    );
    assert_eq!(cfg.upstream_block_cache_max_entries, 1024);
}

#[test]
fn upstream_block_cache_can_be_configured_from_env() {
    let env = HashMap::from([
        (
            "UPSTREAM_BLOCK_CACHE_TTL_MS".to_string(),
            "12345".to_string(),
        ),
        (
            "UPSTREAM_BLOCK_CACHE_MAX_ENTRIES".to_string(),
            "17".to_string(),
        ),
    ]);

    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();

    assert_eq!(cfg.upstream_block_cache_ttl, Duration::from_millis(12_345));
    assert_eq!(cfg.upstream_block_cache_max_entries, 17);
}

#[test]
fn upstream_block_cache_can_be_disabled_from_env() {
    let env = HashMap::from([
        ("UPSTREAM_BLOCK_CACHE_TTL_MS".to_string(), "0".to_string()),
        ("UPSTREAM_BLOCK_CACHE_MAX_ENTRIES".to_string(), "0".to_string()),
    ]);

    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();

    assert_eq!(cfg.upstream_block_cache_ttl, Duration::from_millis(0));
    assert_eq!(cfg.upstream_block_cache_max_entries, 0);
}
```

- [ ] **Step 2: Run config tests and verify failure**

Run:

```bash
timeout 60s cargo test --test config_test upstream_block_cache -- --nocapture
```

Expected: FAIL because `Config` does not yet contain `upstream_block_cache_ttl` and `upstream_block_cache_max_entries`.

- [ ] **Step 3: Add config constants and fields**

In `src/config.rs`, add constants near other defaults:

```rust
const DEFAULT_UPSTREAM_BLOCK_CACHE_TTL_MS: u64 = 300_000;
const DEFAULT_UPSTREAM_BLOCK_CACHE_MAX_ENTRIES: usize = 1024;
```

Add fields to `Config` after `upstream_pool_idle_timeout`:

```rust
pub upstream_block_cache_ttl: Duration,
pub upstream_block_cache_max_entries: usize,
```

Add assignments in `Config::from_env_map` after `upstream_pool_idle_timeout`:

```rust
upstream_block_cache_ttl: Duration::from_millis(parse_non_negative_u64_with_default(
    env_map.get("UPSTREAM_BLOCK_CACHE_TTL_MS"),
    DEFAULT_UPSTREAM_BLOCK_CACHE_TTL_MS,
)),
upstream_block_cache_max_entries: parse_non_negative_usize_with_default(
    env_map.get("UPSTREAM_BLOCK_CACHE_MAX_ENTRIES"),
    DEFAULT_UPSTREAM_BLOCK_CACHE_MAX_ENTRIES,
),
```

Update `defaults_match_runtime_expectations` in `tests/config_test.rs` with:

```rust
assert_eq!(
    cfg.upstream_block_cache_ttl,
    Duration::from_millis(300_000)
);
assert_eq!(cfg.upstream_block_cache_max_entries, 1024);
```

- [ ] **Step 4: Run config tests and verify pass**

Run:

```bash
timeout 60s cargo test --test config_test -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs tests/config_test.rs
git commit -m "feat: add upstream block cache config"
```

---

### Task 2: Upstream Block Cache Module

**Files:**
- Create: `src/upstream_block_cache.rs`
- Modify: `src/lib.rs`
- Test: `src/upstream_block_cache.rs`

- [ ] **Step 1: Write failing module tests**

Create `src/upstream_block_cache.rs` with tests first:

```rust
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use axum::http::{HeaderValue, StatusCode};
use bytes::Bytes;
use lru::LruCache;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_ignores_object_order_and_preserves_array_order() {
        let first = json!({
            "contents": [{
                "parts": [
                    {"text": "first"},
                    {"text": "second"}
                ]
            }],
            "generationConfig": {"temperature": 0.7, "topP": 0.9}
        });
        let second = json!({
            "generationConfig": {"topP": 0.9, "temperature": 0.7},
            "contents": [{
                "parts": [
                    {"text": "first"},
                    {"text": "second"}
                ]
            }]
        });
        let reversed_array = json!({
            "generationConfig": {"temperature": 0.7, "topP": 0.9},
            "contents": [{
                "parts": [
                    {"text": "second"},
                    {"text": "first"}
                ]
            }]
        });

        let first_key = UpstreamBlockCacheKey::new(
            "/v1beta/models/demo:generateContent",
            "https://upstream.example",
            &first,
        );
        let second_key = UpstreamBlockCacheKey::new(
            "/v1beta/models/demo:generateContent",
            "https://upstream.example",
            &second,
        );
        let reversed_key = UpstreamBlockCacheKey::new(
            "/v1beta/models/demo:generateContent",
            "https://upstream.example",
            &reversed_array,
        );

        assert_eq!(first_key, second_key);
        assert_ne!(first_key, reversed_key);
    }

    #[test]
    fn classifier_requires_400_or_502_and_known_keyword() {
        assert_eq!(
            classify_blockable_upstream_error(
                StatusCode::BAD_GATEWAY,
                br#"content blocked: {"error_code":"image_unsafe"}"#
            ),
            Some("image_unsafe")
        );
        assert_eq!(
            classify_blockable_upstream_error(
                StatusCode::BAD_REQUEST,
                b"Upstream moderation triggered: output_moderation"
            ),
            Some("upstream_moderation")
        );
        assert_eq!(
            classify_blockable_upstream_error(StatusCode::TOO_MANY_REQUESTS, b"image_unsafe"),
            None
        );
        assert_eq!(
            classify_blockable_upstream_error(StatusCode::BAD_GATEWAY, b"temporary upstream error"),
            None
        );
    }

    #[tokio::test]
    async fn cache_returns_hits_until_ttl_expires() {
        let cache = UpstreamBlockCache::new(Duration::from_millis(50), 8).unwrap();
        let key = UpstreamBlockCacheKey::new(
            "/v1/images/generations",
            "https://upstream.example",
            &json!({"prompt": "blocked"}),
        );
        cache
            .insert(
                key.clone(),
                CachedBlockResponse {
                    status: StatusCode::BAD_GATEWAY,
                    content_type: HeaderValue::from_static("application/json"),
                    body: Bytes::from_static(br#"{"error":{"message":"content blocked"}}"#),
                    reason: "content_blocked",
                },
            )
            .await;

        let hit = cache.get(&key).await.expect("cache hit before ttl");
        assert_eq!(hit.status, StatusCode::BAD_GATEWAY);
        assert_eq!(hit.reason, "content_blocked");
        assert_eq!(hit.body, Bytes::from_static(br#"{"error":{"message":"content blocked"}}"#));

        tokio::time::sleep(Duration::from_millis(70)).await;
        assert!(cache.get(&key).await.is_none());
    }

    #[tokio::test]
    async fn cache_evicts_least_recently_used_entry() {
        let cache = UpstreamBlockCache::new(Duration::from_secs(60), 2).unwrap();
        let key_a = UpstreamBlockCacheKey::new("/a", "https://upstream.example", &json!({"p": "a"}));
        let key_b = UpstreamBlockCacheKey::new("/b", "https://upstream.example", &json!({"p": "b"}));
        let key_c = UpstreamBlockCacheKey::new("/c", "https://upstream.example", &json!({"p": "c"}));
        let entry = |message: &'static str| CachedBlockResponse {
            status: StatusCode::BAD_GATEWAY,
            content_type: HeaderValue::from_static("application/json"),
            body: Bytes::from_static(message.as_bytes()),
            reason: "content_blocked",
        };

        cache.insert(key_a.clone(), entry("a")).await;
        cache.insert(key_b.clone(), entry("b")).await;
        assert!(cache.get(&key_a).await.is_some());
        cache.insert(key_c.clone(), entry("c")).await;

        assert!(cache.get(&key_a).await.is_some());
        assert!(cache.get(&key_b).await.is_none());
        assert!(cache.get(&key_c).await.is_some());
    }
}
```

- [ ] **Step 2: Run module tests and verify failure**

Run:

```bash
timeout 60s cargo test upstream_block_cache -- --nocapture
```

Expected: FAIL because the module types and functions are not implemented.

- [ ] **Step 3: Implement the module**

Replace the temporary file content with this implementation, keeping the tests at the bottom:

```rust
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use axum::http::{HeaderValue, StatusCode};
use bytes::Bytes;
use lru::LruCache;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UpstreamBlockCacheKey(String);

#[derive(Clone, Debug)]
pub struct CachedBlockResponse {
    pub status: StatusCode,
    pub content_type: HeaderValue,
    pub body: Bytes,
    pub reason: &'static str,
}

#[derive(Clone, Debug)]
pub struct BlockCacheHit {
    pub status: StatusCode,
    pub content_type: HeaderValue,
    pub body: Bytes,
    pub reason: &'static str,
}

#[derive(Clone, Debug)]
struct StoredBlockResponse {
    response: CachedBlockResponse,
    expires_at: Instant,
}

pub struct UpstreamBlockCache {
    ttl: Duration,
    entries: Mutex<LruCache<UpstreamBlockCacheKey, StoredBlockResponse>>,
}

impl UpstreamBlockCacheKey {
    pub fn new(path: &str, upstream_base_url: &str, request_body: &Value) -> Self {
        let canonical = canonicalize_json(request_body);
        let body_bytes = serde_json::to_vec(&canonical).unwrap_or_else(|_| b"null".to_vec());
        let mut hasher = Sha256::new();
        hasher.update(body_bytes);
        let body_hash = hex::encode(hasher.finalize());
        Self(format!("{path}\n{upstream_base_url}\n{body_hash}"))
    }
}

impl UpstreamBlockCache {
    pub fn new(ttl: Duration, max_entries: usize) -> Option<Self> {
        if ttl.is_zero() || max_entries == 0 {
            return None;
        }
        let capacity = NonZeroUsize::new(max_entries)?;
        Some(Self {
            ttl,
            entries: Mutex::new(LruCache::new(capacity)),
        })
    }

    pub async fn get(&self, key: &UpstreamBlockCacheKey) -> Option<BlockCacheHit> {
        let mut entries = self.entries.lock().await;
        let now = Instant::now();
        let Some(entry) = entries.get(key) else {
            return None;
        };
        if entry.expires_at <= now {
            entries.pop(key);
            return None;
        }
        Some(BlockCacheHit {
            status: entry.response.status,
            content_type: entry.response.content_type.clone(),
            body: entry.response.body.clone(),
            reason: entry.response.reason,
        })
    }

    pub async fn insert(&self, key: UpstreamBlockCacheKey, response: CachedBlockResponse) {
        let mut entries = self.entries.lock().await;
        entries.put(
            key,
            StoredBlockResponse {
                response,
                expires_at: Instant::now() + self.ttl,
            },
        );
    }
}

pub fn classify_blockable_upstream_error(
    status: StatusCode,
    body: &[u8],
) -> Option<&'static str> {
    if !matches!(status, StatusCode::BAD_REQUEST | StatusCode::BAD_GATEWAY) {
        return None;
    }
    let text = String::from_utf8_lossy(body).to_ascii_lowercase();
    if text.contains("image_unsafe") {
        return Some("image_unsafe");
    }
    if text.contains("upstream moderation triggered") {
        return Some("upstream_moderation");
    }
    if text.contains("content blocked") {
        return Some("content_blocked");
    }
    None
}

fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_json).collect()),
        Value::Object(map) => {
            let mut sorted = Map::new();
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            for key in keys {
                sorted.insert(key.clone(), canonicalize_json(&map[key]));
            }
            Value::Object(sorted)
        }
        _ => value.clone(),
    }
}
```

- [ ] **Step 4: Export the module**

Add this line to `src/lib.rs` with the other modules:

```rust
pub mod upstream_block_cache;
```

- [ ] **Step 5: Run module tests and verify pass**

Run:

```bash
timeout 60s cargo test upstream_block_cache -- --nocapture
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs src/upstream_block_cache.rs
git commit -m "feat: add upstream block cache module"
```

---

### Task 3: Router Cache Hit Short-Circuit

**Files:**
- Modify: `src/http/router.rs`
- Test: `tests/http_forwarding_test.rs`

- [ ] **Step 1: Write failing Gemini cache-hit test**

Add this route helper near the other mock handlers in `tests/http_forwarding_test.rs`:

```rust
async fn mock_generate_content_content_blocked(
    State(state): State<TestState>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> (StatusCode, Json<Value>) {
    let _ = mock_generate_content(State(state), headers, uri, body).await;
    (
        StatusCode::BAD_GATEWAY,
        Json(json!({
            "error": {
                "message": "content blocked: {\"error_code\":\"image_unsafe\",\"message\":\"unsafe\"}"
            }
        })),
    )
}
```

Add this test near `upstream_json_error_preserves_message_and_adds_proxy_metadata`:

```rust
#[tokio::test]
async fn generate_content_reuses_cached_upstream_block_error() {
    let state = TestState::default();
    let server = Router::new().route(
        "/v1beta/models/demo:generateContent",
        post(mock_generate_content_content_blocked),
    )
    .with_state(state.clone());
    let server_addr = spawn_server(server).await;

    let mut config = rust_sync_proxy::test_config();
    config.upstream_base_url = format!("http://{server_addr}");
    config.upstream_api_key = "first-key".to_string();
    let app = rust_sync_proxy::build_router(config);

    let body = json!({
        "contents": [{
            "parts": [{
                "text": "blocked prompt"
            }]
        }]
    })
    .to_string();

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::BAD_GATEWAY);
    let first_body = to_bytes(first.into_body(), usize::MAX).await.unwrap();

    let second = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent")
                .header(CONTENT_TYPE, "application/json")
                .header("x-goog-api-key", format!("http://{server_addr}|second-key"))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::BAD_GATEWAY);
    let second_body = to_bytes(second.into_body(), usize::MAX).await.unwrap();

    assert_eq!(second_body, first_body);
    let upstream_requests = state.upstream_requests.lock().await.clone();
    assert_eq!(upstream_requests.len(), 1);
    assert_eq!(upstream_requests[0].api_key, "first-key");
}
```

- [ ] **Step 2: Run the new test and verify failure**

Run:

```bash
timeout 60s cargo test --test http_forwarding_test generate_content_reuses_cached_upstream_block_error -- --nocapture
```

Expected: FAIL because both requests still reach upstream.

- [ ] **Step 3: Add cache to AppState and build helpers**

In `src/http/router.rs`, update imports:

```rust
use bytes::Bytes;
```

Add cache imports:

```rust
use crate::upstream_block_cache::{
    CachedBlockResponse, UpstreamBlockCache, UpstreamBlockCacheKey,
    classify_blockable_upstream_error,
};
```

Add this field to `AppState`:

```rust
upstream_block_cache: Option<Arc<UpstreamBlockCache>>,
```

In `build_router`, create and store it after `blob_runtime`:

```rust
let upstream_block_cache = UpstreamBlockCache::new(
    config.upstream_block_cache_ttl,
    config.upstream_block_cache_max_entries,
)
.map(Arc::new);
```

Add it to `AppState`:

```rust
upstream_block_cache,
```

Add helper functions near `raw_reqwest_response_with_body`:

```rust
fn response_from_block_cache_hit(hit: crate::upstream_block_cache::BlockCacheHit) -> Response {
    let mut response = Response::new(Body::from(hit.body));
    *response.status_mut() = hit.status;
    response.headers_mut().insert(CONTENT_TYPE, hit.content_type);
    response
}

fn block_cache_hit_entry(
    request_log: &RequestLogSnapshot,
    hit: &crate::upstream_block_cache::BlockCacheHit,
) -> AdminLogEntry {
    let mut entry = request_log.base_entry();
    entry.status_code = hit.status.as_u16();
    entry.error_source = "proxy".to_string();
    entry.error_stage = "upstream_block_cache".to_string();
    entry.error_kind = "cache_hit".to_string();
    entry.error_message = "upstream block cache hit".to_string();
    entry.error_detail = format!("upstream_block_cache_hit:{}", hit.reason);
    entry
}

async fn maybe_store_upstream_block_error(
    cache: Option<&Arc<UpstreamBlockCache>>,
    key: Option<&UpstreamBlockCacheKey>,
    status: StatusCode,
    content_type: HeaderValue,
    body: &[u8],
) {
    let Some(cache) = cache else {
        return;
    };
    let Some(key) = key else {
        return;
    };
    let Some(reason) = classify_blockable_upstream_error(status, body) else {
        return;
    };
    cache
        .insert(
            key.clone(),
            CachedBlockResponse {
                status,
                content_type,
                body: Bytes::copy_from_slice(body),
                reason,
            },
        )
        .await;
}
```

- [ ] **Step 4: Short-circuit Gemini requests before upstream work**

In `model_action`, after `resolved` succeeds and before `let request_parse_ms = ...`, add:

```rust
let block_cache_key = UpstreamBlockCacheKey::new(&request_path, &resolved.base_url, &parsed_body);
if let Some(cache) = state.upstream_block_cache.as_ref() {
    if let Some(hit) = cache.get(&block_cache_key).await {
        let mut admin_entry = block_cache_hit_entry(&request_log, &hit);
        admin_entry.created_at = created_at;
        admin_entry.method = request_method;
        admin_entry.path = request_path;
        admin_entry.query = request_query;
        admin_entry.remote_addr = remote_addr;
        admin_entry.is_stream = false;
        admin_entry.request_parse_ms = request_parse_started.elapsed().as_millis() as i64;
        admin_entry.duration_ms = started_at.elapsed().as_millis() as i64;
        let response = response_from_block_cache_hit(hit);
        return finalize_admin_response(&state, response, admin_entry).await;
    }
}
```

Pass the key into `forward_gemini_request`:

```rust
Some(block_cache_key),
```

Update the `forward_gemini_request` signature:

```rust
async fn forward_gemini_request(
    state: AppState,
    resolved: ResolvedUpstream,
    target_path: String,
    parts: axum::http::request::Parts,
    body: Value,
    request_log: RequestLogSnapshot,
    block_cache_key: Option<UpstreamBlockCacheKey>,
) -> Result<(Response, AdminLogEntry), ForwardRequestFailure> {
```

- [ ] **Step 5: Store standard Gemini upstream errors**

Update the `handle_non_stream_response` signature:

```rust
async fn handle_non_stream_response(
    upstream_response: reqwest::Response,
    output_mode: OutputMode,
    image_client: &reqwest::Client,
    fetch_service: Option<&Arc<InlineDataUrlFetchService>>,
    uploader: &Uploader,
    blob_runtime: &BlobRuntime,
    config: &Config,
    block_cache: Option<&Arc<UpstreamBlockCache>>,
    block_cache_key: Option<&UpstreamBlockCacheKey>,
) -> Result<(Response, ResponseStageDurations)> {
```

Update the call site in `forward_gemini_request`:

```rust
state.config.as_ref(),
state.upstream_block_cache.as_ref(),
block_cache_key.as_ref(),
```

Inside `handle_non_stream_response`, in the `if !status.is_success()` block, build bytes first and store:

```rust
let response_body_bytes = annotate_upstream_error_json(status, &body_bytes)
    .unwrap_or_else(|| body_bytes.to_vec());
let response_status = StatusCode::from_u16(status.as_u16())?;
maybe_store_upstream_block_error(
    block_cache,
    block_cache_key,
    response_status,
    content_type.clone(),
    &response_body_bytes,
)
.await;
let mut response = Response::new(Body::from(response_body_bytes));
*response.status_mut() = response_status;
response.headers_mut().insert(CONTENT_TYPE, content_type);
return Ok((
    response,
    ResponseStageDurations {
        response_process_ms: response_started.elapsed().as_millis() as i64,
        upload_ms: 0,
    },
));
```

- [ ] **Step 6: Run Gemini cache test and verify pass**

Run:

```bash
timeout 60s cargo test --test http_forwarding_test generate_content_reuses_cached_upstream_block_error -- --nocapture
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/http/router.rs tests/http_forwarding_test.rs
git commit -m "feat: cache blocked gemini upstream errors"
```

---

### Task 4: OpenAI Image Cache Coverage

**Files:**
- Modify: `src/http/router.rs`
- Test: `tests/http_forwarding_test.rs`

- [ ] **Step 1: Write failing OpenAI image tests**

Add this mock near the OpenAI image mocks:

```rust
async fn mock_openai_image_generation_image_unsafe(
    State(state): State<TestState>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> (StatusCode, Json<Value>) {
    let _ = mock_openai_image_generation(State(state), headers, uri, body).await;
    (
        StatusCode::BAD_GATEWAY,
        Json(json!({
            "error_code": "image_unsafe",
            "message": "The generated images appear to be unsafe. Try modifying the prompts or the seeds."
        })),
    )
}
```

Add this test:

```rust
#[tokio::test]
async fn image_generations_reuses_cached_upstream_block_error() {
    let state = TestState::default();
    let server = Router::new()
        .route(
            "/v1/images/generations",
            post(mock_openai_image_generation_image_unsafe),
        )
        .with_state(state.clone());
    let server_addr = spawn_server(server).await;

    let mut config = rust_sync_proxy::test_config();
    config.upstream_base_url = format!("http://{server_addr}");
    config.upstream_api_key = "env-key".to_string();
    let app = rust_sync_proxy::build_router(config);

    let body = json!({
        "model": "gpt-image-2",
        "prompt": "blocked image prompt",
        "response_format": "url"
    })
    .to_string();

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/images/generations")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::BAD_GATEWAY);
    let first_body = to_bytes(first.into_body(), usize::MAX).await.unwrap();

    let second = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/images/generations")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::BAD_GATEWAY);
    let second_body = to_bytes(second.into_body(), usize::MAX).await.unwrap();

    assert_eq!(second_body, first_body);
    assert_eq!(state.upstream_requests.lock().await.len(), 1);
}
```

Add disabled-cache test:

```rust
#[tokio::test]
async fn upstream_block_cache_ttl_zero_disables_cache() {
    let state = TestState::default();
    let server = Router::new()
        .route(
            "/v1/images/generations",
            post(mock_openai_image_generation_image_unsafe),
        )
        .with_state(state.clone());
    let server_addr = spawn_server(server).await;

    let mut config = rust_sync_proxy::test_config();
    config.upstream_base_url = format!("http://{server_addr}");
    config.upstream_api_key = "env-key".to_string();
    config.upstream_block_cache_ttl = std::time::Duration::from_millis(0);
    let app = rust_sync_proxy::build_router(config);

    let body = json!({
        "model": "gpt-image-2",
        "prompt": "blocked image prompt"
    })
    .to_string();

    for _ in 0..2 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/images/generations")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

    assert_eq!(state.upstream_requests.lock().await.len(), 2);
}
```

- [ ] **Step 2: Run OpenAI image tests and verify failure**

Run:

```bash
timeout 60s cargo test --test http_forwarding_test image_generations_reuses_cached_upstream_block_error -- --nocapture
timeout 60s cargo test --test http_forwarding_test upstream_block_cache_ttl_zero_disables_cache -- --nocapture
```

Expected: `image_generations_reuses_cached_upstream_block_error` FAILS because OpenAI image errors are not cached yet. `upstream_block_cache_ttl_zero_disables_cache` PASSES because disabled cache should leave both requests forwarded.

- [ ] **Step 3: Short-circuit OpenAI image requests**

In `image_generations_action`, after `normalized_body` succeeds and before `let request_parse_ms = ...`, add:

```rust
let block_cache_key =
    UpstreamBlockCacheKey::new(&request_path, &resolved.base_url, &normalized_body);
if let Some(cache) = state.upstream_block_cache.as_ref() {
    if let Some(hit) = cache.get(&block_cache_key).await {
        let mut admin_entry = block_cache_hit_entry(&request_log, &hit);
        admin_entry.created_at = created_at;
        admin_entry.method = request_method;
        admin_entry.path = request_path;
        admin_entry.query = request_query;
        admin_entry.remote_addr = remote_addr;
        admin_entry.is_stream = false;
        admin_entry.request_parse_ms = request_parse_started.elapsed().as_millis() as i64;
        admin_entry.duration_ms = started_at.elapsed().as_millis() as i64;
        let response = response_from_block_cache_hit(hit);
        return finalize_admin_response(&state, response, admin_entry).await;
    }
}
```

Pass the key into `forward_openai_image_request`:

```rust
Some(block_cache_key),
```

Update its signature:

```rust
async fn forward_openai_image_request(
    state: AppState,
    resolved: ResolvedUpstream,
    request_body: Value,
    request_query: Option<String>,
    request_log: RequestLogSnapshot,
    block_cache_key: Option<UpstreamBlockCacheKey>,
) -> Result<(Response, AdminLogEntry), ForwardRequestFailure> {
```

- [ ] **Step 4: Store OpenAI image non-success errors**

Update `handle_openai_image_response` signature:

```rust
async fn handle_openai_image_response(
    upstream_response: reqwest::Response,
    uploader: &Uploader,
    config: &Config,
    block_cache: Option<&Arc<UpstreamBlockCache>>,
    block_cache_key: Option<&UpstreamBlockCacheKey>,
) -> Result<(Response, ResponseStageDurations)> {
```

Update both call sites in `forward_openai_image_request`:

```rust
state.config.as_ref(),
state.upstream_block_cache.as_ref(),
block_cache_key.as_ref(),
```

Inside `handle_openai_image_response`, in the `if !status.is_success()` block:

```rust
let response_status = StatusCode::from_u16(status.as_u16())?;
maybe_store_upstream_block_error(
    block_cache,
    block_cache_key,
    response_status,
    content_type.clone(),
    &body_bytes,
)
.await;
let response = raw_reqwest_response_with_body(status, content_type, body_bytes.to_vec());
return Ok((
    response,
    ResponseStageDurations {
        response_process_ms: response_started.elapsed().as_millis() as i64,
        upload_ms: 0,
    },
));
```

- [ ] **Step 5: Run OpenAI image tests and verify pass**

Run:

```bash
timeout 60s cargo test --test http_forwarding_test image_generations_reuses_cached_upstream_block_error -- --nocapture
timeout 60s cargo test --test http_forwarding_test upstream_block_cache_ttl_zero_disables_cache -- --nocapture
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/http/router.rs tests/http_forwarding_test.rs
git commit -m "feat: cache blocked openai image errors"
```

---

### Task 5: Key Boundaries And Non-Blockable Errors

**Files:**
- Modify: `tests/http_forwarding_test.rs`

- [ ] **Step 1: Write key-boundary and non-blockable tests**

Add this mock near error mocks:

```rust
async fn mock_generate_content_bad_request_non_blockable(
    State(state): State<TestState>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> (StatusCode, Json<Value>) {
    let _ = mock_generate_content(State(state), headers, uri, body).await;
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "error": {
                "message": "missing required field"
            }
        })),
    )
}
```

Add these tests:

```rust
#[tokio::test]
async fn upstream_block_cache_does_not_match_different_request_body() {
    let state = TestState::default();
    let server = Router::new()
        .route(
            "/v1beta/models/demo:generateContent",
            post(mock_generate_content_content_blocked),
        )
        .with_state(state.clone());
    let server_addr = spawn_server(server).await;

    let mut config = rust_sync_proxy::test_config();
    config.upstream_base_url = format!("http://{server_addr}");
    config.upstream_api_key = "env-key".to_string();
    let app = rust_sync_proxy::build_router(config);

    for prompt in ["blocked prompt one", "blocked prompt two"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1beta/models/demo:generateContent")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "contents": [{
                                "parts": [{
                                    "text": prompt
                                }]
                            }]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

    assert_eq!(state.upstream_requests.lock().await.len(), 2);
}

#[tokio::test]
async fn upstream_block_cache_does_not_match_different_upstream_base_url() {
    let first_state = TestState::default();
    let first_server = Router::new()
        .route(
            "/v1beta/models/demo:generateContent",
            post(mock_generate_content_content_blocked),
        )
        .with_state(first_state.clone());
    let first_addr = spawn_server(first_server).await;

    let second_state = TestState::default();
    let second_server = Router::new()
        .route(
            "/v1beta/models/demo:generateContent",
            post(mock_generate_content_content_blocked),
        )
        .with_state(second_state.clone());
    let second_addr = spawn_server(second_server).await;

    let mut config = rust_sync_proxy::test_config();
    config.upstream_base_url = format!("http://{first_addr}");
    config.upstream_api_key = "env-key".to_string();
    let app = rust_sync_proxy::build_router(config);
    let body = json!({
        "contents": [{
            "parts": [{
                "text": "blocked prompt"
            }]
        }]
    })
    .to_string();

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::BAD_GATEWAY);

    let second = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent")
                .header(CONTENT_TYPE, "application/json")
                .header("x-goog-api-key", format!("http://{second_addr}|other-key"))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::BAD_GATEWAY);

    assert_eq!(first_state.upstream_requests.lock().await.len(), 1);
    assert_eq!(second_state.upstream_requests.lock().await.len(), 1);
}

#[tokio::test]
async fn upstream_block_cache_does_not_store_non_blockable_400() {
    let state = TestState::default();
    let server = Router::new()
        .route(
            "/v1beta/models/demo:generateContent",
            post(mock_generate_content_bad_request_non_blockable),
        )
        .with_state(state.clone());
    let server_addr = spawn_server(server).await;

    let mut config = rust_sync_proxy::test_config();
    config.upstream_base_url = format!("http://{server_addr}");
    config.upstream_api_key = "env-key".to_string();
    let app = rust_sync_proxy::build_router(config);
    let body = json!({
        "contents": [{
            "parts": [{
                "text": "missing field"
            }]
        }]
    })
    .to_string();

    for _ in 0..2 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1beta/models/demo:generateContent")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    assert_eq!(state.upstream_requests.lock().await.len(), 2);
}
```

- [ ] **Step 2: Run boundary tests**

Run:

```bash
timeout 60s cargo test --test http_forwarding_test upstream_block_cache -- --nocapture
```

Expected: PASS. Failure handling is constrained to these two code paths:

- Different body/base URL incorrectly hits cache: inspect `UpstreamBlockCacheKey::new` and `canonicalize_json`.
- Non-blockable `400` is cached: inspect `classify_blockable_upstream_error` and keep the allowed keywords exactly as specified.

- [ ] **Step 3: Commit**

```bash
git add tests/http_forwarding_test.rs
git commit -m "test: cover upstream block cache boundaries"
```

---

### Task 6: Admin Log Marker For Cache Hits

**Files:**
- Modify: `tests/http_forwarding_test.rs`
- Modify: `src/http/router.rs`

- [ ] **Step 1: Write failing admin marker test**

Add this test:

```rust
#[tokio::test]
async fn upstream_block_cache_hit_is_marked_in_admin_logs() {
    let state = TestState::default();
    let server = Router::new()
        .route(
            "/v1beta/models/demo:generateContent",
            post(mock_generate_content_content_blocked),
        )
        .with_state(state.clone());
    let server_addr = spawn_server(server).await;

    let mut config = rust_sync_proxy::test_config();
    config.upstream_base_url = format!("http://{server_addr}");
    config.upstream_api_key = "env-key".to_string();
    config.admin_password = "pw".to_string();
    let app = rust_sync_proxy::build_router(config);

    let body = json!({
        "contents": [{
            "parts": [{
                "text": "blocked prompt"
            }]
        }]
    })
    .to_string();

    for _ in 0..2 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1beta/models/demo:generateContent")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

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
    let body = to_bytes(logs_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json_body: Value = serde_json::from_slice(&body).unwrap();
    let items = json_body["items"].as_array().unwrap();

    assert_eq!(items.len(), 2);
    assert!(
        items[0]["errorDetail"]
            .as_str()
            .unwrap_or_default()
            .starts_with("upstream_block_cache_hit:")
    );
    assert_eq!(items[0]["statusCode"], 502);
}
```

- [ ] **Step 2: Run admin marker test**

Run:

```bash
timeout 60s cargo test --test http_forwarding_test upstream_block_cache_hit_is_marked_in_admin_logs -- --nocapture
```

Expected: PASS. The hit entry created by `block_cache_hit_entry` must retain `errorDetail` through `finalize_admin_response`.

- [ ] **Step 3: Commit**

```bash
git add src/http/router.rs tests/http_forwarding_test.rs
git commit -m "test: mark upstream block cache hits in admin logs"
```

---

### Task 7: aiapidev Error Storage Hooks

**Files:**
- Modify: `src/http/router.rs`
- Test: `src/upstream_block_cache.rs`, `tests/http_forwarding_test.rs`

- [ ] **Step 1: Add helper for cacheable JSON error responses**

In `src/http/router.rs`, add this helper near `proxy_error_response`:

```rust
async fn cacheable_json_error_response(
    status: StatusCode,
    value: Value,
    block_cache: Option<&Arc<UpstreamBlockCache>>,
    block_cache_key: Option<&UpstreamBlockCacheKey>,
) -> Response {
    let body = serde_json::to_vec(&value).unwrap_or_else(|_| {
        json!({"error": {"code": status.as_u16(), "message": "failed to encode error response"}})
            .to_string()
            .into_bytes()
    });
    let content_type = HeaderValue::from_static("application/json");
    maybe_store_upstream_block_error(block_cache, block_cache_key, status, content_type.clone(), &body).await;
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response.headers_mut().insert(CONTENT_TYPE, content_type);
    response
}
```

This helper is intentionally async because storing in the cache awaits the mutex.

- [ ] **Step 2: Thread cache key into aiapidev handlers**

Update `handle_aiapidev_response` signature:

```rust
async fn handle_aiapidev_response(
    resolved: &ResolvedUpstream,
    target_path: &str,
    request_query: Option<&str>,
    request_body: Value,
    output_mode: OutputMode,
    upstream_client: &reqwest::Client,
    image_client: &reqwest::Client,
    fetch_service: Option<&Arc<InlineDataUrlFetchService>>,
    config: &Config,
    block_cache: Option<&Arc<UpstreamBlockCache>>,
    block_cache_key: Option<&UpstreamBlockCacheKey>,
) -> Response {
```

Update its call site in the aiapidev branch of `forward_gemini_request`:

```rust
state.config.as_ref(),
state.upstream_block_cache.as_ref(),
block_cache_key.as_ref(),
```

Update `handle_aiapidev_openai_image_response` signature similarly:

```rust
async fn handle_aiapidev_openai_image_response(
    resolved: &ResolvedUpstream,
    request_query: Option<&str>,
    request_body: Value,
    upstream_client: &reqwest::Client,
    config: &Config,
    block_cache: Option<&Arc<UpstreamBlockCache>>,
    block_cache_key: Option<&UpstreamBlockCacheKey>,
) -> Response {
```

Update its call site:

```rust
state.config.as_ref(),
state.upstream_block_cache.as_ref(),
block_cache_key.as_ref(),
```

- [ ] **Step 3: Cache aiapidev create and poll raw upstream errors**

Update `raw_reqwest_response` to accept optional cache parameters:

```rust
async fn raw_reqwest_response(
    upstream_response: reqwest::Response,
    block_cache: Option<&Arc<UpstreamBlockCache>>,
    block_cache_key: Option<&UpstreamBlockCacheKey>,
) -> Response {
```

Inside it, after reading `body_bytes`, add:

```rust
let response_status =
    StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
maybe_store_upstream_block_error(
    block_cache,
    block_cache_key,
    response_status,
    content_type.clone(),
    &body_bytes,
)
.await;
```

Then return:

```rust
raw_reqwest_response_with_body(status, content_type, body_bytes.to_vec())
```

Update existing calls:

```rust
raw_reqwest_response(create_response, block_cache, block_cache_key).await
```

For poll helpers, update signatures so they can store raw non-retryable or final retry errors:

```rust
async fn poll_aiapidev_task(
    upstream_client: &reqwest::Client,
    resolved: &ResolvedUpstream,
    request_id: &str,
    poll_interval: Duration,
    block_cache: Option<&Arc<UpstreamBlockCache>>,
    block_cache_key: Option<&UpstreamBlockCacheKey>,
) -> std::result::Result<Value, Response> {
```

```rust
async fn poll_aiapidev_openai_image_task(
    upstream_client: &reqwest::Client,
    resolved: &ResolvedUpstream,
    request_id: &str,
    block_cache: Option<&Arc<UpstreamBlockCache>>,
    block_cache_key: Option<&UpstreamBlockCacheKey>,
) -> std::result::Result<Value, Response> {
```

Update their `raw_reqwest_response(response).await` calls to:

```rust
raw_reqwest_response(response, block_cache, block_cache_key).await
```

- [ ] **Step 4: Cache aiapidev task terminal failures**

In both aiapidev handlers, replace terminal task failure response:

```rust
return (
    StatusCode::BAD_GATEWAY,
    Json(json!({"error": {"code": 502, "message": message}})),
)
    .into_response();
```

with:

```rust
return cacheable_json_error_response(
    StatusCode::BAD_GATEWAY,
    json!({"error": {"code": 502, "message": message}}),
    block_cache,
    block_cache_key,
)
.await;
```

Do the same for normalization failures where `err.to_string()` may include blockable upstream messages:

```rust
return cacheable_json_error_response(
    StatusCode::BAD_GATEWAY,
    json!({"error": {"code": 502, "message": err.to_string()}}),
    block_cache,
    block_cache_key,
)
.await;
```

- [ ] **Step 5: Run compiler and focused tests**

Run:

```bash
timeout 60s cargo test --test http_forwarding_test upstream_block_cache -- --nocapture
```

Expected: PASS. The aiapidev create, poll, and terminal-failure paths compile with the new cache arguments.

- [ ] **Step 6: Commit**

```bash
git add src/http/router.rs
git commit -m "feat: cache aiapidev block errors"
```

---

### Task 8: Full Verification And Documentation Touch-Up

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Document configuration and behavior**

In `README.md`, under the error troubleshooting/configuration area, add this section:

```markdown
### 上游违规错误短期拦截缓存

代理会短期缓存明确的上游违规错误，避免下游重试机制反复请求同一违规内容。

默认行为：

- `UPSTREAM_BLOCK_CACHE_TTL_MS=300000`，即 5 分钟。
- `UPSTREAM_BLOCK_CACHE_MAX_ENTRIES=1024`。
- 设置任一值为 `0` 会关闭该机制。
- 缓存 key 为请求路径、上游 base URL 和规范化请求体摘要，不包含 API key。
- 只缓存 `400` 或 `502` 中包含以下关键词的错误：
  - `content blocked`
  - `image_unsafe`
  - `Upstream moderation triggered`

缓存命中时，代理返回首次错误的 status、content-type 和 body，不再请求上游。
admin 日志的 `errorDetail` 会包含 `upstream_block_cache_hit:<reason>`。
```

- [ ] **Step 2: Run formatting**

Run:

```bash
cargo fmt
```

Expected: no output or formatted files.

- [ ] **Step 3: Run full test suite**

Run:

```bash
timeout 60s cargo test -- --nocapture
```

Expected: PASS.

- [ ] **Step 4: Review changed files**

Run:

```bash
git diff --stat
git diff -- src/upstream_block_cache.rs src/config.rs src/http/router.rs tests/config_test.rs tests/http_forwarding_test.rs README.md
```

Expected:

- No changes to `.gitignore`.
- No broad router refactor beyond cache hooks.
- No cache storage for `429/500/503/504`.
- No API-key component in `UpstreamBlockCacheKey`.

- [ ] **Step 5: Commit**

```bash
git add README.md
git commit -m "docs: document upstream block cache"
```

- [ ] **Step 6: Final verification command**

Run:

```bash
timeout 60s cargo test -- --nocapture
```

Expected: PASS.

---

## Self-Review Notes

Spec coverage:

1. 5-minute default TTL and `0` disable are covered by Task 1.
2. `path + base_url + canonical request body hash` is covered by Task 2 and Task 5.
3. Key excludes API key and is tested by Task 3.
4. Gemini standard path is covered by Task 3.
5. OpenAI image path is covered by Task 4.
6. aiapidev create/poll/task-failure hooks are covered by Task 7.
7. Non-blockable `400/502` behavior is covered by Task 5.
8. Admin `errorDetail` marker is covered by Task 6.
9. Full `timeout 60s cargo test` verification is covered by Task 8.

Implementation constraints:

1. Keep `upstream_block_cache.rs` independent from router internals.
2. Keep cache storage fail-open: if there is no cache or no key, continue normal response behavior.
3. Do not cache ordinary rate-limit or transient upstream errors.
4. Do not alter cached response body, status, or content-type on hit.
