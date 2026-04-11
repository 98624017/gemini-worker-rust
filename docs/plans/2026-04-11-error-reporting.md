# Proxy Error Modeling and Reporting Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Introduce a unified proxy error model that preserves upstream error semantics while returning stable structured errors for proxy-generated failures and recording richer diagnostics in admin logs.

**Architecture:** Add a small error modeling layer in `src/http/router.rs` that converts proxy-generated failures into a stable downstream JSON shape and enriches `AdminLogEntry` with structured error metadata. Preserve upstream non-2xx responses as the source of truth, but normalize their bookkeeping so admin can distinguish upstream failures from proxy failures.

**Tech Stack:** Rust 2024, Axum, Reqwest, Serde JSON, existing admin log UI, Cargo tests

---

### Task 1: 定义错误模型边界与失败测试

**Files:**
- Modify: `src/http/router.rs`
- Modify: `tests/router_test.rs`

**Step 1: 写失败测试**

```rust
#[test]
fn proxy_error_body_contains_source_stage_and_kind() {
    let body = rust_sync_proxy::http::router::proxy_error_json(
        502,
        "failed to decode upstream response body",
        "proxy",
        "decode_upstream_body",
        "body_decode_failed",
    );
    assert_eq!(body["error"]["source"], "proxy");
    assert_eq!(body["error"]["stage"], "decode_upstream_body");
    assert_eq!(body["error"]["kind"], "body_decode_failed");
}
```

**Step 2: 运行测试确认失败**

Run: `timeout 60s ~/.cargo/bin/cargo test proxy_error_body_contains_source_stage_and_kind -- --nocapture`

Expected: FAIL，因为统一错误构造尚不存在。

**Step 3: 写最小实现**

- 在 `src/http/router.rs` 增加统一错误结构生成逻辑
- 先只实现纯函数或辅助函数，不接业务路径

**Step 4: 运行测试确认通过**

Run: `timeout 60s ~/.cargo/bin/cargo test proxy_error_body_contains_source_stage_and_kind -- --nocapture`

Expected: PASS

**Step 5: Commit**

```bash
git add src/http/router.rs tests/router_test.rs
git commit -m "refactor: add structured proxy error model"
```

### Task 2: 接入标准渠道本层错误出口

**Files:**
- Modify: `src/http/router.rs`
- Modify: `tests/http_forwarding_test.rs`

**Step 1: 写失败测试**

```rust
#[tokio::test]
async fn standard_proxy_body_decode_failure_returns_structured_proxy_error() {
    // 模拟已返回响应头但 body 解码失败的上游
    // 断言下游返回 code/message/source/stage/kind
}
```

**Step 2: 运行测试确认失败**

Run: `timeout 60s ~/.cargo/bin/cargo test standard_proxy_body_decode_failure_returns_structured_proxy_error -- --nocapture`

Expected: FAIL，当前仍返回底层原始错误字符串。

**Step 3: 写最小实现**

- 把标准渠道这些本层错误统一映射到结构化错误：
  - `read_request_body`
  - `parse_request_json`
  - `materialize_request`
  - `encode_request_body`
  - `connect_upstream`
  - `read_upstream_headers`
  - `read_upstream_body`
  - `decode_upstream_body`
  - `parse_upstream_json`
  - `rewrite_upstream_response`
  - `finalize_response`
- 对 message 使用稳定文案
- 保留底层原始 detail 供 admin 使用

**Step 4: 运行目标测试确认通过**

Run: `timeout 60s ~/.cargo/bin/cargo test standard_proxy_body_decode_failure_returns_structured_proxy_error -- --nocapture`

Expected: PASS

**Step 5: 跑相关回归**

Run: `timeout 60s ~/.cargo/bin/cargo test http_forwarding_test -- --nocapture`

Expected: PASS

**Step 6: Commit**

```bash
git add src/http/router.rs tests/http_forwarding_test.rs
git commit -m "fix: structure standard proxy-generated errors"
```

### Task 3: 统一上游显式错误透传元信息

**Files:**
- Modify: `src/http/router.rs`
- Modify: `tests/http_forwarding_test.rs`

**Step 1: 写失败测试**

```rust
#[tokio::test]
async fn upstream_non_2xx_error_is_marked_as_upstream_source() {
    // 模拟上游 429 JSON 错误
    // 断言状态码保留，body 语义保留，admin 元信息标记 source=upstream
}
```

**Step 2: 运行测试确认失败**

Run: `timeout 60s ~/.cargo/bin/cargo test upstream_non_2xx_error_is_marked_as_upstream_source -- --nocapture`

Expected: FAIL，因为当前没有统一的 upstream/source bookkeeping。

**Step 3: 写最小实现**

- 为上游显式错误建立统一记录逻辑
- 保留上游状态码和主体语义
- 为 admin 记录 `source=upstream`、`stage=upstream_response`、`kind=upstream_error`

**Step 4: 运行测试确认通过**

Run: `timeout 60s ~/.cargo/bin/cargo test upstream_non_2xx_error_is_marked_as_upstream_source -- --nocapture`

Expected: PASS

**Step 5: Commit**

```bash
git add src/http/router.rs tests/http_forwarding_test.rs
git commit -m "refactor: classify upstream error passthrough"
```

### Task 4: 接入 `aiapidev` 本层错误出口

**Files:**
- Modify: `src/http/router.rs`
- Modify: `tests/router_test.rs`

**Step 1: 写失败测试**

```rust
#[tokio::test]
async fn aiapidev_poll_timeout_returns_structured_proxy_error() {
    // 断言 source=proxy stage=aiapidev_poll_task kind=timeout
}

#[tokio::test]
async fn aiapidev_invalid_json_returns_parse_stage_error() {
    // create 或 poll 必需 JSON 解析失败
}
```

**Step 2: 运行测试确认失败**

Run: `timeout 60s ~/.cargo/bin/cargo test aiapidev_poll_timeout_returns_structured_proxy_error -- --nocapture`

Expected: FAIL，因为当前只返回旧风格 message。

**Step 3: 写最小实现**

- 把 `aiapidev` 的 create / poll / parse / rewrite / finalize 失败映射到统一结构
- 保留“连续 5 次后返回最后一次上游错误”的既有行为
- 仅增强本层错误出口，不破坏上游错误透传

**Step 4: 运行目标测试确认通过**

Run: `timeout 60s ~/.cargo/bin/cargo test aiapidev -- --nocapture`

Expected: PASS

**Step 5: Commit**

```bash
git add src/http/router.rs tests/router_test.rs
git commit -m "fix: structure aiapidev proxy-generated errors"
```

### Task 5: 扩展 admin 日志结构与基础展示

**Files:**
- Modify: `src/admin.rs`
- Modify: `src/http/router.rs`
- Modify: `tests/admin_test.rs`

**Step 1: 写失败测试**

```rust
#[tokio::test]
async fn admin_log_records_error_source_stage_kind_and_detail() {
    // 触发一条代理错误
    // 断言 AdminLogEntry 新字段存在且值正确
}
```

**Step 2: 运行测试确认失败**

Run: `timeout 60s ~/.cargo/bin/cargo test admin_log_records_error_source_stage_kind_and_detail -- --nocapture`

Expected: FAIL，因为 `AdminLogEntry` 尚无这些字段。

**Step 3: 写最小实现**

- 在 `AdminLogEntry` 增加：
  - `error_source`
  - `error_stage`
  - `error_kind`
  - `error_message`
  - `error_detail`
  - `upstream_status_code`
  - `upstream_error_body`
- 在 `finalize_admin_response` 和错误出口填充这些字段
- 在 admin 详情页增加错误区块和上游错误摘录显示

**Step 4: 运行目标测试确认通过**

Run: `timeout 60s ~/.cargo/bin/cargo test admin_test -- --nocapture`

Expected: PASS

**Step 5: Commit**

```bash
git add src/admin.rs src/http/router.rs tests/admin_test.rs
git commit -m "feat: enrich admin logs for proxy error diagnosis"
```

### Task 6: 同步 README 与回归验证

**Files:**
- Modify: `README.md`

**Step 1: 写文档断言**

手工核对 `README.md` 当前尚未描述新的错误结构字段和透传原则。

**Step 2: 写最小实现**

- 在 `README.md` 补充：
  - 上游错误透传原则
  - 代理错误结构字段
  - admin 错误诊断字段说明

**Step 3: 运行回归**

Run: `timeout 60s ~/.cargo/bin/cargo test --tests -- --nocapture`

Expected: PASS

**Step 4: 跑 Python 脚本测试**

Run: `python3 -m unittest scripts/test_docker_aiapidev_regression.py`

Expected: PASS

**Step 5: Commit**

```bash
git add README.md
git commit -m "docs: describe structured proxy error reporting"
```

### Task 7: 最终人工验收

**Files:**
- Modify: `README.md`

**Step 1: 本地人工验证错误样例**

手工确认至少 3 类输出：

- 标准渠道本层错误
- 上游显式业务错误
- `aiapidev` 本层轮询错误

**Step 2: 核对 admin 展示**

检查 admin 页面详情是否能看到：

- `error_source`
- `error_stage`
- `error_kind`
- `error_message`
- `error_detail`

**Step 3: 记录剩余风险**

- 某些 `reqwest` 底层错误字符串在不同版本仍可能变化
- 第一版未引入 trace id
- 第一版不做 admin 高级筛选

**Step 4: 最终提交**

```bash
git status
```

Expected: 仅剩本次相关变更，工作树干净或符合预期

