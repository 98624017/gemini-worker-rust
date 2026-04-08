use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header::CONTENT_TYPE};
use axum::routing::get;
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

async fn spawn_png_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let state = ImageState {
        png: vec![137, 80, 78, 71, 13, 10, 26, 10],
        delay: Duration::ZERO,
        in_flight: Arc::new(AtomicUsize::new(0)),
        peak_in_flight: Arc::new(AtomicUsize::new(0)),
    };

    let app = Router::new()
        .route("/image.png", get(serve_png))
        .with_state(state);

    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (format!("http://{address}/image.png"), server)
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
