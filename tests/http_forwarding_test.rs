use std::sync::Arc;

use axum::body::{Body, Bytes, to_bytes};
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, Request, StatusCode, Uri};
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tower::ServiceExt;

#[derive(Clone, Default)]
struct UpstreamCapture {
    request_body: Arc<Mutex<Vec<u8>>>,
    query_string: Arc<Mutex<String>>,
    api_key: Arc<Mutex<String>>,
    authorization: Arc<Mutex<String>>,
}

#[derive(Clone, Default)]
struct UploadCapture {
    request_count: Arc<Mutex<usize>>,
    content_type: Arc<Mutex<String>>,
    user_agent: Arc<Mutex<String>>,
}

#[derive(Clone, Default)]
struct R2Capture {
    request_count: Arc<Mutex<usize>>,
    method: Arc<Mutex<String>>,
    path: Arc<Mutex<String>>,
    authorization: Arc<Mutex<String>>,
    content_type: Arc<Mutex<String>>,
}

#[derive(Clone)]
struct ImageState {
    png: Vec<u8>,
}

#[tokio::test]
async fn generate_content_forwards_rewritten_request_and_normalizes_response_in_base64_mode() {
    let capture = UpstreamCapture::default();
    let upstream = Router::new()
        .route(
            "/v1beta/models/demo:generateContent",
            post(mock_generate_content),
        )
        .with_state(capture.clone());
    let upstream_addr = spawn_server(upstream).await;

    let mut config = rust_sync_proxy::test_config();
    config.upstream_base_url = format!("http://{}", upstream_addr);
    config.upstream_api_key = "env-key".to_string();

    let app = rust_sync_proxy::build_router(config);
    let request_body = json!({
        "output": "base64",
        "contents": [{"parts": [{"text": "hello"}]}]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent?lang=zh")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(request_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json_body: Value = serde_json::from_slice(&body).unwrap();

    assert!(json_body.get("thoughtSignature").is_none());
    let parts = json_body["candidates"][0]["content"]["parts"]
        .as_array()
        .unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0]["text"], "kept");
    assert_eq!(parts[1]["inlineData"]["data"], "aaaaaaaa");

    let captured_body = String::from_utf8(capture.request_body.lock().await.clone()).unwrap();
    assert!(!captured_body.contains("\"output\""));
    assert_eq!(*capture.query_string.lock().await, "lang=zh");
    assert_eq!(*capture.api_key.lock().await, "env-key");
    assert_eq!(*capture.authorization.lock().await, "Bearer env-key");
}

#[tokio::test]
async fn generate_content_rewrites_inline_data_to_wrapped_urls_when_output_url_enabled() {
    let capture = UpstreamCapture::default();
    let upstream = Router::new()
        .route(
            "/v1beta/models/demo:generateContent",
            post(mock_generate_content),
        )
        .with_state(capture.clone());
    let upstream_addr = spawn_server(upstream).await;

    let upload_capture = UploadCapture::default();
    let upload = Router::new()
        .route("/uguu", post(mock_legacy_upload))
        .with_state(upload_capture.clone());
    let upload_addr = spawn_server(upload).await;

    let mut config = rust_sync_proxy::test_config();
    config.upstream_base_url = format!("http://{}", upstream_addr);
    config.upstream_api_key = "env-key".to_string();
    config.public_base_url = "https://proxy.example.com".to_string();
    config.legacy_uguu_upload_url = format!("http://{upload_addr}/uguu");

    let app = rust_sync_proxy::build_router(config);
    let request_body = json!({
        "output": "url",
        "contents": [{"parts": [{"text": "hello"}]}]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent?lang=zh")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(request_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json_body: Value = serde_json::from_slice(&body).unwrap();

    assert!(json_body.get("thoughtSignature").is_none());
    let parts = json_body["candidates"][0]["content"]["parts"]
        .as_array()
        .unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0]["text"], "kept");
    assert_eq!(
        parts[1]["inlineData"]["data"],
        "https://proxy.example.com/proxy/image?url=https%3A%2F%2Fh.uguu.se%2Ffixed-image.png"
    );

    let captured_body = String::from_utf8(capture.request_body.lock().await.clone()).unwrap();
    assert!(!captured_body.contains("\"output\""));
    assert_eq!(*capture.query_string.lock().await, "lang=zh");
    assert_eq!(*capture.api_key.lock().await, "env-key");
    assert_eq!(*capture.authorization.lock().await, "Bearer env-key");
    assert_eq!(*upload_capture.request_count.lock().await, 1);
    assert!(
        upload_capture
            .content_type
            .lock()
            .await
            .starts_with("multipart/form-data; boundary=")
    );
    assert_eq!(
        *upload_capture.user_agent.lock().await,
        "ComfyUI-Banana/1.0"
    );
}

#[tokio::test]
async fn generate_content_materializes_request_image_urls_before_forwarding() {
    let capture = UpstreamCapture::default();
    let upstream = Router::new()
        .route(
            "/v1beta/models/demo:generateContent",
            post(mock_generate_content),
        )
        .with_state(capture.clone());
    let upstream_addr = spawn_server(upstream).await;

    let image_server = Router::new()
        .route("/image.png", get(serve_png))
        .with_state(ImageState {
            png: vec![137, 80, 78, 71, 13, 10, 26, 10],
        });
    let image_addr = spawn_server(image_server).await;

    let mut config = rust_sync_proxy::test_config();
    config.upstream_base_url = format!("http://{}", upstream_addr);
    config.upstream_api_key = "env-key".to_string();

    let app = rust_sync_proxy::build_router(config);
    let request_body = json!({
        "contents": [{
            "parts": [{
                "inlineData": {
                    "data": format!("http://{image_addr}/image.png")
                }
            }]
        }]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(request_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let captured_body = capture.request_body.lock().await.clone();
    let json_body: Value = serde_json::from_slice(&captured_body).unwrap();
    let inline_data = &json_body["contents"][0]["parts"][0]["inlineData"];
    assert_eq!(inline_data["mimeType"], "image/png");
    assert_eq!(inline_data["data"], "iVBORw0KGgo=");
}

#[tokio::test]
async fn generate_content_rewrites_inline_data_to_direct_r2_url_when_r2_enabled() {
    let capture = UpstreamCapture::default();
    let upstream = Router::new()
        .route(
            "/v1beta/models/demo:generateContent",
            post(mock_generate_content),
        )
        .with_state(capture.clone());
    let upstream_addr = spawn_server(upstream).await;

    let r2_capture = R2Capture::default();
    let r2 = Router::new()
        .route("/{*path}", put(mock_r2_upload_success))
        .with_state(r2_capture.clone());
    let r2_addr = spawn_server(r2).await;

    let mut config = rust_sync_proxy::test_config();
    config.upstream_base_url = format!("http://{}", upstream_addr);
    config.upstream_api_key = "env-key".to_string();
    config.public_base_url = "https://proxy.example.com".to_string();
    config.image_host_mode = "r2".to_string();
    config.r2_endpoint = format!("http://{r2_addr}");
    config.r2_bucket = "bucket".to_string();
    config.r2_access_key_id = "test-key".to_string();
    config.r2_secret_access_key = "test-secret".to_string();
    config.r2_public_base_url = "https://img.example.com".to_string();
    config.r2_object_prefix = "images".to_string();

    let app = rust_sync_proxy::build_router(config);
    let request_body = json!({
        "output": "url",
        "contents": [{"parts": [{"text": "hello"}]}]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(request_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json_body: Value = serde_json::from_slice(&body).unwrap();

    let parts = json_body["candidates"][0]["content"]["parts"]
        .as_array()
        .unwrap();
    let image_url = parts[1]["inlineData"]["data"].as_str().unwrap();
    assert!(image_url.starts_with("https://img.example.com/images/"));
    assert!(image_url.ends_with(".png"));
    assert!(!image_url.contains("/proxy/image?url="));

    assert_eq!(*r2_capture.request_count.lock().await, 1);
    assert_eq!(*r2_capture.method.lock().await, "PUT");
    assert!(r2_capture.path.lock().await.starts_with("/bucket/images/"));
    assert!(
        r2_capture
            .authorization
            .lock()
            .await
            .starts_with("AWS4-HMAC-SHA256 Credential=test-key/")
    );
    assert_eq!(*r2_capture.content_type.lock().await, "image/png");
}

#[tokio::test]
async fn generate_content_falls_back_to_legacy_wrapped_url_when_r2_then_legacy_enabled() {
    let capture = UpstreamCapture::default();
    let upstream = Router::new()
        .route(
            "/v1beta/models/demo:generateContent",
            post(mock_generate_content),
        )
        .with_state(capture.clone());
    let upstream_addr = spawn_server(upstream).await;

    let r2_capture = R2Capture::default();
    let r2 = Router::new()
        .route("/{*path}", put(mock_r2_upload_failure))
        .with_state(r2_capture.clone());
    let r2_addr = spawn_server(r2).await;

    let upload_capture = UploadCapture::default();
    let upload = Router::new()
        .route("/uguu", post(mock_legacy_upload))
        .with_state(upload_capture.clone());
    let upload_addr = spawn_server(upload).await;

    let mut config = rust_sync_proxy::test_config();
    config.upstream_base_url = format!("http://{}", upstream_addr);
    config.upstream_api_key = "env-key".to_string();
    config.public_base_url = "https://proxy.example.com".to_string();
    config.image_host_mode = "r2_then_legacy".to_string();
    config.legacy_uguu_upload_url = format!("http://{upload_addr}/uguu");
    config.r2_endpoint = format!("http://{r2_addr}");
    config.r2_bucket = "bucket".to_string();
    config.r2_access_key_id = "test-key".to_string();
    config.r2_secret_access_key = "test-secret".to_string();
    config.r2_public_base_url = "https://img.example.com".to_string();
    config.r2_object_prefix = "images".to_string();

    let app = rust_sync_proxy::build_router(config);
    let request_body = json!({
        "output": "url",
        "contents": [{"parts": [{"text": "hello"}]}]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(request_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json_body: Value = serde_json::from_slice(&body).unwrap();

    let parts = json_body["candidates"][0]["content"]["parts"]
        .as_array()
        .unwrap();
    assert_eq!(
        parts[1]["inlineData"]["data"],
        "https://proxy.example.com/proxy/image?url=https%3A%2F%2Fh.uguu.se%2Ffixed-image.png"
    );
    assert_eq!(*r2_capture.request_count.lock().await, 1);
    assert_eq!(*upload_capture.request_count.lock().await, 1);
}

#[tokio::test]
async fn generate_content_preserves_base64_when_r2_upload_fails() {
    let capture = UpstreamCapture::default();
    let upstream = Router::new()
        .route(
            "/v1beta/models/demo:generateContent",
            post(mock_generate_content),
        )
        .with_state(capture.clone());
    let upstream_addr = spawn_server(upstream).await;

    let r2_capture = R2Capture::default();
    let r2 = Router::new()
        .route("/{*path}", put(mock_r2_upload_failure))
        .with_state(r2_capture.clone());
    let r2_addr = spawn_server(r2).await;

    let mut config = rust_sync_proxy::test_config();
    config.upstream_base_url = format!("http://{}", upstream_addr);
    config.upstream_api_key = "env-key".to_string();
    config.public_base_url = "https://proxy.example.com".to_string();
    config.image_host_mode = "r2".to_string();
    config.r2_endpoint = format!("http://{r2_addr}");
    config.r2_bucket = "bucket".to_string();
    config.r2_access_key_id = "test-key".to_string();
    config.r2_secret_access_key = "test-secret".to_string();
    config.r2_public_base_url = "https://img.example.com".to_string();
    config.r2_object_prefix = "images".to_string();

    let app = rust_sync_proxy::build_router(config);
    let request_body = json!({
        "output": "url",
        "contents": [{"parts": [{"text": "hello"}]}]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(request_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json_body: Value = serde_json::from_slice(&body).unwrap();

    let parts = json_body["candidates"][0]["content"]["parts"]
        .as_array()
        .unwrap();
    assert_eq!(parts[1]["inlineData"]["data"], "aaaaaaaa");
    assert_eq!(*r2_capture.request_count.lock().await, 1);
}

#[tokio::test]
async fn generate_content_normalizes_markdown_image_to_proxy_url_when_output_url_enabled() {
    let capture = UpstreamCapture::default();
    let upstream = Router::new()
        .route(
            "/v1beta/models/demo:generateContent",
            post(mock_generate_markdown_content),
        )
        .with_state(capture.clone());
    let upstream_addr = spawn_server(upstream).await;

    let mut config = rust_sync_proxy::test_config();
    config.upstream_base_url = format!("http://{}", upstream_addr);
    config.upstream_api_key = "env-key".to_string();
    config.public_base_url = "https://proxy.example.com".to_string();
    config.proxy_special_upstream_urls = true;

    let app = rust_sync_proxy::build_router(config);
    let request_body = json!({
        "output": "url",
        "contents": [{"parts": [{"text": "hello"}]}]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(request_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json_body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json_body["candidates"][0]["content"]["parts"],
        json!([{
            "inlineData": {
                "mimeType": "image/png",
                "data": "https://proxy.example.com/proxy/image?u=aHR0cHM6Ly9leGFtcGxlLmNvbS9wYXRoL2RlbW8ucG5n"
            }
        }])
    );
}

async fn mock_generate_content(
    State(capture): State<UpstreamCapture>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> impl IntoResponse {
    store_request_capture(&capture, &headers, &uri).await;
    *capture.request_body.lock().await = body.to_vec();

    Json(json!({
        "thoughtSignature": "secret",
        "candidates": [{
            "finishReason": "STOP",
            "content": {
                "parts": [
                    { "inlineData": { "mimeType": "image/png", "data": "aaaa" } },
                    { "text": "kept" },
                    { "inlineData": { "mimeType": "image/png", "data": "aaaaaaaa" } }
                ]
            }
        }]
    }))
}

async fn mock_generate_markdown_content(
    State(capture): State<UpstreamCapture>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> impl IntoResponse {
    store_request_capture(&capture, &headers, &uri).await;
    *capture.request_body.lock().await = body.to_vec();

    Json(json!({
        "candidates": [{
            "finishReason": "STOP",
            "content": {
                "parts": [
                    { "text": "before" },
                    { "text": "![img](https://example.com/path/demo.png)" },
                    { "text": "after" }
                ]
            }
        }]
    }))
}

async fn mock_legacy_upload(
    State(capture): State<UploadCapture>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    *capture.request_count.lock().await += 1;
    *capture.content_type.lock().await = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    *capture.user_agent.lock().await = headers
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(!body.is_empty());

    Json(json!({
        "success": true,
        "files": [{
            "url": "https://h.uguu.se/fixed-image.png"
        }]
    }))
}

async fn mock_r2_upload_success(
    State(capture): State<R2Capture>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> impl IntoResponse {
    store_r2_capture(&capture, &headers, &uri, body.len()).await;
    StatusCode::OK
}

async fn mock_r2_upload_failure(
    State(capture): State<R2Capture>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> impl IntoResponse {
    store_r2_capture(&capture, &headers, &uri, body.len()).await;
    (StatusCode::INTERNAL_SERVER_ERROR, "r2 down")
}

async fn store_request_capture(capture: &UpstreamCapture, headers: &HeaderMap, uri: &Uri) {
    *capture.query_string.lock().await = uri.query().unwrap_or_default().to_string();
    *capture.api_key.lock().await = headers
        .get("x-goog-api-key")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    *capture.authorization.lock().await = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
}

async fn store_r2_capture(capture: &R2Capture, headers: &HeaderMap, uri: &Uri, body_len: usize) {
    *capture.request_count.lock().await += 1;
    *capture.method.lock().await = "PUT".to_string();
    *capture.path.lock().await = uri.path().to_string();
    *capture.authorization.lock().await = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    *capture.content_type.lock().await = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(body_len > 0);
}

async fn serve_png(State(state): State<ImageState>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, "image/png".parse().unwrap());
    (StatusCode::OK, headers, state.png)
}

async fn spawn_server(app: Router) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    address
}
