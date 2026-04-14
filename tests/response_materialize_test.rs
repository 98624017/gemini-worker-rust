use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::routing::{post, put};
use axum::{Json, http::HeaderMap};
use base64::Engine;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

#[derive(Clone, Default)]
struct UploadCapture {
    body: Arc<Mutex<Vec<u8>>>,
}

#[tokio::test]
async fn output_url_response_rewrites_uploaded_url_with_external_proxy_prefix() {
    let capture = UploadCapture::default();
    let upload_server = Router::new()
        .route("/uguu", post(mock_legacy_upload))
        .with_state(capture);
    let upload_addr = spawn_server(upload_server).await;

    let mut config = rust_sync_proxy::test_config();
    config.image_host_mode = "legacy".to_string();
    config.external_image_proxy_prefix = "https://external-proxy.example/fetch?url=".to_string();
    config.legacy_uguu_upload_url = format!("http://{upload_addr}/uguu");
    let uploader = rust_sync_proxy::upload::Uploader::new(reqwest::Client::new(), config.clone());

    let runtime = rust_sync_proxy::test_blob_runtime(8 * 1024 * 1024);
    let mut body = json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "inlineData": {
                        "mimeType": "image/png",
                        "data": "AQID"
                    }
                }]
            }
        }]
    });

    rust_sync_proxy::response_materialize::finalize_output_urls(
        &mut body, &runtime, &uploader, &config,
    )
    .await
    .unwrap();

    assert_eq!(
        body["candidates"][0]["content"]["parts"][0]["inlineData"]["data"],
        "https://external-proxy.example/fetch?url=https%3A%2F%2Fimg.example.com%2Fa.png"
    );
}

#[tokio::test]
async fn output_url_response_respects_proxy_standard_output_urls_flag() {
    let capture = UploadCapture::default();
    let upload_server = Router::new()
        .route("/uguu", post(mock_legacy_upload))
        .with_state(capture);
    let upload_addr = spawn_server(upload_server).await;

    let mut config = rust_sync_proxy::test_config();
    config.image_host_mode = "legacy".to_string();
    config.proxy_standard_output_urls = false;
    config.external_image_proxy_prefix = "https://external-proxy.example/fetch?url=".to_string();
    config.legacy_uguu_upload_url = format!("http://{upload_addr}/uguu");
    let uploader = rust_sync_proxy::upload::Uploader::new(reqwest::Client::new(), config.clone());

    let runtime = rust_sync_proxy::test_blob_runtime(8 * 1024 * 1024);
    let mut body = json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "inlineData": {
                        "mimeType": "image/png",
                        "data": "AQID"
                    }
                }]
            }
        }]
    });

    rust_sync_proxy::response_materialize::finalize_output_urls(
        &mut body, &runtime, &uploader, &config,
    )
    .await
    .unwrap();

    assert_eq!(
        body["candidates"][0]["content"]["parts"][0]["inlineData"]["data"],
        "https://img.example.com/a.png"
    );
}

#[tokio::test]
async fn output_url_response_keeps_custom_r2_public_url_without_proxy_prefix() {
    let capture = UploadCapture::default();
    let r2_server = Router::new()
        .route("/{*path}", put(mock_r2_put))
        .with_state(capture);
    let r2_addr = spawn_server(r2_server).await;

    let mut config = rust_sync_proxy::test_config();
    config.image_host_mode = "r2".to_string();
    config.external_image_proxy_prefix = "https://external-proxy.example/fetch?url=".to_string();
    config.r2_endpoint = format!("http://{r2_addr}");
    config.r2_bucket = "images".to_string();
    config.r2_access_key_id = "key".to_string();
    config.r2_secret_access_key = "secret".to_string();
    config.r2_public_base_url = "https://img.example.com".to_string();
    let uploader = rust_sync_proxy::upload::Uploader::new(reqwest::Client::new(), config.clone());

    let runtime = rust_sync_proxy::test_blob_runtime(8 * 1024 * 1024);
    let mut body = json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "inlineData": {
                        "mimeType": "image/png",
                        "data": "AQID"
                    }
                }]
            }
        }]
    });

    rust_sync_proxy::response_materialize::finalize_output_urls(
        &mut body, &runtime, &uploader, &config,
    )
    .await
    .unwrap();

    let final_url = body["candidates"][0]["content"]["parts"][0]["inlineData"]["data"]
        .as_str()
        .unwrap();
    assert!(final_url.starts_with("https://img.example.com/images/"));
    assert!(!final_url.starts_with("https://external-proxy.example/"));
}

#[tokio::test]
async fn output_url_response_streams_large_base64_without_runtime_spill() {
    let capture = UploadCapture::default();
    let upload_server = Router::new()
        .route("/uguu", post(mock_legacy_upload))
        .with_state(capture.clone());
    let upload_addr = spawn_server(upload_server).await;

    let mut config = rust_sync_proxy::test_config();
    config.image_host_mode = "legacy".to_string();
    config.legacy_uguu_upload_url = format!("http://{upload_addr}/uguu");
    let uploader = rust_sync_proxy::upload::Uploader::new(reqwest::Client::new(), config.clone());

    let runtime = rust_sync_proxy::BlobRuntime::new(rust_sync_proxy::BlobRuntimeConfig {
        inline_max_bytes: 1024,
        request_hot_budget_bytes: 1024,
        global_hot_budget_bytes: 8 * 1024,
        spill_dir: std::env::temp_dir().join(format!("response-materialize-{}", next_spill_id())),
    });
    let decoded = vec![7_u8; 4096];
    let encoded = base64::engine::general_purpose::STANDARD.encode(&decoded);
    let mut body = json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "inlineData": {
                        "mimeType": "image/png",
                        "data": encoded
                    }
                }]
            }
        }]
    });

    rust_sync_proxy::response_materialize::finalize_output_urls(
        &mut body, &runtime, &uploader, &config,
    )
    .await
    .unwrap();

    assert_eq!(runtime.stats_snapshot().spill_count, 0);
    let uploaded = capture.body.lock().await.clone();
    assert!(uploaded.windows(4).any(|window| window == [7, 7, 7, 7]));
}

async fn mock_legacy_upload(
    State(capture): State<UploadCapture>,
    _headers: HeaderMap,
    body: Bytes,
) -> Json<serde_json::Value> {
    *capture.body.lock().await = body.to_vec();
    Json(json!({
        "success": true,
        "files": [{
            "url": "https://img.example.com/a.png"
        }]
    }))
}

async fn mock_r2_put(
    State(_capture): State<UploadCapture>,
    _headers: HeaderMap,
    _body: Bytes,
) -> Json<serde_json::Value> {
    Json(json!({}))
}

async fn spawn_server(app: Router) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    address
}

fn next_spill_id() -> u64 {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}
