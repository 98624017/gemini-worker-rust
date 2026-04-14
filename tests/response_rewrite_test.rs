use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header::CONTENT_TYPE};
use axum::routing::get;
use tokio::net::TcpListener;

#[test]
fn keeps_only_largest_inline_image_per_candidate() {
    let input = json!({
        "candidates": [{
            "content": {"parts": [
                {"inlineData": {"mimeType": "image/png", "data": "aaaa"}},
                {"inlineData": {"mimeType": "image/png", "data": "aaaaaaaa"}}
            ]}
        }]
    });

    let output = rust_sync_proxy::response_rewrite::keep_largest_inline_image(input);
    let parts = output["candidates"][0]["content"]["parts"]
        .as_array()
        .unwrap();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0]["inlineData"]["data"], "aaaaaaaa");
}

#[derive(Clone)]
struct MarkdownImageState {
    png: Vec<u8>,
    request_count: Arc<AtomicUsize>,
}

#[tokio::test]
async fn markdown_base64_normalization_uses_fetch_service_cache() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let request_count = Arc::new(AtomicUsize::new(0));
    let state = MarkdownImageState {
        png: vec![137, 80, 78, 71, 13, 10, 26, 10],
        request_count: Arc::clone(&request_count),
    };
    let app = Router::new()
        .route("/image.png", get(serve_markdown_png))
        .with_state(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let mut config = rust_sync_proxy::test_config();
    config.inline_data_url_memory_cache_max_bytes = 1024;
    config.inline_data_url_background_fetch_wait_timeout = Duration::from_millis(100);
    config.inline_data_url_background_fetch_total_timeout = Duration::from_millis(500);
    let fetch_service = rust_sync_proxy::cache::InlineDataUrlFetchService::from_response_config(
        &config,
        reqwest::Client::new(),
        rust_sync_proxy::image_io::DEFAULT_MAX_IMAGE_BYTES,
        true,
    )
    .unwrap();

    let body = json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "text": format!("![img](http://{address}/image.png)")
                }]
            }
        }]
    });

    let first = rust_sync_proxy::response_rewrite::normalize_special_markdown_image_response(
        body.clone(),
        rust_sync_proxy::response_rewrite::OutputMode::Base64,
        &reqwest::Client::new(),
        Some(&fetch_service),
        &config,
    )
    .await
    .unwrap();
    let second = rust_sync_proxy::response_rewrite::normalize_special_markdown_image_response(
        body,
        rust_sync_proxy::response_rewrite::OutputMode::Base64,
        &reqwest::Client::new(),
        Some(&fetch_service),
        &config,
    )
    .await
    .unwrap();

    assert_eq!(
        first["candidates"][0]["content"]["parts"][0]["inlineData"]["data"],
        "iVBORw0KGgo="
    );
    assert_eq!(
        second["candidates"][0]["content"]["parts"][0]["inlineData"]["data"],
        "iVBORw0KGgo="
    );
    assert_eq!(request_count.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn markdown_url_normalization_uses_external_proxy_prefix() {
    let mut config = rust_sync_proxy::test_config();
    config.external_image_proxy_prefix = "https://proxy.example.com/fetch?url=".to_string();
    config.proxy_special_upstream_urls = true;

    let body = json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "text": "![img](https://example.com/path/demo.png)"
                }]
            }
        }]
    });

    let output = rust_sync_proxy::response_rewrite::normalize_special_markdown_image_response(
        body,
        rust_sync_proxy::response_rewrite::OutputMode::Url,
        &reqwest::Client::new(),
        None,
        &config,
    )
    .await
    .unwrap();

    assert_eq!(
        output["candidates"][0]["content"]["parts"][0]["inlineData"]["data"],
        "https://proxy.example.com/fetch?url=https%3A%2F%2Fexample.com%2Fpath%2Fdemo.png"
    );
}

#[tokio::test]
async fn aiapidev_task_response_url_mode_reuses_external_proxy_prefix() {
    let mut config = rust_sync_proxy::test_config();
    config.external_image_proxy_prefix = "https://proxy.example.com/fetch?url=".to_string();
    config.proxy_special_upstream_urls = true;

    let body = json!({
        "status": "succeeded",
        "result": {
            "items": [{
                "url": "https://pub.example.com/demo.png",
                "type": "image"
            }]
        }
    });

    let output = rust_sync_proxy::response_rewrite::normalize_aiapidev_task_response(
        body,
        rust_sync_proxy::response_rewrite::OutputMode::Url,
        &reqwest::Client::new(),
        None,
        &config,
    )
    .await
    .unwrap();

    assert_eq!(output["candidates"][0]["content"]["role"], "model");
    assert_eq!(
        output["candidates"][0]["content"]["parts"][0]["inlineData"],
        json!({
            "mimeType": "image/png",
            "data": "https://proxy.example.com/fetch?url=https%3A%2F%2Fpub.example.com%2Fdemo.png"
        })
    );
    assert_eq!(
        output["usageMetadata"],
        json!({
            "candidatesTokenCount": 1024,
            "promptTokenCount": 1024,
            "totalTokenCount": 2048
        })
    );
}

#[tokio::test]
async fn aiapidev_task_response_url_mode_falls_back_to_public_base_url_proxy_path() {
    let mut config = rust_sync_proxy::test_config();
    config.public_base_url = "https://proxy.example.com/base/".to_string();
    config.proxy_special_upstream_urls = true;

    let body = json!({
        "status": "succeeded",
        "result": {
            "items": [{
                "url": "https://pub.example.com/demo.png",
                "type": "image"
            }]
        }
    });

    let output = rust_sync_proxy::response_rewrite::normalize_aiapidev_task_response(
        body,
        rust_sync_proxy::response_rewrite::OutputMode::Url,
        &reqwest::Client::new(),
        None,
        &config,
    )
    .await
    .unwrap();

    assert_eq!(
        output["candidates"][0]["content"]["parts"][0]["inlineData"],
        json!({
            "mimeType": "image/png",
            "data": "https://proxy.example.com/base/proxy/image?url=https%3A%2F%2Fpub.example.com%2Fdemo.png"
        })
    );
}

#[tokio::test]
async fn aiapidev_task_response_base64_mode_downloads_image() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let request_count = Arc::new(AtomicUsize::new(0));
    let state = MarkdownImageState {
        png: vec![137, 80, 78, 71, 13, 10, 26, 10],
        request_count: Arc::clone(&request_count),
    };
    let app = Router::new()
        .route("/image.png", get(serve_markdown_png))
        .with_state(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let mut config = rust_sync_proxy::test_config();
    config.inline_data_url_memory_cache_max_bytes = 1024;
    config.inline_data_url_background_fetch_wait_timeout = Duration::from_millis(100);
    config.inline_data_url_background_fetch_total_timeout = Duration::from_millis(500);
    let fetch_service = rust_sync_proxy::cache::InlineDataUrlFetchService::from_response_config(
        &config,
        reqwest::Client::new(),
        rust_sync_proxy::image_io::DEFAULT_MAX_IMAGE_BYTES,
        true,
    )
    .unwrap();

    let body = json!({
        "status": "succeeded",
        "result": {
            "items": [{
                "url": format!("http://{address}/image.png"),
                "type": "image"
            }]
        }
    });

    let output = rust_sync_proxy::response_rewrite::normalize_aiapidev_task_response(
        body,
        rust_sync_proxy::response_rewrite::OutputMode::Base64,
        &reqwest::Client::new(),
        Some(&fetch_service),
        &config,
    )
    .await
    .unwrap();

    assert_eq!(
        output["candidates"][0]["content"]["parts"][0]["inlineData"]["data"],
        "iVBORw0KGgo="
    );
    assert_eq!(request_count.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn aiapidev_task_response_base64_mode_allows_images_over_request_limit() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let request_count = Arc::new(AtomicUsize::new(0));
    let state = MarkdownImageState {
        png: vec![7_u8; rust_sync_proxy::image_io::REQUEST_MAX_IMAGE_BYTES + 1],
        request_count: Arc::clone(&request_count),
    };
    let app = Router::new()
        .route("/large-image.png", get(serve_markdown_png))
        .with_state(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let mut config = rust_sync_proxy::test_config();
    config.inline_data_url_memory_cache_max_bytes = 1024;
    config.inline_data_url_background_fetch_wait_timeout = Duration::from_millis(100);
    config.inline_data_url_background_fetch_total_timeout = Duration::from_millis(500);
    let fetch_service = rust_sync_proxy::cache::InlineDataUrlFetchService::from_response_config(
        &config,
        reqwest::Client::new(),
        rust_sync_proxy::image_io::DEFAULT_MAX_IMAGE_BYTES,
        true,
    )
    .unwrap();

    let body = json!({
        "status": "succeeded",
        "result": {
            "items": [{
                "url": format!("http://{address}/large-image.png"),
                "type": "image"
            }]
        }
    });

    let output = rust_sync_proxy::response_rewrite::normalize_aiapidev_task_response(
        body,
        rust_sync_proxy::response_rewrite::OutputMode::Base64,
        &reqwest::Client::new(),
        Some(&fetch_service),
        &config,
    )
    .await
    .unwrap();

    let encoded = output["candidates"][0]["content"]["parts"][0]["inlineData"]["data"]
        .as_str()
        .unwrap();
    assert!(!encoded.is_empty());
    assert_eq!(
        output["candidates"][0]["content"]["parts"][0]["inlineData"]["mimeType"],
        "image/png"
    );
    assert_eq!(request_count.load(Ordering::Relaxed), 1);
}

async fn serve_markdown_png(
    State(state): State<MarkdownImageState>,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    state.request_count.fetch_add(1, Ordering::Relaxed);
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("image/png"));
    (StatusCode::OK, headers, state.png)
}
