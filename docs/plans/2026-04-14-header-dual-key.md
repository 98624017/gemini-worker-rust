# Header Dual Key Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 在 Rust 版中实现仅限请求头覆盖的双 key 上游路由，并保持标准上游 / `aiapidev` 分叉逻辑不变。

**Architecture:** 先读取请求 JSON，再根据 header token 与 `generationConfig.imageConfig.imageSize` 选择最终上游。上游选择完成后，继续复用既有请求转发与特殊上游逻辑，不引入配置层改动。

**Tech Stack:** Rust, axum, serde_json, reqwest, 现有集成测试

---

### Task 1: 补齐上游选择 RED 测试

**Files:**
- Modify: `tests/upstream_auth_test.rs`

**Step 1: Write the failing test**

- 新增测试覆盖：
  - `4k/4K` 选择第二组 `<baseUrl>|<apiKey>`
  - 非 `4k` 选择第一组
  - token 含逗号但格式非法时返回错误

**Step 2: Run test to verify it fails**

Run: `timeout 60s ~/.cargo/bin/cargo test --test upstream_auth_test -- --nocapture`
Expected: FAIL，因为当前仅支持单组 `baseUrl|apiKey`

**Step 3: Write minimal implementation**

- 在 `src/upstream.rs` 增加按 `imageSize` 选择上游的解析函数
- 区分 `401 Missing upstream apiKey` 与 `400 malformed dual upstream token`

**Step 4: Run test to verify it passes**

Run: `timeout 60s ~/.cargo/bin/cargo test --test upstream_auth_test -- --nocapture`
Expected: PASS

### Task 2: 接入请求链路并保持后续分叉不变

**Files:**
- Modify: `src/http/router.rs`

**Step 1: Write the failing test**

- 在集成测试里新增：
  - 双 key 且 `imageSize=4k` 时，请求命中第二个标准上游
  - 双 key 且第二组是 `aiapidev` 时，请求仍走 `aiapidev` 特殊链路

**Step 2: Run test to verify it fails**

Run: `timeout 60s ~/.cargo/bin/cargo test --test http_forwarding_test -- --nocapture`
Expected: FAIL，因为当前在读取 body 前就已经固定了单个上游

**Step 3: Write minimal implementation**

- 先读取请求 body bytes
- 解析 JSON
- 调用新的上游解析函数选择最终上游
- 将已解析 body 继续传给现有转发流程

**Step 4: Run test to verify it passes**

Run: `timeout 60s ~/.cargo/bin/cargo test --test http_forwarding_test -- --nocapture`
Expected: PASS

### Task 3: 更新文档与回归验证

**Files:**
- Modify: `README.md`

**Step 1: Update docs**

- 把“当前 Rust 版不支持双上游”改为新的行为说明
- 明确仅支持请求头覆盖，不支持环境变量双 key
- 写明 `4k/4K` 才走第二组

**Step 2: Run focused verification**

Run: `timeout 60s ~/.cargo/bin/cargo test --test upstream_auth_test --test http_forwarding_test -- --nocapture`
Expected: PASS

**Step 3: Run broader verification**

Run: `timeout 60s ~/.cargo/bin/cargo test --tests -- --nocapture`
Expected: PASS
