use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header::CONTENT_TYPE};
use axum::routing::get;
use image::ColorType;
use image::ImageEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::time::Duration;

#[derive(Clone)]
struct ImageState {
    png: Vec<u8>,
    delay: Duration,
    in_flight: Arc<AtomicUsize>,
    peak_in_flight: Arc<AtomicUsize>,
}

#[tokio::test]
async fn request_materialize_fetches_image_url_into_blob_handle() {
    let (image_url, _server) = spawn_png_server().await;
    let runtime = rust_sync_proxy::test_blob_runtime(8 * 1024 * 1024);
    let request = json!({
        "contents": [{"parts": [{"inlineData": {"data": image_url}}]}]
    });

    let materialized = rust_sync_proxy::request_materialize::materialize_request_images(
        request,
        &runtime,
        &reqwest::Client::new(),
    )
    .await
    .unwrap();

    assert_eq!(materialized.replacements.len(), 1);
    assert_eq!(materialized.replacements[0].mime_type, "image/png");
}

#[test]
fn request_materialize_uses_20mib_request_limit() {
    assert_eq!(
        rust_sync_proxy::image_io::REQUEST_MAX_IMAGE_BYTES,
        20 * 1024 * 1024
    );
}

#[tokio::test]
async fn request_materialize_fetches_unique_urls_concurrently() {
    let (base_url, peak_in_flight, _server) =
        spawn_delayed_png_server(Duration::from_millis(80)).await;
    let runtime = rust_sync_proxy::test_blob_runtime(8 * 1024 * 1024);
    let request = json!({
        "contents": [{
            "parts": [
                {"inlineData": {"data": format!("{base_url}/image-a.png")}},
                {"inlineData": {"data": format!("{base_url}/image-b.png")}}
            ]
        }]
    });

    let materialized = rust_sync_proxy::request_materialize::materialize_request_images(
        request,
        &runtime,
        &reqwest::Client::new(),
    )
    .await
    .unwrap();

    assert_eq!(materialized.replacements.len(), 2);
    assert!(
        peak_in_flight.load(Ordering::Relaxed) >= 2,
        "expected concurrent fetches, got peak={}",
        peak_in_flight.load(Ordering::Relaxed)
    );
}

#[tokio::test]
async fn request_materialize_rejects_images_over_request_limit() {
    let oversized_png = vec![0_u8; rust_sync_proxy::image_io::REQUEST_MAX_IMAGE_BYTES + 1];
    let (image_url, _server) = spawn_named_png_server("/oversized.png", oversized_png).await;
    let runtime = rust_sync_proxy::test_blob_runtime(8 * 1024 * 1024);
    let request = json!({
        "contents": [{"parts": [{"inlineData": {"data": image_url}}]}]
    });

    let err = rust_sync_proxy::request_materialize::materialize_request_images(
        request,
        &runtime,
        &reqwest::Client::new(),
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains(&format!(
        "image too large: {} > {}",
        rust_sync_proxy::image_io::REQUEST_MAX_IMAGE_BYTES + 1,
        rust_sync_proxy::image_io::REQUEST_MAX_IMAGE_BYTES
    )));
}

#[tokio::test]
async fn request_materialize_keeps_large_png_without_webp_optimization() {
    let (image_url, _server) =
        spawn_named_png_server("/large.png", noisy_png_bytes(2048, 1536)).await;
    let runtime = rust_sync_proxy::test_blob_runtime(8 * 1024 * 1024);
    let request = json!({
        "contents": [{"parts": [{"inlineData": {"data": image_url}}]}]
    });

    let materialized =
        rust_sync_proxy::request_materialize::materialize_request_images_with_services(
            request,
            &runtime,
            &rust_sync_proxy::request_materialize::RequestMaterializeServices {
                image_client: reqwest::Client::new(),
                max_image_bytes: rust_sync_proxy::image_io::REQUEST_MAX_IMAGE_BYTES,
                allow_private_networks: true,
                enable_webp_optimization: false,
                fetch_service: None,
                cache_observer: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(materialized.replacements.len(), 1);
    assert_eq!(materialized.replacements[0].mime_type, "image/png");
}

#[tokio::test]
async fn request_materialize_converts_large_png_when_webp_optimization_enabled() {
    let (image_url, _server) =
        spawn_named_png_server("/large.png", noisy_png_bytes(2048, 1536)).await;
    let runtime = rust_sync_proxy::test_blob_runtime(8 * 1024 * 1024);
    let request = json!({
        "contents": [{"parts": [{"inlineData": {"data": image_url}}]}]
    });

    let materialized =
        rust_sync_proxy::request_materialize::materialize_request_images_with_services(
            request,
            &runtime,
            &rust_sync_proxy::request_materialize::RequestMaterializeServices {
                image_client: reqwest::Client::new(),
                max_image_bytes: rust_sync_proxy::image_io::REQUEST_MAX_IMAGE_BYTES,
                allow_private_networks: true,
                enable_webp_optimization: true,
                fetch_service: None,
                cache_observer: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(materialized.replacements.len(), 1);
    assert_eq!(materialized.replacements[0].mime_type, "image/webp");
}

async fn spawn_png_server() -> (String, tokio::task::JoinHandle<()>) {
    spawn_named_png_server("/image.png", vec![137, 80, 78, 71, 13, 10, 26, 10]).await
}

async fn spawn_named_png_server(path: &str, png: Vec<u8>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let state = ImageState {
        png,
        delay: Duration::ZERO,
        in_flight: Arc::new(AtomicUsize::new(0)),
        peak_in_flight: Arc::new(AtomicUsize::new(0)),
    };

    let app = Router::new().route(path, get(serve_png)).with_state(state);

    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (format!("http://{address}{path}"), server)
}

async fn spawn_delayed_png_server(
    delay: Duration,
) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let in_flight = Arc::new(AtomicUsize::new(0));
    let peak_in_flight = Arc::new(AtomicUsize::new(0));
    let state = ImageState {
        png: vec![137, 80, 78, 71, 13, 10, 26, 10],
        delay,
        in_flight: Arc::clone(&in_flight),
        peak_in_flight: Arc::clone(&peak_in_flight),
    };

    let app = Router::new()
        .route("/image-a.png", get(serve_png))
        .route("/image-b.png", get(serve_png))
        .with_state(state);

    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (format!("http://{address}"), peak_in_flight, server)
}

async fn serve_png(State(state): State<ImageState>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let current = state.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
    state.peak_in_flight.fetch_max(current, Ordering::SeqCst);
    if !state.delay.is_zero() {
        tokio::time::sleep(state.delay).await;
    }
    state.in_flight.fetch_sub(1, Ordering::SeqCst);

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("image/png"));
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
    assert!(
        encoded.len() > 10 * 1024 * 1024,
        "png too small: {}",
        encoded.len()
    );
    encoded
}
