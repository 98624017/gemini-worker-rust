# Proxy Extra Overhead Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 建立“本层额外增加的耗时”分阶段基线，并在不明显抬高峰值内存的前提下，完成第一轮图片重路径性能优化。

**Architecture:** 先补分阶段观测和 benchmark，对比“直连上游”和“经过本层”的差值，得到本层额外耗时基线；再只做低内存代价的首轮优化，最后用同一套样本复测。实现上优先复用现有 admin stats、benchmark 脚本和请求/响应处理链路，避免新起一套完全平行的性能框架。

**Tech Stack:** Rust 2024, axum 0.8, tokio, serde_json, Python 3, 现有 benchmark 脚本与 admin stats

---

### Task 1: 锁定“本层额外耗时”观测字段

**Files:**
- Modify: `src/admin.rs`
- Modify: `src/http/router.rs`
- Modify: `tests/admin_test.rs`

**Step 1: Write the failing test**

- 新增 admin/stats 或 admin/logs 测试，断言新增的分阶段耗时字段存在
- 至少覆盖：
  - `requestParseMs`
  - `requestImagePrepareMs`
  - `upstreamBuildMs`
  - `responseProcessMs`
  - `uploadMs`

**Step 2: Run test to verify it fails**

Run: `timeout 60s ~/.cargo/bin/cargo test --test admin_test -- --nocapture`
Expected: FAIL，因为这些字段尚未记录或返回。

**Step 3: Write minimal implementation**

- 在请求生命周期里补阶段计时
- 把阶段耗时挂进 admin log / stats 输出
- 保持默认路径开销最小，不引入重格式化

**Step 4: Run test to verify it passes**

Run: `timeout 60s ~/.cargo/bin/cargo test --test admin_test -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add src/admin.rs src/http/router.rs tests/admin_test.rs
git commit -m "feat: record staged proxy overhead metrics"
```

### Task 2: 锁定 benchmark 的“直连 vs 经代理”对照能力

**Files:**
- Modify: `scripts/benchmark_docker_mock_upstream.py`
- Modify: `scripts/test_benchmark_docker_mock_upstream.py`
- Modify: `README.md`

**Step 1: Write the failing test**

- 新增 Python 单测，要求 benchmark helper 能同时生成：
  - 直连上游请求
  - 经过代理请求
- 要求输出产物中包含：
  - `direct_total_ms`
  - `proxy_total_ms`
  - `proxy_overhead_ms`

**Step 2: Run test to verify it fails**

Run: `python3 -m unittest scripts/test_benchmark_docker_mock_upstream.py`
Expected: FAIL，因为当前 benchmark 还没有“差值 = 本层额外耗时”的对照输出。

**Step 3: Write minimal implementation**

- 扩展 benchmark 脚本
- 固定四组图片重路径样本：
  - miss/base64
  - hit/base64
  - miss/output=url
  - hit/output=url
- 产出 JSON/CSV 对照结果

**Step 4: Run test to verify it passes**

Run: `python3 -m unittest scripts/test_benchmark_docker_mock_upstream.py`
Expected: PASS

**Step 5: Commit**

```bash
git add scripts/benchmark_docker_mock_upstream.py scripts/test_benchmark_docker_mock_upstream.py README.md
git commit -m "feat: benchmark proxy extra overhead against direct upstream"
```

### Task 3: 建立第一版图片重路径基线报告

**Files:**
- Modify: `docs/plans/2026-04-09-generate-content-memory-redesign-benchmark-notes.md`
- Create: `benchmark-output/<timestamp>/...`（运行产物，不入库）

**Step 1: Run baseline benchmark**

Run:

```bash
python3 scripts/benchmark_docker_mock_upstream.py --help
```

确认脚本参数支持分场景输出后，再跑四组样本 benchmark。

**Step 2: Record baseline**

- 记录四组样本的：
  - `P50/P95 proxy_overhead_ms`
  - 各阶段耗时
  - 峰值 RSS
  - spill 统计

**Step 3: Update notes**

- 把当前基线写入 benchmark notes
- 明确第一轮优化要优先打哪一段

**Step 4: Commit**

```bash
git add docs/plans/2026-04-09-generate-content-memory-redesign-benchmark-notes.md
git commit -m "docs: record proxy extra overhead baseline"
```

### Task 4: 第一轮优化包 1 — 快路径裁剪与重复工作消除

**Files:**
- Modify: `src/http/router.rs`
- Modify: `src/request_materialize.rs`
- Modify: `src/request_encode.rs`
- Modify: `src/response_rewrite.rs`
- Modify: `tests/http_forwarding_test.rs`
- Modify: `tests/request_materialize_test.rs`
- Modify: `tests/response_rewrite_test.rs`

**Step 1: Write the failing test**

- 新增或扩展测试，锁定：
  - 轻路径不会误走重图片处理逻辑
  - 请求/响应侧不会重复扫描或重复改写同一结构

**Step 2: Run test to verify it fails**

Run: `timeout 60s ~/.cargo/bin/cargo test --test http_forwarding_test --test request_materialize_test --test response_rewrite_test -- --nocapture`
Expected: FAIL，因为当前仍存在可裁剪的重路径通用开销。

**Step 3: Write minimal implementation**

- 快路径提前返回
- 合并重复扫描
- 删除无必要的中间态和重复判断

**Step 4: Run focused tests**

Run: `timeout 60s ~/.cargo/bin/cargo test --test http_forwarding_test --test request_materialize_test --test response_rewrite_test -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add src/http/router.rs src/request_materialize.rs src/request_encode.rs src/response_rewrite.rs tests/http_forwarding_test.rs tests/request_materialize_test.rs tests/response_rewrite_test.rs
git commit -m "perf: trim heavy-path overhead in request and response flow"
```

### Task 5: 第一轮优化包 2 — 请求侧缓存命中路径瘦身

**Files:**
- Modify: `src/cache.rs`
- Modify: `src/request_materialize.rs`
- Modify: `tests/request_cache_test.rs`

**Step 1: Write the failing test**

- 新增测试锁定：
  - 缓存命中路径不做多余转换
  - 缓存命中路径不重复分配大对象
  - 请求侧缓存收益不会串到响应侧语义

**Step 2: Run test to verify it fails**

Run: `timeout 60s ~/.cargo/bin/cargo test --test request_cache_test -- --nocapture`
Expected: FAIL，因为当前缓存命中路径仍有可减重开销。

**Step 3: Write minimal implementation**

- 缩短缓存命中路径
- 减少命中后的封装/复制
- 保持峰值内存不明显上升

**Step 4: Run focused tests**

Run: `timeout 60s ~/.cargo/bin/cargo test --test request_cache_test -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add src/cache.rs src/request_materialize.rs tests/request_cache_test.rs
git commit -m "perf: shorten request image cache hit path"
```

### Task 6: 第一轮优化包 3 — `output=url` 上传前准备减重

**Files:**
- Modify: `src/response_materialize.rs`
- Modify: `src/upload.rs`
- Modify: `tests/upload_mode_test.rs`
- Modify: `tests/response_materialize_test.rs`

**Step 1: Write the failing test**

- 新增测试锁定：
  - 上传前不会重复 materialize / 重复构造大对象
  - `output=url` 行为保持不变

**Step 2: Run test to verify it fails**

Run: `timeout 60s ~/.cargo/bin/cargo test --test upload_mode_test --test response_materialize_test -- --nocapture`
Expected: FAIL，因为当前上传前路径还未按“额外耗时最小化”收敛。

**Step 3: Write minimal implementation**

- 瘦身上传前准备链路
- 避免重复对象构造和重复 bytes 搬运

**Step 4: Run focused tests**

Run: `timeout 60s ~/.cargo/bin/cargo test --test upload_mode_test --test response_materialize_test -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add src/response_materialize.rs src/upload.rs tests/upload_mode_test.rs tests/response_materialize_test.rs
git commit -m "perf: reduce output url upload preparation overhead"
```

### Task 7: 用同一套样本复测并做收益/代价判断

**Files:**
- Modify: `docs/plans/2026-04-09-generate-content-memory-redesign-benchmark-notes.md`

**Step 1: Run full verification**

Run:

```bash
timeout 60s ~/.cargo/bin/cargo test --tests -- --nocapture
python3 -m unittest scripts/test_benchmark_docker_mock_upstream.py
python3 scripts/benchmark_docker_mock_upstream.py --help
```

然后复跑四组 benchmark 样本。

**Step 2: Compare baseline vs optimized**

- 对比：
  - `proxy_overhead_ms P50/P95`
  - 分阶段耗时
  - 峰值 RSS
  - spill 统计

**Step 3: Make the decision**

- 若耗时明显下降且内存基本不变：接受
- 若耗时大幅下降但内存上涨：单独记录并请人判断
- 若耗时收益一般但内存明显反弹：回退该优化

**Step 4: Update notes**

- 记录第一轮优化收益
- 明确是否进入第二轮极限优化

**Step 5: Commit**

```bash
git add docs/plans/2026-04-09-generate-content-memory-redesign-benchmark-notes.md
git commit -m "docs: record first-pass proxy overhead optimization results"
```
