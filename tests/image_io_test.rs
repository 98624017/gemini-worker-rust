use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header::CONTENT_TYPE};
use axum::routing::get;
use bytes::Bytes as RawBytes;
use futures_util::stream;
use image::ColorType;
use image::ImageEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use tokio::net::TcpListener;
use tokio::time::Duration;

#[tokio::test]
async fn rejects_images_over_max_size() {
    let result =
        rust_sync_proxy::image_io::enforce_max_size(35 * 1024 * 1024 + 1, 35 * 1024 * 1024);
    assert!(result.is_err());
}

#[tokio::test]
async fn fetch_image_rejects_when_content_length_exceeds_limit() {
    let app = Router::new().route("/image.png", get(serve_small_png));
    let address = spawn_server(app).await;

    let err = rust_sync_proxy::image_io::fetch_image_as_inline_data_with_options(
        &reqwest::Client::new(),
        &format!("http://{address}/image.png"),
        3,
        true,
    )
    .await
    .unwrap_err();

    let err_text = err.to_string();
    assert!(err_text.contains("image too large:"), "{err_text}");
    assert!(err_text.contains("> 3"), "{err_text}");
}

#[tokio::test]
async fn fetch_image_rejects_stream_when_size_exceeds_limit_without_content_length() {
    let request_count = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/stream.png", get(serve_chunked_png))
        .with_state(request_count.clone());
    let address = spawn_server(app).await;

    let err = rust_sync_proxy::image_io::fetch_image_as_inline_data_with_options(
        &reqwest::Client::new(),
        &format!("http://{address}/stream.png"),
        5,
        true,
    )
    .await
    .unwrap_err();

    let err_text = err.to_string();
    assert!(err_text.contains("image too large:"), "{err_text}");
    assert!(err_text.contains("> 5"), "{err_text}");
    assert_eq!(request_count.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn png_to_webp_failure_falls_back_to_original_bytes() {
    let original = vec![7_u8; rust_sync_proxy::image_io::REQUEST_PNG_WEBP_THRESHOLD_BYTES + 1];
    let fetched = rust_sync_proxy::image_io::FetchedInlineData {
        mime_type: "image/png".to_string(),
        bytes: RawBytes::from(original.clone()),
    };

    let optimized = rust_sync_proxy::image_io::maybe_convert_large_png_to_lossless_webp(fetched)
        .await
        .unwrap();

    assert_eq!(optimized.mime_type, "image/png");
    assert_eq!(optimized.bytes, RawBytes::from(original));
}

#[tokio::test]
async fn png_to_webp_timeout_falls_back_to_original_bytes() {
    let original = noisy_png_bytes(2048, 1536);
    let fetched = rust_sync_proxy::image_io::FetchedInlineData {
        mime_type: "image/png".to_string(),
        bytes: RawBytes::from(original.clone()),
    };

    let optimized =
        rust_sync_proxy::image_io::maybe_convert_large_png_to_lossless_webp_with_timeout(
            fetched,
            Duration::from_nanos(1),
        )
        .await
        .unwrap();

    assert_eq!(optimized.mime_type, "image/png");
    assert_eq!(optimized.bytes, RawBytes::from(original));
}

async fn serve_small_png() -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("image/png"));
    (
        StatusCode::OK,
        headers,
        vec![137, 80, 78, 71, 13, 10, 26, 10],
    )
}

async fn serve_chunked_png(
    State(request_count): State<Arc<AtomicUsize>>,
) -> (StatusCode, HeaderMap, Body) {
    request_count.fetch_add(1, Ordering::Relaxed);
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("image/png"));
    let chunks = vec![
        Ok::<Bytes, Infallible>(Bytes::from_static(&[137, 80, 78, 71])),
        Ok::<Bytes, Infallible>(Bytes::from_static(&[13, 10, 26, 10])),
    ];
    (
        StatusCode::OK,
        headers,
        Body::from_stream(stream::iter(chunks)),
    )
}

async fn spawn_server(app: Router) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    address
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
        encoded.len() > rust_sync_proxy::image_io::REQUEST_PNG_WEBP_THRESHOLD_BYTES,
        "png too small: {}",
        encoded.len()
    );
    encoded
}
