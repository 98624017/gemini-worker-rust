# Request PNG WebP Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Raise request-side fetched image limit to 20MiB and transcode large PNG inputs to lossless WebP before forwarding upstream.

**Architecture:** Keep the change inside request-side fetch/materialize flow so standard-chain URL inputs are normalized before request JSON encoding. Reuse the existing cache storage model so memory and disk caches persist the transformed bytes plus updated `mimeType`.

**Tech Stack:** Rust, `image`, reqwest, tokio, axum tests

---

### Task 1: Add failing request/image tests

**Files:**
- Modify: `tests/request_materialize_test.rs`
- Modify: `tests/request_cache_test.rs`

**Step 1: Write the failing test**

Add tests for:
- `REQUEST_MAX_IMAGE_BYTES == 20 * 1024 * 1024`
- oversized request image rejection still uses the new 20MiB cap
- cached request-side fetches return `image/webp` after large PNG optimization

**Step 2: Run test to verify it fails**

Run: `cargo test request_materialize request_cache -- --nocapture`
Expected: FAIL because current code still uses `15MiB`, lacks WebP encoding, and cached MIME remains `image/png`.

### Task 2: Implement PNG-to-WebP optimization

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/image_io.rs`
- Modify: `src/cache.rs`

**Step 1: Write minimal implementation**

Add request-side constants and a helper that:
- only handles `image/png`
- only triggers above 10MiB
- encodes lossless WebP
- falls back to original PNG if encoding fails or does not shrink bytes

Then invoke the helper in request-side direct fetch path before publishing to caches.

**Step 2: Run targeted tests**

Run: `cargo test request_materialize request_cache -- --nocapture`
Expected: PASS

### Task 3: Document and verify

**Files:**
- Modify: `README.md`

**Step 1: Update docs**

Document the new 20MiB request input cap and the request-side large-PNG lossless WebP rewrite rule.

**Step 2: Final verification**

Run:
- `cargo test request_materialize -- --nocapture`
- `cargo test request_cache -- --nocapture`
- `cargo fmt --check`

Expected: PASS
