# Rust 惯用模式与性能改进设计

日期: 2026-04-08
状态: 已批准

## 目标

提升代码质量：消除不必要的内存分配，采用更惯用的 Rust 类型和模式，优化并发数据结构。

## 改动清单

### 1. 消除不必要的 clone / to_string / to_vec

- **router.rs**: 在解构 request 前提取 method 避免二次 clone；延迟 to_string 到写入 AdminLogEntry 时；提取所需 header 值而非 clone 整个 HeaderMap
- **cache.rs**: `CachedInlineData.mime_type` 从 `String` 改为 `Arc<str>`；inflight 查找用 `&str` key，仅 insert 时 to_string
- **upload.rs**: 梳理 data 转换链确保只发生一次 to_vec；Part::bytes 优先传 Bytes；错误消息只截取前 200 字节
- **request_rewrite.rs**: raw_url 用 &str 引用传递

原则：只改确定不影响语义的 clone。跨 .await 需要 Send + 'static 的 owned 值保留。

### 2. u64 毫秒 → Duration

config.rs 中所有超时字段从 `u64` 改为 `std::time::Duration`：
- `image_fetch_timeout_ms` → `image_fetch_timeout`
- `image_upload_timeout_ms` → `image_upload_timeout`
- `upstream_timeout_ms` → `upstream_timeout`
- `background_fetch_wait_timeout_ms` → `background_fetch_wait_timeout`

解析层用 `Duration::from_millis(parse_u64_with_default(...))`，消费方直接传 Duration。

### 3. BTreeSet → HashSet

- `request_rewrite.rs`: unique_urls 从 `BTreeSet<String>` 改为 `HashSet<String>`（顺序无关）
- `admin.rs`: `redact_inline_data_and_collect_image_urls` 改用 `Vec<String>` + `HashSet<String>` 辅助去重（保持插入序，与 Go 版行为一致）

### 4. cache.rs LRU 用 `lru` crate 替换 VecDeque

新增依赖 `lru = "0.12"`。

`DiskCacheLru` 从 `VecDeque<String>` + `HashMap<String, u64>` 改为 `lru::LruCache<String, u64>`（unbounded 模式，按 total_bytes/max_bytes 自行管理淘汰）。

操作复杂度：touch O(n)→O(1)，remove O(n)→O(1)。

### 5. 减少 Mutex 锁持有时间

cache.rs inflight 逻辑从"查找→判断→插入"单个长临界区拆为两个短临界区 + 双重检查：
- Phase 1: 获取锁 → 查找 → clone Arc → 释放锁 → 无锁等待
- Phase 2: 获取锁 → 双重检查 → 插入 → 释放锁 → 无锁 spawn

### 6. AtomicI64 Relaxed ordering 注释

确认 Relaxed 是正确的（独立统计计数器，不参与同步链）。为每个原子操作添加注释说明理由。

## 新增依赖

- `lru = "0.12"` — 零依赖，成熟的 O(1) LRU 缓存实现

## 验证标准

- `cargo build` 编译通过
- `cargo test` 全量通过
- `cargo clippy` 无 warning
