use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rust_sync_proxy::blob_runtime::{BlobRuntime, BlobRuntimeConfig};

fn unique_spill_dir(name: &str) -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);

    let suffix = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    std::env::temp_dir().join(format!("rust-sync-proxy-{name}-{unix_ms}-{suffix}"))
}

fn test_blob_runtime(inline_max_bytes: u64) -> BlobRuntime {
    BlobRuntime::new(BlobRuntimeConfig {
        inline_max_bytes,
        request_hot_budget_bytes: 24 * 1024 * 1024,
        global_hot_budget_bytes: 384 * 1024 * 1024,
        spill_dir: unique_spill_dir("blob-runtime"),
    })
}

#[tokio::test]
async fn blob_runtime_keeps_small_blob_inline() {
    let runtime = BlobRuntime::new(BlobRuntimeConfig {
        inline_max_bytes: 8 * 1024 * 1024,
        request_hot_budget_bytes: 24 * 1024 * 1024,
        global_hot_budget_bytes: 384 * 1024 * 1024,
        spill_dir: unique_spill_dir("inline"),
    });

    let handle = runtime
        .store_bytes(b"abc".to_vec(), "image/png".into())
        .await
        .unwrap();

    assert!(runtime.is_inline(&handle).await);
}

#[tokio::test]
async fn blob_runtime_spills_large_blob_to_disk() {
    let runtime = test_blob_runtime(1024);
    let handle = runtime
        .store_bytes(vec![7; 4096], "image/png".into())
        .await
        .unwrap();

    assert!(runtime.is_spilled(&handle).await);
}
