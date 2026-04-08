# Rust 惯用模式与性能改进实施计划

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 消除不必要的内存分配，采用惯用 Rust 类型与模式，优化并发数据结构，零功能回归。

**Architecture:** 6 项改动按依赖关系排序：Duration 重构 → 集合类型替换 → LRU crate → Clone 消除 → Mutex 优化 → 注释。每个 Task 独立可编译可测试。

**Tech Stack:** Rust 2024, lru 0.12, std::time::Duration, std::collections::HashSet

---

## 依赖关系

```
Task 1 (Duration) ──┐
Task 2 (HashSet)    ├── Task 4 (Clone) ── Task 5 (Mutex) ── Task 6 (Comments)
Task 3 (lru crate) ─┘
```

Tasks 1/2/3 互相独立，可并行。Task 4 依赖 1+3（字段名和结构变化）。Task 5 依赖 4。Task 6 最后。

---

### Task 1: u64 毫秒 → Duration

**Files:**
- Modify: `src/config.rs`
- Modify: `src/lib.rs` (`test_config()`)
- Modify: `src/http/router.rs` (`build_http_client`, `log_slow_request`)
- Modify: `src/cache.rs` (`InlineDataUrlFetchService::from_config`)
- Modify: `tests/config_test.rs`

**Step 1: 修改 config.rs — 字段类型与解析**

将以下字段从 `u64` 改为 `Duration`，去掉 `_ms` 后缀：

```rust
// src/config.rs 顶部新增
use std::time::Duration;

// Config struct 中替换字段：
pub slow_log_threshold: Duration,           // was slow_log_threshold_ms: u64
pub image_fetch_timeout: Duration,          // was image_fetch_timeout_ms: u64
pub image_tls_handshake_timeout: Duration,  // was image_tls_handshake_timeout_ms: u64
pub inline_data_url_cache_ttl: Duration,    // was inline_data_url_cache_ttl_ms: u64
pub inline_data_url_background_fetch_wait_timeout: Duration, // was ..._ms: u64
pub inline_data_url_background_fetch_total_timeout: Duration, // was ..._ms: u64
pub upload_timeout: Duration,               // was upload_timeout_ms: u64
pub upload_tls_handshake_timeout: Duration, // was upload_tls_handshake_timeout_ms: u64
```

在 `from_env_map` 中用 `Duration::from_millis(...)` 包装解析结果：

```rust
slow_log_threshold: Duration::from_millis(parse_non_negative_u64_with_default(
    env_map.get("SLOW_LOG_THRESHOLD_MS"),
    DEFAULT_SLOW_LOG_THRESHOLD_MS,
)),
image_fetch_timeout: Duration::from_millis(parse_positive_u64_with_default(
    env_map.get("IMAGE_FETCH_TIMEOUT_MS"),
    DEFAULT_IMAGE_FETCH_TIMEOUT_MS,
)),
// ... 同理其他字段
```

注意 `inline_data_url_background_fetch_wait_timeout` 的默认值依赖 `image_fetch_timeout`，需先计算 raw ms 再包 Duration：

```rust
let image_fetch_timeout_ms_raw = parse_positive_u64_with_default(
    env_map.get("IMAGE_FETCH_TIMEOUT_MS"),
    DEFAULT_IMAGE_FETCH_TIMEOUT_MS,
);
// ...
image_fetch_timeout: Duration::from_millis(image_fetch_timeout_ms_raw),
// ...
inline_data_url_background_fetch_wait_timeout: Duration::from_millis(
    parse_wait_timeout_ms(
        env_map.get("INLINE_DATA_URL_BACKGROUND_FETCH_WAIT_TIMEOUT_MS"),
        image_fetch_timeout_ms_raw,
    ),
),
```

**Step 2: 修改 lib.rs — test_config()**

```rust
pub fn test_config() -> Config {
    Config {
        // ...
        slow_log_threshold: Duration::from_millis(100_000),
        image_fetch_timeout: Duration::from_millis(20_000),
        image_tls_handshake_timeout: Duration::from_millis(15_000),
        inline_data_url_cache_ttl: Duration::from_millis(3_600_000),
        inline_data_url_background_fetch_wait_timeout: Duration::from_millis(20_000),
        inline_data_url_background_fetch_total_timeout: Duration::from_millis(90_000),
        upload_timeout: Duration::from_millis(10_000),
        upload_tls_handshake_timeout: Duration::from_millis(10_000),
        // ...
    }
}
```

顶部加 `use std::time::Duration;`。

**Step 3: 修改 router.rs — build_http_client 和 log_slow_request**

`build_http_client` 签名改为接受 `Duration`：

```rust
fn build_http_client(
    timeout: Duration,
    tls_handshake_timeout: Duration,
    insecure_skip_verify: bool,
) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(tls_handshake_timeout)
        .danger_accept_invalid_certs(insecure_skip_verify)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}
```

调用处：

```rust
let image_client = build_http_client(
    config.image_fetch_timeout,
    config.image_tls_handshake_timeout,
    config.image_fetch_insecure_skip_verify,
);
let upload_client = build_http_client(
    config.upload_timeout,
    config.upload_tls_handshake_timeout,
    config.upload_insecure_skip_verify,
);
```

`log_slow_request`：

```rust
fn log_slow_request(config: &Config, entry: &AdminLogEntry) {
    if config.slow_log_threshold.is_zero() {
        return;
    }
    if entry.duration_ms < config.slow_log_threshold.as_millis() as i64 {
        return;
    }
    // ...
}
```

顶部加 `use std::time::Duration;`。

**Step 4: 修改 cache.rs — from_config**

```rust
// 原来的 Duration::from_millis 包装不再需要
let disk_cache = if !config.inline_data_url_cache_dir.trim().is_empty()
    && !config.inline_data_url_cache_ttl.is_zero()
    && config.inline_data_url_cache_max_bytes > 0
{
    Some(Arc::new(DiskCache::new(
        PathBuf::from(config.inline_data_url_cache_dir.clone()),
        config.inline_data_url_cache_ttl,  // 直接传 Duration
        config.inline_data_url_cache_max_bytes,
    )))
} else { None };

// ...
wait_timeout: config.inline_data_url_background_fetch_wait_timeout,
total_timeout: config.inline_data_url_background_fetch_total_timeout,
```

条件判断 `config.inline_data_url_background_fetch_total_timeout_ms == 0` 改为 `config.inline_data_url_background_fetch_total_timeout.is_zero()`。

**Step 5: 修改 tests/config_test.rs**

```rust
use std::time::Duration;

// defaults_match_go_proxy_expectations:
assert_eq!(cfg.slow_log_threshold, Duration::from_millis(100_000));
assert_eq!(cfg.image_fetch_timeout, Duration::from_millis(20_000));
assert_eq!(cfg.upload_timeout, Duration::from_millis(10_000));
assert_eq!(cfg.image_tls_handshake_timeout, Duration::from_millis(15_000));
assert_eq!(cfg.upload_tls_handshake_timeout, Duration::from_millis(10_000));

// background_fetch_wait_timeout_defaults_to_image_fetch_timeout:
assert_eq!(cfg.image_fetch_timeout, Duration::from_millis(3456));
assert_eq!(cfg.inline_data_url_background_fetch_wait_timeout, Duration::from_millis(3456));
```

**Step 6: 编译验证**

Run: `cargo build 2>&1`
Expected: 编译成功

Run: `cargo test 2>&1`
Expected: 全部测试 PASS

Run: `cargo clippy 2>&1`
Expected: 无 warning

**Step 7: Commit**

```bash
git add src/config.rs src/lib.rs src/http/router.rs src/cache.rs tests/config_test.rs
git commit -m "refactor: replace u64 timeout fields with Duration for type safety"
```

---

### Task 2: BTreeSet → HashSet

**Files:**
- Modify: `src/request_rewrite.rs`
- Modify: `src/admin.rs`

**Step 1: request_rewrite.rs — BTreeSet → HashSet**

```rust
// 行 1: 替换 import
use std::collections::{HashSet, HashMap};
// 删除 BTreeSet

// 行 42: 替换类型
let mut unique_urls = HashSet::new();

// 行 47: 更新 walk 签名
fn walk(
    node: &Value,
    unique_urls: &mut HashSet<String>,
    total_refs: &mut usize,
) -> Result<()> {
```

**Step 2: admin.rs — BTreeSet → Vec + HashSet 去重**

```rust
// 行 1: 替换 import
use std::collections::{HashSet, VecDeque};
// 删除 BTreeSet

// redact_inline_data_and_collect_image_urls 函数:
fn redact_inline_data_and_collect_image_urls(root: &mut Value) -> Vec<String> {
    let mut urls = Vec::new();
    let mut seen = HashSet::new();

    fn walk(node: &mut Value, urls: &mut Vec<String>, seen: &mut HashSet<String>) {
        match node {
            Value::Object(map) => {
                for key in ["inlineData", "inline_data"] {
                    if let Some(Value::Object(inline)) = map.get_mut(key) {
                        if let Some(Value::String(data)) = inline.get("data") {
                            let trimmed = data.trim().to_string();
                            if is_image_url(&trimmed) {
                                if seen.insert(trimmed.clone()) {
                                    urls.push(trimmed);
                                }
                            } else if !trimmed.is_empty() {
                                inline.insert(
                                    "data".to_string(),
                                    Value::String(format!("[base64 omitted len={}]", data.len())),
                                );
                            }
                        }
                    }
                }
                for child in map.values_mut() {
                    walk(child, urls, seen);
                }
            }
            Value::Array(items) => {
                for child in items {
                    walk(child, urls, seen);
                }
            }
            _ => {}
        }
    }

    walk(root, &mut urls, &mut seen);
    urls
}
```

**Step 3: 编译验证**

Run: `cargo build 2>&1`
Expected: 编译成功

Run: `cargo test 2>&1`
Expected: 全部测试 PASS

注意: `admin_log_collects_proxy_and_http_image_urls` 测试断言 image_urls 的顺序。BTreeSet 是排序的，改为 Vec+HashSet 后顺序变为插入序。确认测试中的期望顺序与 JSON 中出现的先后一致（`https://img.example/a.png` 先出现，`/proxy/image?u=abc` 后出现）。如果测试失败，调整期望顺序。

**Step 4: Commit**

```bash
git add src/request_rewrite.rs src/admin.rs
git commit -m "refactor: replace BTreeSet with HashSet for O(1) lookups"
```

---

### Task 3: lru crate 替换 MemoryCache 的 VecDeque

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/cache.rs`

**Step 1: 添加 lru 依赖**

```toml
# Cargo.toml [dependencies] 添加
lru = "0.12"
```

**Step 2: 替换 MemoryCacheInner**

删除 `MemoryEntry`、`MemoryCacheInner` 结构体和 `move_to_back`、`remove_from_order` 函数。

替换为：

```rust
use lru::LruCache;
use std::num::NonZeroUsize;

struct MemoryCacheInner {
    items: LruCache<String, MemoryEntry>,
    max_bytes: u64,
    cur_bytes: u64,
}

#[derive(Clone)]
struct MemoryEntry {
    mime_type: String,
    bytes: Bytes,
    size: u64,
}
```

**Step 3: 替换 MemoryCache 实现**

```rust
impl MemoryCache {
    fn new(max_bytes: u64) -> Self {
        Self {
            inner: Mutex::new(MemoryCacheInner {
                // unbounded: 我们自己按 bytes 管理淘汰
                items: LruCache::unbounded(),
                max_bytes,
                cur_bytes: 0,
            }),
        }
    }

    async fn get(&self, url: &str) -> Option<CachedInlineData> {
        let mut guard = self.inner.lock().await;
        let item = guard.items.get(url)?;  // LruCache::get 自动提升
        Some(CachedInlineData {
            mime_type: item.mime_type.clone(),
            bytes: item.bytes.clone(),
        })
    }

    async fn set(&self, url: &str, value: &CachedInlineData) {
        let mut guard = self.inner.lock().await;
        let size = value.bytes.len() as u64;
        if size > guard.max_bytes {
            return;
        }
        // 移除旧值
        if let Some(previous) = guard.items.pop(url) {
            guard.cur_bytes = guard.cur_bytes.saturating_sub(previous.size);
        }
        // 淘汰直到有空间
        while guard.cur_bytes + size > guard.max_bytes {
            if let Some((_key, evicted)) = guard.items.pop_lru() {
                guard.cur_bytes = guard.cur_bytes.saturating_sub(evicted.size);
            } else {
                break;
            }
        }
        guard.items.put(
            url.to_string(),
            MemoryEntry {
                mime_type: value.mime_type.clone(),
                bytes: value.bytes.clone(),
                size,
            },
        );
        guard.cur_bytes += size;
    }
}
```

**Step 4: 删除废弃代码**

删除 `move_to_back` 和 `remove_from_order` 函数（行 478-487）。

如果 `VecDeque` import 不再被使用（`AdminLogBuffer` 在 admin.rs 中使用 VecDeque，但 cache.rs 可能不再需要），清理 import。

**Step 5: 编译验证**

Run: `cargo build 2>&1`
Expected: 编译成功

Run: `cargo test 2>&1`
Expected: 全部测试 PASS

**Step 6: Commit**

```bash
git add Cargo.toml src/cache.rs
git commit -m "refactor: replace VecDeque LRU with lru crate for O(1) operations"
```

---

### Task 4: 消除不必要的 clone

**Files:**
- Modify: `src/http/router.rs`
- Modify: `src/cache.rs`
- Modify: `src/upload.rs`

**Step 1: router.rs — 减少 clone**

1. `build_http_client` 已在 Task 1 中改为 Duration，无需额外改动。

2. 行 166 `request.method().to_string()` → 因为 method 只用于写入 AdminLogEntry.method（String 类型），这个 to_string 不可避免。但可以提前提取避免 borrow 冲突：

```rust
// 行 94 附近，在解构 request 之前提取所有需要的值
let request_method = request.method().to_string();
let request_path = request.uri().path().to_string();
let request_query = request.uri().query().unwrap_or_default().to_string();
let remote_addr = request
    .headers()
    .get("x-forwarded-for")
    .and_then(|value| value.to_str().ok())
    .unwrap_or_default()
    .to_string();
```

然后删除行 166 的 `let request_method = request.method().to_string();`，因为已在上面提前提取。

3. 行 211 `request_headers.clone()` — 提取所需 header 而非 clone 整个 HeaderMap：

```rust
// forward_gemini_request 签名不变，但内部改为提取
let content_type_header = request.headers().get(CONTENT_TYPE).cloned();
let accept_header = request.headers().get(ACCEPT).cloned();
let request_body = to_bytes(request.into_body(), MAX_REQUEST_BODY_BYTES)
    .await
    .map_err(|err| anyhow!("failed to read request body: {err}"))?;
// ...
if let Some(value) = content_type_header {
    upstream_request = upstream_request.header(CONTENT_TYPE, value);
}
if let Some(value) = accept_header {
    upstream_request = upstream_request.header(ACCEPT, value);
}
```

删除 `let request_headers = request.headers().clone();` 和后续对 `request_headers` 的引用。

**Step 2: cache.rs — CachedInlineData.mime_type → Arc<str>**

```rust
#[derive(Clone, Debug)]
pub struct CachedInlineData {
    pub mime_type: Arc<str>,
    pub bytes: Bytes,
}
```

所有构造处更新：
- `mime_type: fetched.mime_type.clone()` → `mime_type: Arc::from(fetched.mime_type.as_str())`（当从 FetchedInlineData 转换时）
- `mime_type: meta.mime_type` → `mime_type: Arc::from(meta.mime_type.as_str())`（从 DiskMeta 读取时）
- `published_result` 返回处：`mime_type: hit.mime_type.clone()` → 保持（Arc clone 只增计数）

`FetchResult.mime_type` 保持为 `String`（面向外部接口），在转换时用 `hit.mime_type.to_string()`。

```rust
#[derive(Clone, Debug)]
pub struct FetchResult {
    pub mime_type: String,  // 保持 String，外部接口
    pub bytes: Bytes,
    pub from_cache: bool,
}
```

转换处：
```rust
Ok(FetchResult {
    mime_type: hit.mime_type.to_string(),
    bytes: hit.bytes,
    from_cache: true,
})
```

DiskCache set 中 `value.mime_type.clone()` → `value.mime_type.to_string()`（DiskMeta 需要 String）。

**Step 3: upload.rs — 减少 to_vec**

`upload_to_uguu` 和 `upload_to_kefan` 中 `data.to_vec()`:
- `Part::bytes()` 接受 `impl Into<Cow<'static, [u8]>>`，`&[u8]` 不行，必须 owned。但可以用 `Bytes::copy_from_slice(data)` 然后 `Part::stream(bytes)` —— 实际上 `Part::bytes(data.to_vec())` 是最直接的方式，且 multipart form 需要 owned 数据。这里的 `to_vec()` 是必要的。

`upload_r2` 中 `let body = data.to_vec();`：
- 需要 owned 数据计算 sha256 和发送 body。to_vec 是必要的。
- 但可以共享 body 引用：sha256_hex 只需 `&[u8]`，`self.client.put(...).body(body)` 消耗 Vec。当前代码已经是这样做的。

结论：upload.rs 的 `to_vec()` 是必要的，不改。

`upload_image_with_mode` 中多次 `data.to_vec()` 和 `mime_type.to_string()`：

```rust
// 当前：每个分支都 to_vec + to_string
ImageHostMode::R2ThenLegacy => {
    match r2_uploader(data.to_vec(), mime_type.to_string()).await {
        Ok(result) => Ok(result),
        Err(_) => legacy_uploader(data.to_vec(), mime_type.to_string()).await,
    }
}
```

R2ThenLegacy 分支最坏情况两次 to_vec。改为预分配：

```rust
ImageHostMode::R2ThenLegacy => {
    let owned_data = data.to_vec();
    let owned_mime = mime_type.to_string();
    match r2_uploader(owned_data.clone(), owned_mime.clone()).await {
        Ok(result) => Ok(result),
        Err(_) => legacy_uploader(owned_data, owned_mime).await,
    }
}
```

只在失败回退时 clone 一次（而非两次 to_vec）。其他分支同理。

**Step 4: 编译验证**

Run: `cargo build 2>&1`
Expected: 编译成功

Run: `cargo test 2>&1`
Expected: 全部测试 PASS

**Step 5: Commit**

```bash
git add src/http/router.rs src/cache.rs src/upload.rs
git commit -m "perf: eliminate unnecessary clones and allocations"
```

---

### Task 5: 减少 Mutex 锁持有时间

**Files:**
- Modify: `src/cache.rs`

**Step 1: 重构 InlineDataUrlFetchService::fetch 的 inflight 逻辑**

将现有的单个长临界区（行 179-229）拆为两个短临界区 + 双重检查：

```rust
pub async fn fetch(self: &Arc<Self>, raw_url: &str) -> Result<FetchResult> {
    // --- Memory/disk cache lookups (unchanged) ---
    if let Some(cache) = &self.memory_cache {
        if let Some(hit) = cache.get(raw_url).await {
            return Ok(FetchResult {
                mime_type: hit.mime_type.to_string(),
                bytes: hit.bytes,
                from_cache: true,
            });
        }
    }
    if let Some(cache) = &self.disk_cache {
        if let Some(hit) = cache.get(raw_url).await? {
            if let Some(memory) = &self.memory_cache {
                memory.set(raw_url, &hit).await;
            }
            return Ok(FetchResult {
                mime_type: hit.mime_type.to_string(),
                bytes: hit.bytes,
                from_cache: true,
            });
        }
    }

    // --- Phase 1: 短临界区 — 查找已有 inflight 任务 ---
    let existing = {
        let inflight = self.inflight.lock().await;
        inflight.get(raw_url).map(Arc::clone)
    }; // 锁立即释放

    if let Some(task) = existing {
        return self.wait_for_task(&task).await;
    }

    // --- Phase 2: 短临界区 — 双重检查 + 插入 ---
    let task = {
        let mut inflight = self.inflight.lock().await;
        // 双重检查: 可能在 Phase 1 释放锁后被其他线程插入
        if let Some(task) = inflight.get(raw_url) {
            let task = Arc::clone(task);
            drop(inflight); // 显式释放
            return self.wait_for_task(&task).await;
        }
        if inflight.len() >= self.max_inflight {
            drop(inflight);
            let fetched = self.direct_fetch(raw_url).await?;
            let cached = CachedInlineData {
                mime_type: Arc::from(fetched.mime_type.as_str()),
                bytes: fetched.bytes.clone(),
            };
            self.store_in_caches(raw_url, &cached).await;
            return Ok(FetchResult {
                mime_type: fetched.mime_type,
                bytes: fetched.bytes,
                from_cache: false,
            });
        }
        let task = Arc::new(FetchTask {
            notify: Notify::new(),
            result: Mutex::new(None),
        });
        inflight.insert(raw_url.to_string(), Arc::clone(&task));
        task
    }; // 锁释放

    // --- 无锁: spawn 后台任务 ---
    let service = Arc::clone(self);
    let url = raw_url.to_string();
    let task_for_spawn = Arc::clone(&task);
    tokio::spawn(async move {
        let result = service
            .direct_fetch(&url)
            .await
            .map(|fetched| CachedInlineData {
                mime_type: Arc::from(fetched.mime_type.as_str()),
                bytes: fetched.bytes,
            });
        let published = result
            .as_ref()
            .map(Clone::clone)
            .map_err(|err| err.to_string());
        let mut guard = task_for_spawn.result.lock().await;
        *guard = Some(published);
        drop(guard);
        task_for_spawn.notify.notify_waiters();
        service.inflight.lock().await.remove(&url);
        if let Ok(hit) = &result {
            service.store_in_caches(&url, hit).await;
        }
    });

    self.wait_for_task(&task).await
}
```

**Step 2: 提取 wait_for_task 辅助方法**

```rust
async fn wait_for_task(self: &Arc<Self>, task: &Arc<FetchTask>) -> Result<FetchResult> {
    let notified = task.notify.notified();
    if let Some(result) = Self::published_result(task).await? {
        return Ok(result);
    }

    let wait_timeout = if self.wait_timeout.is_zero() {
        self.total_timeout
    } else {
        self.wait_timeout
    };

    if tokio::time::timeout(wait_timeout, notified).await.is_err() {
        if let Some(result) = Self::published_result(task).await? {
            return Ok(result);
        }
        return Err(BackgroundFetchWaitTimeoutError {
            wait_timeout,
            total_timeout: self.total_timeout,
        }
        .into());
    }

    match Self::published_result(task).await? {
        Some(result) => Ok(result),
        None => Err(anyhow!("background fetch finished without result")),
    }
}
```

**Step 3: 编译验证**

Run: `cargo build 2>&1`
Expected: 编译成功

Run: `cargo test 2>&1`
Expected: 全部测试 PASS

**Step 4: Commit**

```bash
git add src/cache.rs
git commit -m "perf: reduce Mutex lock holding time in inflight dedup"
```

---

### Task 6: AtomicI64 Relaxed ordering 注释

**Files:**
- Modify: `src/http/router.rs`
- Modify: `src/admin.rs`

**Step 1: router.rs — 添加注释**

在 `finalize_admin_response` 中的原子操作前添加注释：

```rust
// Relaxed: 独立统计计数器，不与其他原子操作构成同步链。
// 读端（admin stats API）可接受最终一致。
stats
    .total_requests
    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
```

同理在 `error_requests` 和 `total_duration_ms` 的 fetch_add 前添加同样注释。

cache_observer 闭包中的 `cache_hits` 也加注释：

```rust
let cache_observer = admin_stats.map(|stats| {
    Arc::new(move |_raw_url: &str, from_cache: bool| {
        if from_cache {
            // Relaxed: 独立统计计数器，读端可接受最终一致。
            stats
                .cache_hits
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }) as Arc<dyn Fn(&str, bool) + Send + Sync>
});
```

**Step 2: admin.rs — 添加注释**

在 `admin_stats_response` 和 `record` 中的原子操作前添加注释：

```rust
pub async fn record(&self, mut entry: AdminLogEntry) {
    // Relaxed: 单调递增 ID 生成，fetch_add 本身保证原子性，
    // 不需要与其他操作同步。
    let id = self.logs.next_id.fetch_add(1, Ordering::Relaxed) + 1;
    entry.id = id;
    // ...
}
```

```rust
pub fn admin_stats_response(state: &AdminState) -> Response {
    let stats = state.stats();
    // Relaxed: 统计快照，读端可接受最终一致。
    Json(AdminStatsPayload {
        total_requests: stats.total_requests.load(Ordering::Relaxed),
        // ...
    })
    .into_response()
}
```

**Step 3: 编译验证**

Run: `cargo build 2>&1`
Expected: 编译成功

Run: `cargo test 2>&1`
Expected: 全部测试 PASS

**Step 4: Commit**

```bash
git add src/http/router.rs src/admin.rs
git commit -m "docs: annotate Relaxed ordering rationale on atomic counters"
```
