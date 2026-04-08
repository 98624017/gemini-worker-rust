use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header::CONTENT_TYPE};
use axum::routing::get;
use tokio::net::TcpListener;

#[derive(Clone)]
struct ImageState {
    png: Vec<u8>,
    delay_ms: u64,
    request_count: Arc<AtomicUsize>,
}

#[tokio::test]
async fn request_cache_reuses_fetched_image_across_calls() {
    let (address, request_count) = spawn_image_server(0).await;
    let mut config = rust_sync_proxy::test_config();
    config.inline_data_url_memory_cache_max_bytes = 1024;
    config.inline_data_url_background_fetch_wait_timeout = Duration::from_millis(100);
    config.inline_data_url_background_fetch_total_timeout = Duration::from_millis(500);
    let service = rust_sync_proxy::cache::InlineDataUrlFetchService::from_config(
        &config,
        reqwest::Client::new(),
        rust_sync_proxy::image_io::DEFAULT_MAX_IMAGE_BYTES,
        true,
    )
    .unwrap();

    let url = format!("http://{address}/image.png");
    let first = service.fetch(&url).await.unwrap();
    assert!(!first.from_cache);
    let second = service.fetch(&url).await.unwrap();
    assert!(second.from_cache);
    assert_eq!(request_count.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn request_cache_background_bridge_reuses_inflight_download() {
    let (address, request_count) = spawn_image_server(30).await;
    let mut config = rust_sync_proxy::test_config();
    config.inline_data_url_memory_cache_max_bytes = 1024;
    config.inline_data_url_background_fetch_wait_timeout = Duration::from_millis(20);
    config.inline_data_url_background_fetch_total_timeout = Duration::from_millis(500);
    let service = rust_sync_proxy::cache::InlineDataUrlFetchService::from_config(
        &config,
        reqwest::Client::new(),
        rust_sync_proxy::image_io::DEFAULT_MAX_IMAGE_BYTES,
        true,
    )
    .unwrap();

    let url = format!("http://{address}/image.png");
    let first_err = service.fetch(&url).await.unwrap_err();
    assert!(
        first_err
            .downcast_ref::<rust_sync_proxy::cache::BackgroundFetchWaitTimeoutError>()
            .is_some()
    );

    let second = service.fetch(&url).await.unwrap();
    assert_eq!(second.mime_type, "image/png");
    assert_eq!(
        second.bytes,
        bytes::Bytes::from_static(&[137, 80, 78, 71, 13, 10, 26, 10])
    );
    assert_eq!(request_count.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn request_cache_still_populates_cache_when_background_bridge_disabled() {
    let (address, request_count) = spawn_image_server(0).await;
    let mut config = rust_sync_proxy::test_config();
    config.inline_data_url_memory_cache_max_bytes = 1024;
    config.inline_data_url_background_fetch_max_inflight = 0;
    config.inline_data_url_background_fetch_wait_timeout = Duration::ZERO;
    config.inline_data_url_background_fetch_total_timeout = Duration::from_millis(500);
    let service = rust_sync_proxy::cache::InlineDataUrlFetchService::from_config(
        &config,
        reqwest::Client::new(),
        rust_sync_proxy::image_io::DEFAULT_MAX_IMAGE_BYTES,
        true,
    )
    .unwrap();

    let url = format!("http://{address}/image.png");
    let first = service.fetch(&url).await.unwrap();
    assert!(!first.from_cache);
    let second = service.fetch(&url).await.unwrap();
    assert!(second.from_cache);
    assert_eq!(request_count.load(Ordering::Relaxed), 1);
}

async fn spawn_image_server(delay_ms: u64) -> (std::net::SocketAddr, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let request_count = Arc::new(AtomicUsize::new(0));
    let state = ImageState {
        png: vec![137, 80, 78, 71, 13, 10, 26, 10],
        delay_ms,
        request_count: Arc::clone(&request_count),
    };

    tokio::spawn(async move {
        let app = Router::new()
            .route("/image.png", get(serve_png))
            .with_state(state);
        axum::serve(listener, app).await.unwrap();
    });

    (address, request_count)
}

async fn serve_png(State(state): State<ImageState>) -> (StatusCode, HeaderMap, Vec<u8>) {
    state.request_count.fetch_add(1, Ordering::Relaxed);
    if state.delay_ms > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(state.delay_ms)).await;
    }
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("image/png"));
    (StatusCode::OK, headers, state.png)
}

#[allow(dead_code)]
fn unique_temp_dir() -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("rust-sync-proxy-cache-{nanos}"))
}
