# Request WebP Toggle And Album Defaults Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将请求侧 URL 拉图后的大 PNG 无损转 WebP 优化改为默认关闭且仅在环境变量开启时生效，同时让 `/admin` 相册视图默认展示图片。

**Architecture:** 在配置层新增一个请求侧专用布尔开关，并把该开关一路下沉到请求侧图片抓取的缓存路径和直抓路径，避免复用响应侧 `ENABLE_IMAGE_COMPRESSION` 语义。`/admin` 页面继续沿用 `src/admin.rs` 的单文件 HTML/CSS/JS 实现，只调整相册图片渲染默认值与对应测试断言，不改后端 API。

**Tech Stack:** Rust, Axum, reqwest, serde_json, cargo test, GitHub Actions

---

### Task 1: 为请求侧 WebP 优化补失败测试并固定配置语义

**Files:**
- Modify: `tests/config_test.rs`
- Modify: `tests/request_cache_test.rs`
- Modify: `src/lib.rs`
- Modify: `src/config.rs`

- [ ] **Step 1: 写出失败测试，固定新环境变量默认关闭且可显式开启**

```rust
#[test]
fn request_image_webp_optimization_defaults_to_disabled() {
    let cfg = rust_sync_proxy::config::Config::from_env_map(&HashMap::new()).unwrap();
    assert!(!cfg.enable_request_image_webp_optimization);
}

#[test]
fn request_image_webp_optimization_can_be_enabled_from_env() {
    let env = HashMap::from([(
        "ENABLE_REQUEST_IMAGE_WEBP_OPTIMIZATION".to_string(),
        "true".to_string(),
    )]);
    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();
    assert!(cfg.enable_request_image_webp_optimization);
}
```

- [ ] **Step 2: 运行测试，确认当前失败**

Run: `timeout 60 cargo test request_image_webp_optimization --test config_test -- --exact`
Expected: FAIL，提示 `Config` 中缺少 `enable_request_image_webp_optimization`

- [ ] **Step 3: 增加请求侧缓存服务行为测试，固定默认不转 WebP、开启后才转**

```rust
#[tokio::test]
async fn request_cache_does_not_convert_large_png_to_webp_by_default() {
    // 准备一个 >10MiB 的 PNG 响应，断言 fetch 后 mime_type 仍为 image/png
}

#[tokio::test]
async fn request_cache_converts_large_png_to_webp_when_env_enabled() {
    // 同样的 PNG 响应，在显式开启后断言 fetch 后 mime_type 为 image/webp
}
```

- [ ] **Step 4: 运行测试，确认当前失败**

Run: `timeout 60 cargo test request_cache_ --test request_cache_test`
Expected: 新增的 2 个测试至少有 1 个失败，因为当前默认仍会转 WebP

### Task 2: 实现请求侧 WebP 开关并同步文档

**Files:**
- Modify: `src/config.rs`
- Modify: `src/lib.rs`
- Modify: `src/cache.rs`
- Modify: `src/request_materialize.rs`
- Modify: `README.md`

- [ ] **Step 1: 在配置中新增请求侧开关字段并读取环境变量**

```rust
pub struct Config {
    // ...
    pub enable_request_image_webp_optimization: bool,
}
```

```rust
enable_request_image_webp_optimization: parse_bool(
    env_map.get("ENABLE_REQUEST_IMAGE_WEBP_OPTIMIZATION"),
    false,
),
```

- [ ] **Step 2: 把开关下沉到请求侧缓存服务**

```rust
optimize_large_png_as_webp: config.enable_request_image_webp_optimization,
```

- [ ] **Step 3: 把开关下沉到无缓存直抓路径**

```rust
let fetched = if services.enable_webp_optimization {
    maybe_convert_large_png_to_lossless_webp(fetched).await?
} else {
    fetched
};
```

- [ ] **Step 4: 更新 README，请求侧文档改为默认关闭且需显式开启**

```md
- `ENABLE_REQUEST_IMAGE_WEBP_OPTIMIZATION`
  默认关闭；开启后，标准链路请求侧按 URL 拉图时，若抓到的 PNG 大于 `10MiB`，
  会先尝试无损转成 `image/webp` 再发往真实上游，并把这版字节写入请求侧缓存。
```

- [ ] **Step 5: 运行相关测试，确认全部通过**

Run: `timeout 60 cargo test config_test request_cache_test`
Expected: PASS

### Task 3: 让 admin 相册模式默认展示图片

**Files:**
- Modify: `tests/admin_test.rs`
- Modify: `src/admin.rs`

- [ ] **Step 1: 先写失败测试，固定相册模式默认显示图片**

```rust
#[tokio::test]
async fn admin_logs_page_defaults_album_images_to_visible() {
    let html = fetch_admin_logs_page_html().await;
    assert!(html.contains("var albumImagesVisible = true;"));
}
```

- [ ] **Step 2: 运行测试，确认当前失败**

Run: `timeout 60 cargo test admin_logs_page_defaults_album_images_to_visible --test admin_test -- --exact`
Expected: FAIL，因为当前默认值不是显示

- [ ] **Step 3: 修改 admin 前端脚本，让相册图片默认显示**

```javascript
var albumImagesVisible = true;
```

```javascript
function shouldShowAlbumImages() {
  return albumImagesVisible !== false;
}
```

- [ ] **Step 4: 运行 admin 相关测试，确认通过**

Run: `timeout 60 cargo test admin_logs_page_ --test admin_test`
Expected: PASS

### Task 4: 回归验证、提交、推送

**Files:**
- Modify: `docs/superpowers/plans/2026-04-18-request-webp-and-album-defaults.md`

- [ ] **Step 1: 运行聚焦回归测试**

Run: `timeout 60 cargo test config_test request_cache_test admin_test`
Expected: PASS

- [ ] **Step 2: 查看工作区变更**

Run: `git status --short`
Expected: 仅包含本任务相关文件，以及已有未跟踪的用户文件

- [ ] **Step 3: 提交改动**

```bash
git add src/config.rs src/lib.rs src/cache.rs src/request_materialize.rs src/admin.rs tests/config_test.rs tests/request_cache_test.rs tests/admin_test.rs README.md docs/superpowers/plans/2026-04-18-request-webp-and-album-defaults.md
git commit -m "feat: gate request webp optimization and show album images by default"
```

- [ ] **Step 4: 推送当前分支**

Run: `git push origin main`
Expected: 推送成功，并触发 `.github/workflows/docker.yml`

- [ ] **Step 5: 确认远程 Docker 构建触发**

Run: `gh run list --workflow docker.yml --limit 5`
Expected: 出现本次最新 push 对应的 workflow run
