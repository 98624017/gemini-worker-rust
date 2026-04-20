use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header::CONTENT_TYPE};
use axum::routing::get;
use image::ColorType;
use image::ImageEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Clone)]
struct ImageState {
    png: Vec<u8>,
    delay_ms: u64,
    request_count: Arc<AtomicUsize>,
    content_type: &'static str,
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

    tokio::time::sleep(Duration::from_millis(60)).await;
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

#[tokio::test]
async fn response_cache_reuses_inflight_fetch_concurrently() {
    let (address, request_count) = spawn_image_server(150).await;
    let mut config = rust_sync_proxy::test_config();
    config.inline_data_url_memory_cache_max_bytes = 1024;
    config.inline_data_url_background_fetch_wait_timeout = Duration::from_millis(100);
    config.inline_data_url_background_fetch_total_timeout = Duration::from_millis(500);
    let service = rust_sync_proxy::cache::InlineDataUrlFetchService::from_response_config(
        &config,
        reqwest::Client::new(),
        rust_sync_proxy::image_io::DEFAULT_MAX_IMAGE_BYTES,
        true,
    )
    .unwrap();

    let url = format!("http://{address}/image.png");
    let first_service = service.clone();
    let first_url = url.clone();
    let second_url = url.clone();
    let (first, second) = tokio::join!(
        async move { first_service.fetch(&first_url).await.unwrap() },
        async move { service.fetch(&second_url).await.unwrap() }
    );

    assert_eq!(first.mime_type, "image/png");
    assert_eq!(second.mime_type, "image/png");
    assert!(!first.from_cache);
    assert!(!second.from_cache);
    assert_eq!(request_count.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn response_cache_cancellation_does_not_finish_or_populate_cache() {
    let (address, request_count) = spawn_image_server(150).await;
    let mut config = rust_sync_proxy::test_config();
    config.inline_data_url_memory_cache_max_bytes = 1024;
    config.inline_data_url_background_fetch_wait_timeout = Duration::from_millis(100);
    config.inline_data_url_background_fetch_total_timeout = Duration::from_millis(500);
    let service = rust_sync_proxy::cache::InlineDataUrlFetchService::from_response_config(
        &config,
        reqwest::Client::new(),
        rust_sync_proxy::image_io::DEFAULT_MAX_IMAGE_BYTES,
        true,
    )
    .unwrap();

    let url = format!("http://{address}/image.png");
    let aborted_service = service.clone();
    let aborted_url = url.clone();
    let task = tokio::spawn(async move {
        let _ = aborted_service.fetch(&aborted_url).await;
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    task.abort();

    tokio::time::sleep(Duration::from_millis(220)).await;
    let second = service.fetch(&url).await.unwrap();

    assert_eq!(second.mime_type, "image/png");
    assert!(!second.from_cache);
    assert_eq!(request_count.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn request_materialize_reuses_fetch_service_cache_between_calls() {
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

    let runtime = rust_sync_proxy::test_blob_runtime(8 * 1024 * 1024);
    let request = json!({
        "contents": [{
            "parts": [{
                "inlineData": {
                    "data": format!("http://{address}/image.png")
                }
            }]
        }]
    });

    let first = rust_sync_proxy::request_materialize::materialize_request_images_with_services(
        request.clone(),
        &runtime,
        &rust_sync_proxy::request_materialize::RequestMaterializeServices {
            image_client: reqwest::Client::new(),
            max_image_bytes: rust_sync_proxy::image_io::DEFAULT_MAX_IMAGE_BYTES,
            allow_private_networks: true,
            enable_webp_optimization: false,
            fetch_service: Some(service.clone()),
            cache_observer: None,
        },
    )
    .await
    .unwrap();
    let second = rust_sync_proxy::request_materialize::materialize_request_images_with_services(
        request,
        &runtime,
        &rust_sync_proxy::request_materialize::RequestMaterializeServices {
            image_client: reqwest::Client::new(),
            max_image_bytes: rust_sync_proxy::image_io::DEFAULT_MAX_IMAGE_BYTES,
            allow_private_networks: true,
            enable_webp_optimization: false,
            fetch_service: Some(service),
            cache_observer: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(first.replacements.len(), 1);
    assert_eq!(second.replacements.len(), 1);
    assert_eq!(request_count.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn request_materialize_keeps_large_cached_bytes_in_memory_without_spill() {
    let large_image = vec![5_u8; 4096];
    let (address, _request_count) = spawn_custom_image_server(large_image, "image/jpeg", 0).await;
    let mut config = rust_sync_proxy::test_config();
    config.inline_data_url_memory_cache_max_bytes = 16 * 1024;
    config.inline_data_url_background_fetch_wait_timeout = Duration::from_millis(100);
    config.inline_data_url_background_fetch_total_timeout = Duration::from_millis(500);
    let service = rust_sync_proxy::cache::InlineDataUrlFetchService::from_config(
        &config,
        reqwest::Client::new(),
        rust_sync_proxy::image_io::DEFAULT_MAX_IMAGE_BYTES,
        true,
    )
    .unwrap();

    let runtime = rust_sync_proxy::BlobRuntime::new(rust_sync_proxy::BlobRuntimeConfig {
        inline_max_bytes: 1024,
        request_hot_budget_bytes: 1024,
        global_hot_budget_bytes: 8 * 1024,
        spill_dir: std::env::temp_dir()
            .join(format!("rust-sync-proxy-request-cache-{}", now_unix_ms())),
    });
    let request = json!({
        "contents": [{
            "parts": [{
                "inlineData": {
                    "data": format!("http://{address}/image.png")
                }
            }]
        }]
    });

    rust_sync_proxy::request_materialize::materialize_request_images_with_services(
        request.clone(),
        &runtime,
        &rust_sync_proxy::request_materialize::RequestMaterializeServices {
            image_client: reqwest::Client::new(),
            max_image_bytes: rust_sync_proxy::image_io::DEFAULT_MAX_IMAGE_BYTES,
            allow_private_networks: true,
            enable_webp_optimization: false,
            fetch_service: Some(service.clone()),
            cache_observer: None,
        },
    )
    .await
    .unwrap();

    let cache_hit = Arc::new(AtomicUsize::new(0));
    let second = rust_sync_proxy::request_materialize::materialize_request_images_with_services(
        request,
        &runtime,
        &rust_sync_proxy::request_materialize::RequestMaterializeServices {
            image_client: reqwest::Client::new(),
            max_image_bytes: rust_sync_proxy::image_io::DEFAULT_MAX_IMAGE_BYTES,
            allow_private_networks: true,
            enable_webp_optimization: false,
            fetch_service: Some(service),
            cache_observer: Some({
                let cache_hit = Arc::clone(&cache_hit);
                Arc::new(move |_url, from_cache| {
                    if from_cache {
                        cache_hit.fetch_add(1, Ordering::Relaxed);
                    }
                })
            }),
        },
    )
    .await
    .unwrap();

    assert_eq!(cache_hit.load(Ordering::Relaxed), 1);
    assert!(runtime.is_inline(&second.replacements[0].blob).await);
    assert_eq!(runtime.stats_snapshot().spill_count, 1);
}

#[tokio::test]
async fn request_cache_retries_once_after_connection_drops_before_headers() {
    let (address, request_count) = spawn_flaky_connection_server().await;
    let mut config = rust_sync_proxy::test_config();
    config.inline_data_url_memory_cache_max_bytes = 1024;
    config.inline_data_url_background_fetch_wait_timeout = Duration::from_secs(2);
    config.inline_data_url_background_fetch_total_timeout = Duration::from_secs(2);
    let service = rust_sync_proxy::cache::InlineDataUrlFetchService::from_config(
        &config,
        reqwest::Client::new(),
        rust_sync_proxy::image_io::DEFAULT_MAX_IMAGE_BYTES,
        true,
    )
    .unwrap();

    let url = format!("http://{address}/image.png");
    let fetched = service.fetch(&url).await.unwrap();
    assert_eq!(fetched.mime_type, "image/png");
    assert_eq!(
        fetched.bytes,
        bytes::Bytes::from_static(&[137, 80, 78, 71, 13, 10, 26, 10])
    );
    assert_eq!(request_count.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn request_cache_keeps_large_png_when_request_webp_optimization_is_disabled() {
    let png = noisy_png_bytes(2048, 1536);
    assert!(png.len() > 10 * 1024 * 1024, "png too small: {}", png.len());

    let (address, request_count) = spawn_custom_image_server(png, "image/png", 0).await;
    let mut config = rust_sync_proxy::test_config();
    config.inline_data_url_memory_cache_max_bytes = 64 * 1024 * 1024;
    config.inline_data_url_background_fetch_wait_timeout = Duration::from_secs(2);
    config.inline_data_url_background_fetch_total_timeout = Duration::from_secs(5);
    let service = rust_sync_proxy::cache::InlineDataUrlFetchService::from_config(
        &config,
        reqwest::Client::new(),
        rust_sync_proxy::image_io::REQUEST_MAX_IMAGE_BYTES,
        true,
    )
    .unwrap();

    let url = format!("http://{address}/image.png");
    let first = service.fetch(&url).await.unwrap();
    assert_eq!(first.mime_type, "image/png");
    assert!(!first.from_cache);

    let second = service.fetch(&url).await.unwrap();
    assert_eq!(second.mime_type, "image/png");
    assert!(second.from_cache);
    assert_eq!(second.bytes, first.bytes);
    assert_eq!(request_count.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn request_cache_stores_large_png_as_lossless_webp_when_enabled() {
    let png = noisy_png_bytes(2048, 1536);
    assert!(png.len() > 10 * 1024 * 1024, "png too small: {}", png.len());

    let (address, request_count) = spawn_custom_image_server(png, "image/png", 0).await;
    let mut config = rust_sync_proxy::test_config();
    config.enable_request_image_webp_optimization = true;
    config.inline_data_url_memory_cache_max_bytes = 64 * 1024 * 1024;
    config.inline_data_url_background_fetch_wait_timeout = Duration::from_secs(2);
    config.inline_data_url_background_fetch_total_timeout = Duration::from_secs(5);
    let service = rust_sync_proxy::cache::InlineDataUrlFetchService::from_config(
        &config,
        reqwest::Client::new(),
        rust_sync_proxy::image_io::REQUEST_MAX_IMAGE_BYTES,
        true,
    )
    .unwrap();

    let url = format!("http://{address}/image.png");
    let first = service.fetch(&url).await.unwrap();
    assert_eq!(first.mime_type, "image/webp");
    assert!(!first.from_cache);
    assert!(first.bytes.starts_with(b"RIFF"), "unexpected header");

    let second = service.fetch(&url).await.unwrap();
    assert_eq!(second.mime_type, "image/webp");
    assert!(second.from_cache);
    assert_eq!(second.bytes, first.bytes);
    assert_eq!(request_count.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn request_cache_does_not_retry_non_retryable_status() {
    let (address, request_count) = spawn_status_server(StatusCode::NOT_FOUND).await;
    let mut config = rust_sync_proxy::test_config();
    config.inline_data_url_memory_cache_max_bytes = 1024;
    config.inline_data_url_background_fetch_wait_timeout = Duration::from_secs(2);
    config.inline_data_url_background_fetch_total_timeout = Duration::from_secs(2);
    let service = rust_sync_proxy::cache::InlineDataUrlFetchService::from_config(
        &config,
        reqwest::Client::new(),
        rust_sync_proxy::image_io::DEFAULT_MAX_IMAGE_BYTES,
        true,
    )
    .unwrap();

    let url = format!("http://{address}/image.png");
    let err = service.fetch(&url).await.unwrap_err();
    assert!(
        err.to_string()
            .contains("image fetch failed with status 404"),
        "unexpected error: {err:#}"
    );
    assert_eq!(request_count.load(Ordering::Relaxed), 1);
}

async fn spawn_image_server(delay_ms: u64) -> (std::net::SocketAddr, Arc<AtomicUsize>) {
    spawn_custom_image_server(vec![137, 80, 78, 71, 13, 10, 26, 10], "image/png", delay_ms).await
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

async fn spawn_custom_image_server(
    png: Vec<u8>,
    content_type: &'static str,
    delay_ms: u64,
) -> (std::net::SocketAddr, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let request_count = Arc::new(AtomicUsize::new(0));
    let state = ImageState {
        png,
        delay_ms,
        request_count: Arc::clone(&request_count),
        content_type,
    };

    tokio::spawn(async move {
        let app = Router::new()
            .route("/image.png", get(serve_png))
            .with_state(state);
        axum::serve(listener, app).await.unwrap();
    });

    (address, request_count)
}

async fn spawn_status_server(status: StatusCode) -> (std::net::SocketAddr, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let request_count = Arc::new(AtomicUsize::new(0));
    let request_count_for_task = Arc::clone(&request_count);

    tokio::spawn(async move {
        let app = Router::new().route(
            "/image.png",
            get(move || async move {
                request_count_for_task.fetch_add(1, Ordering::Relaxed);
                (status, HeaderMap::new(), Vec::<u8>::new())
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });

    (address, request_count)
}

async fn spawn_flaky_connection_server() -> (std::net::SocketAddr, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let request_count = Arc::new(AtomicUsize::new(0));
    let request_count_for_task = Arc::clone(&request_count);

    tokio::spawn(async move {
        let (mut first_stream, _) = listener.accept().await.unwrap();
        request_count_for_task.fetch_add(1, Ordering::Relaxed);
        let mut discard = [0_u8; 1024];
        let _ = first_stream.read(&mut discard).await;
        drop(first_stream);

        let (mut second_stream, _) = listener.accept().await.unwrap();
        request_count_for_task.fetch_add(1, Ordering::Relaxed);
        let _ = second_stream.read(&mut discard).await;
        second_stream
            .write_all(
                b"HTTP/1.1 200 OK\r\ncontent-type: image/png\r\ncontent-length: 8\r\nconnection: close\r\n\r\n\x89PNG\r\n\x1a\n",
            )
            .await
            .unwrap();
        second_stream.shutdown().await.unwrap();
    });

    (address, request_count)
}

async fn serve_png(State(state): State<ImageState>) -> (StatusCode, HeaderMap, Vec<u8>) {
    state.request_count.fetch_add(1, Ordering::Relaxed);
    if state.delay_ms > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(state.delay_ms)).await;
    }
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(state.content_type));
    (StatusCode::OK, headers, state.png)
}

fn noisy_png_bytes(width: u32, height: u32) -> Vec<u8> {
    let mut rgba = vec![0_u8; (width as usize) * (height as usize) * 4];
    for (index, byte) in rgba.iter_mut().enumerate() {
        *byte = ((index * 31 + 17) % 251) as u8;
    }

    let mut encoded = Vec::new();
    PngEncoder::new_with_quality(&mut encoded, CompressionType::Fast, FilterType::NoFilter)
        .write_image(&rgba, width, height, ColorType::Rgba8.into())
        .unwrap();
    encoded
}

#[allow(dead_code)]
fn unique_temp_dir() -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("rust-sync-proxy-cache-{nanos}"))
}
