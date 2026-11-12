use std::sync::Arc;
use std::time::Duration;

use axum::{Router, http::StatusCode, routing::get};
use rust_sync_proxy::request_materialize::materialize_request_images;
use rust_sync_proxy::{BlobRuntime, BlobRuntimeConfig};
use serde_json::json;
use tokio::sync::Notify;

fn runtime(directory: &std::path::Path, inline_max_bytes: u64) -> BlobRuntime {
    BlobRuntime::new(BlobRuntimeConfig {
        inline_max_bytes,
        request_hot_budget_bytes: 4,
        global_hot_budget_bytes: 4,
        spill_dir: directory.to_path_buf(),
    })
}

#[tokio::test]
async fn last_blob_handle_releases_budget_and_explicit_remove_is_idempotent() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = runtime(directory.path(), 4);
    let first = runtime
        .store_bytes(vec![1; 4], "image/png".into())
        .await
        .unwrap();
    let retained = first.clone();
    drop(first);
    let second = runtime
        .store_bytes(vec![2; 4], "image/png".into())
        .await
        .unwrap();
    assert!(runtime.is_spilled(&second).await);
    runtime.remove(&second).await.unwrap();
    drop(retained);

    let third = runtime
        .store_bytes(vec![3; 4], "image/png".into())
        .await
        .unwrap();
    assert!(
        runtime.is_inline(&third).await,
        "dropping the final handle must return its budget"
    );
    runtime.remove(&third).await.unwrap();
    runtime.remove(&third).await.unwrap();
    drop(third);
    let fourth = runtime
        .store_bytes(vec![4; 4], "image/png".into())
        .await
        .unwrap();
    assert!(
        runtime.is_inline(&fourth).await,
        "explicit removal and drop must not release twice"
    );
}

#[tokio::test]
async fn failed_materialization_cleans_completed_images() {
    check_materialization_cleanup(false).await;
}

#[tokio::test]
async fn cancelled_materialization_cleans_completed_images() {
    check_materialization_cleanup(true).await;
}

async fn check_materialization_cleanup(cancel: bool) {
    let directory = tempfile::tempdir().unwrap();
    let runtime = runtime(directory.path(), 0);
    let fail_gate = Arc::new(Notify::new());
    let handler_gate = fail_gate.clone();
    let app = Router::new()
        .route(
            "/good.png",
            get(|| async { ([("content-type", "image/png")], vec![7_u8; 4096]) }),
        )
        .route(
            "/bad.png",
            get(move || {
                let gate = handler_gate.clone();
                async move {
                    gate.notified().await;
                    StatusCode::NOT_FOUND
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let request_runtime = runtime.clone();
    let request = tokio::spawn(async move {
        materialize_request_images(
            json!({"contents": [{"parts": [
                {"inlineData": {"data": format!("{base}/good.png")}},
                {"inlineData": {"data": format!("{base}/bad.png")}}
            ]}]}),
            &request_runtime,
            &reqwest::Client::builder().no_proxy().build().unwrap(),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while runtime.stats_snapshot().spill_count == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    if cancel {
        request.abort();
        assert!(request.await.unwrap_err().is_cancelled());
    } else {
        fail_gate.notify_one();
        assert!(request.await.unwrap().is_err());
    }
    server.abort();
    let _ = server.await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while std::fs::read_dir(directory.path())
            .unwrap()
            .next()
            .is_some()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("failed or cancelled requests must remove completed spill files");
}
