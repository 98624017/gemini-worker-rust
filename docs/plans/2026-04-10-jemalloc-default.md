# jemalloc Default Allocator Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make `rust-sync-proxy` use `jemalloc` by default on Linux GNU targets, with a documented production default of `MALLOC_CONF=background_thread:true,dirty_decay_ms:500,muzzy_decay_ms:500`.

**Architecture:** Add a dedicated allocator module that installs `tikv-jemallocator` as the global allocator on Linux GNU and keeps the system allocator elsewhere. Keep allocator runtime tuning outside business logic by documenting and setting `MALLOC_CONF` in Docker and README rather than parsing it in Rust config.

**Tech Stack:** Rust 2024, `tikv-jemallocator`, Cargo, Docker, integration tests

---

### Task 1: 建立 allocator 边界和失败测试

**Files:**
- Create: `tests/allocator_test.rs`
- Modify: `src/lib.rs`
- Create: `src/allocator.rs`

**Step 1: 写失败测试**

```rust
#[test]
fn jemalloc_default_decay_matches_approved_value() {
    assert_eq!(
        rust_sync_proxy::allocator::DEFAULT_JEMALLOC_MALLOC_CONF,
        "background_thread:true,dirty_decay_ms:500,muzzy_decay_ms:500"
    );
}

#[test]
fn compiled_allocator_matches_platform_policy() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    assert_eq!(rust_sync_proxy::allocator::compiled_allocator_name(), "jemalloc");

    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    assert_eq!(rust_sync_proxy::allocator::compiled_allocator_name(), "system");
}
```

**Step 2: 运行测试确认失败**

Run: `~/.cargo/bin/cargo test --test allocator_test -- --nocapture`

Expected: FAIL，因为 `allocator` 模块尚不存在。

**Step 3: 做最小实现**

- 新建 `src/allocator.rs`
- 定义全局 allocator
- 暴露 `DEFAULT_JEMALLOC_MALLOC_CONF`
- 暴露 `compiled_allocator_name()`

**Step 4: 跑测试确认通过**

Run: `~/.cargo/bin/cargo test --test allocator_test -- --nocapture`

Expected: PASS

### Task 2: 接入依赖与启动日志

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/main.rs`

**Step 1: 增加失败断点**

Run: `~/.cargo/bin/cargo build --release --locked`

Expected: 仍使用系统 allocator，且无 allocator 启动信息。

**Step 2: 做最小实现**

- 在 `Cargo.toml` 增加 `tikv-jemallocator`
- 在 `main.rs` 启动日志中打印编译期 allocator 名称

**Step 3: 重新构建验证**

Run: `~/.cargo/bin/cargo build --release --locked`

Expected: PASS

### Task 3: 同步 Docker 和文档默认值

**Files:**
- Modify: `Dockerfile`
- Modify: `README.md`

**Step 1: 写文档断言**

手工核对 README 与 Docker 当前尚未声明 `jemalloc` 默认策略。

**Step 2: 做最小实现**

- 在 `Dockerfile` 增加默认 `MALLOC_CONF`
- 在 `README.md` 增加 allocator 说明、默认值与覆盖方式

**Step 3: 构建镜像验证**

Run: `docker build -t rust-sync-proxy:jemalloc-test .`

Expected: PASS

### Task 4: 全量回归和接入证据

**Files:**
- Modify: `README.md`

**Step 1: 跑测试**

Run: `~/.cargo/bin/cargo test --tests -- --nocapture`

Expected: PASS

**Step 2: 跑发布构建**

Run: `~/.cargo/bin/cargo build --release --locked`

Expected: PASS

**Step 3: 做接入证据检查**

Run: `strings target/release/rust-sync-proxy | rg jemalloc`

Expected: 能看到 `jemalloc` 相关符号或字符串

**Step 4: 记录结论**

- 记录默认 allocator
- 记录默认 `MALLOC_CONF`
- 记录未完成项：真正的 RSS 压测仍需下一轮数据验证
