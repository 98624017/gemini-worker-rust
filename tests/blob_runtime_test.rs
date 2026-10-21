use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use bytes::Bytes;
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

#[tokio::test]
async fn blob_runtime_records_spill_count_and_bytes() {
    let runtime = test_blob_runtime(1024);

    let first = runtime
        .store_bytes(vec![7; 4096], "image/png".into())
        .await
        .unwrap();
    let second = runtime
        .store_bytes(vec![8; 2048], "image/png".into())
        .await
        .unwrap();

    assert!(runtime.is_spilled(&first).await);
    assert!(runtime.is_spilled(&second).await);

    let stats = runtime.stats_snapshot();
    assert_eq!(stats.spill_count, 2);
    assert_eq!(stats.spill_bytes_total, 4096 + 2048);
}

#[tokio::test]
async fn blob_runtime_does_not_record_spill_when_spill_write_fails() {
    let guard = tempfile::NamedTempFile::new().unwrap();
    let bad_dir = guard.path().to_path_buf();
    let runtime = BlobRuntime::new(BlobRuntimeConfig {
        inline_max_bytes: 1024,
        request_hot_budget_bytes: 24 * 1024 * 1024,
        global_hot_budget_bytes: 384 * 1024 * 1024,
        spill_dir: bad_dir,
    });

    let err = runtime
        .store_bytes(vec![7; 4096], "image/png".into())
        .await
        .unwrap_err();

    let stats = runtime.stats_snapshot();
    assert_eq!(stats.spill_count, 0);
    assert_eq!(stats.spill_bytes_total, 0);
    assert!(
        err.to_string().contains("Not a directory")
            || err.to_string().contains("not a directory")
            || err.to_string().contains("os error")
    );
}

#[tokio::test]
async fn blob_runtime_can_store_shared_bytes_without_vec_copy() {
    let runtime = test_blob_runtime(8 * 1024 * 1024);
    let data = Bytes::from_static(b"shared-bytes");

    let handle = runtime
        .store_shared_bytes(data.clone(), "image/png".into())
        .await
        .unwrap();

    assert!(runtime.is_inline(&handle).await);
    assert_eq!(runtime.read_bytes(&handle).await.unwrap(), data);
}

#[tokio::test]
async fn blob_runtime_can_store_external_shared_bytes_without_spill() {
    let runtime = test_blob_runtime(1024);
    let data = Bytes::from(vec![9_u8; 4096]);

    let handle = runtime
        .store_external_shared_bytes(data.clone(), "image/png".into())
        .await
        .unwrap();

    assert!(runtime.is_inline(&handle).await);
    assert_eq!(runtime.read_bytes(&handle).await.unwrap(), data);
    assert_eq!(runtime.stats_snapshot().spill_count, 0);
}

#[tokio::test]
async fn interrupted_blob_stream_removes_partial_spill() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = BlobRuntime::new(BlobRuntimeConfig {
        inline_max_bytes: 8,
        request_hot_budget_bytes: 8,
        global_hot_budget_bytes: 8,
        spill_dir: directory.path().to_path_buf(),
    });
    let chunks = futures_util::stream::iter([
        Ok(Bytes::from_static(b"partial image payload")),
        Err(std::io::Error::from(std::io::ErrorKind::ConnectionReset)),
    ]);
    let error = runtime
        .store_stream(
            tokio_util::io::StreamReader::new(chunks),
            "application/json".into(),
        )
        .await
        .unwrap_err();

    assert!(error.downcast_ref::<std::io::Error>().is_some());
    assert!(
        std::fs::read_dir(directory.path())
            .unwrap()
            .next()
            .is_none()
    );
    assert_eq!(runtime.stats_snapshot().spill_count, 0);
}

#[tokio::test]
async fn cancelled_blob_stream_removes_partial_spill() {
    use tokio::io::AsyncWriteExt;

    let directory = tempfile::tempdir().unwrap();
    let runtime = BlobRuntime::new(BlobRuntimeConfig {
        inline_max_bytes: 8,
        request_hot_budget_bytes: 8,
        global_hot_budget_bytes: 8,
        spill_dir: directory.path().to_path_buf(),
    });
    let store_runtime = runtime.clone();
    let (mut writer, reader) = tokio::io::duplex(64);
    let storing = tokio::spawn(async move {
        store_runtime
            .store_stream(reader, "application/json".into())
            .await
    });
    writer.write_all(b"partial image payload").await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while std::fs::read_dir(directory.path())
            .unwrap()
            .next()
            .is_none()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    storing.abort();
    assert!(storing.await.unwrap_err().is_cancelled());
    assert!(
        std::fs::read_dir(directory.path())
            .unwrap()
            .next()
            .is_none()
    );
    assert_eq!(runtime.stats_snapshot().spill_count, 0);
}
