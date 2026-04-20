use std::sync::Arc;

use axum::body::{Body, Bytes, to_bytes};
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use axum::http::{HeaderMap, Request, StatusCode, Uri};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tower::ServiceExt;

#[derive(Clone, Debug, PartialEq, Eq)]
struct CapturedUpstreamRequest {
    body: Vec<u8>,
    query: String,
    api_key: String,
    authorization: String,
}

#[derive(Clone, Default)]
struct TestState {
    upstream_requests: Arc<Mutex<Vec<CapturedUpstreamRequest>>>,
    upload_count: Arc<Mutex<usize>>,
    upload_content_types: Arc<Mutex<Vec<String>>>,
    upload_user_agents: Arc<Mutex<Vec<String>>>,
}

#[tokio::test]
async fn generate_content_forwards_expected_upstream_request_and_output_url_result() {
    let state = TestState::default();
    let server = Router::new()
        .route(
            "/v1beta/models/demo:generateContent",
            post(mock_generate_content),
        )
        .route("/uguu", post(mock_legacy_upload))
        .with_state(state.clone());
    let server_addr = spawn_server(server).await;

    let mut config = rust_sync_proxy::test_config();
    config.upstream_base_url = format!("http://{server_addr}");
    config.upstream_api_key = "env-key".to_string();
    config.external_image_proxy_prefix = "https://proxy.example.com/fetch?url=".to_string();
    config.legacy_uguu_upload_url = format!("http://{server_addr}/uguu");

    let app = rust_sync_proxy::build_router(config);

    let base64_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent?lang=zh")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "output": "base64",
                        "contents": [{
                            "parts": [{
                                "text": "hello"
                            }]
                        }]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(base64_response.status(), StatusCode::OK);
    let base64_body = to_bytes(base64_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let base64_json: Value = serde_json::from_slice(&base64_body).unwrap();
    assert!(base64_json.get("thoughtSignature").is_none());
    assert_eq!(
        base64_json["candidates"][0]["content"]["parts"],
        json!([
            {"text": "kept"},
            {"inlineData": {"mimeType": "image/png", "data": "AQID"}}
        ])
    );

    let url_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent?lang=zh")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "output": "url",
                        "contents": [{
                            "parts": [{
                                "text": "hello"
                            }]
                        }]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(url_response.status(), StatusCode::OK);
    let url_body = to_bytes(url_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let url_json: Value = serde_json::from_slice(&url_body).unwrap();
    assert_eq!(
        url_json["candidates"][0]["content"]["parts"],
        json!([
            {"text": "kept"},
            {
                "inlineData": {
                    "mimeType": "image/png",
                    "data": "https://proxy.example.com/fetch?url=https%3A%2F%2Fimg.example.com%2Fforwarded.png"
                }
            }
        ])
    );

    let upstream_requests = state.upstream_requests.lock().await.clone();
    assert_eq!(upstream_requests.len(), 2);
    for captured in upstream_requests {
        let upstream_json: Value = serde_json::from_slice(&captured.body).unwrap();
        assert!(upstream_json.get("output").is_none());
        assert_eq!(
            upstream_json["contents"][0]["parts"][0]["text"],
            Value::String("hello".to_string())
        );
        assert_eq!(captured.query, "lang=zh");
        assert_eq!(captured.api_key, "env-key");
        assert_eq!(captured.authorization, "Bearer env-key");
    }

    assert_eq!(*state.upload_count.lock().await, 1);
    assert!(
        state
            .upload_content_types
            .lock()
            .await
            .iter()
            .all(|value| value.starts_with("multipart/form-data; boundary="))
    );
    assert_eq!(
        state.upload_user_agents.lock().await.as_slice(),
        ["ComfyUI-Banana/1.0"]
    );
}

#[tokio::test]
async fn invalid_request_json_returns_structured_proxy_error() {
    let app = rust_sync_proxy::build_router(rust_sync_proxy::test_config());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"contents":[{"parts":[{"text":"hello"}]}]"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json_body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json_body["error"]["code"], 502);
    assert_eq!(json_body["error"]["message"], "invalid request json body");
    assert_eq!(json_body["error"]["source"], "proxy");
    assert_eq!(json_body["error"]["stage"], "parse_request_json");
    assert_eq!(json_body["error"]["kind"], "invalid_json");
}

#[tokio::test]
async fn truncated_upstream_body_returns_structured_proxy_error() {
    let server_addr = spawn_truncated_body_server().await;

    let mut config = rust_sync_proxy::test_config();
    config.upstream_base_url = format!("http://{server_addr}");
    config.upstream_api_key = "env-key".to_string();
    let app = rust_sync_proxy::build_router(config);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "contents": [{
                            "parts": [{
                                "text": "hello"
                            }]
                        }]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json_body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json_body["error"]["code"], 502);
    assert_eq!(
        json_body["error"]["message"],
        "failed to read upstream response body"
    );
    assert_eq!(json_body["error"]["source"], "proxy");
    assert_eq!(json_body["error"]["stage"], "read_upstream_body");
    assert_eq!(json_body["error"]["kind"], "body_truncated");
}

#[tokio::test]
async fn upstream_json_error_preserves_message_and_adds_proxy_metadata() {
    let server = Router::new().route(
        "/v1beta/models/demo:generateContent",
        post(mock_generate_content_rate_limited_json),
    );
    let server_addr = spawn_server(server).await;

    let mut config = rust_sync_proxy::test_config();
    config.upstream_base_url = format!("http://{server_addr}");
    config.upstream_api_key = "env-key".to_string();
    let app = rust_sync_proxy::build_router(config);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "contents": [{
                            "parts": [{
                                "text": "hello"
                            }]
                        }]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json_body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json_body["error"]["message"], "rate limited");
    assert_eq!(json_body["error"]["code"], 429);
    assert_eq!(json_body["error"]["source"], "upstream");
    assert_eq!(json_body["error"]["stage"], "upstream_response");
    assert_eq!(json_body["error"]["kind"], "upstream_error");
}

#[tokio::test]
async fn dual_upstream_header_routes_4k_requests_to_second_standard_upstream() {
    let first_state = TestState::default();
    let first_server = Router::new()
        .route(
            "/v1beta/models/demo:generateContent",
            post(mock_generate_content),
        )
        .with_state(first_state.clone());
    let first_addr = spawn_server(first_server).await;

    let second_state = TestState::default();
    let second_server = Router::new()
        .route(
            "/v1beta/models/demo:generateContent",
            post(mock_generate_content),
        )
        .with_state(second_state.clone());
    let second_addr = spawn_server(second_server).await;

    let mut config = rust_sync_proxy::test_config();
    config.upstream_base_url = format!("http://{first_addr}");
    config.upstream_api_key = "env-key".to_string();
    let app = rust_sync_proxy::build_router(config);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent")
                .header(CONTENT_TYPE, "application/json")
                .header(
                    "x-goog-api-key",
                    format!("http://{first_addr}|first-key,http://{second_addr}|second-key"),
                )
                .body(Body::from(
                    json!({
                        "generationConfig": {
                            "imageConfig": {
                                "imageSize": "4k"
                            }
                        },
                        "contents": [{
                            "parts": [{
                                "text": "hello"
                            }]
                        }]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let first_requests = first_state.upstream_requests.lock().await.clone();
    let second_requests = second_state.upstream_requests.lock().await.clone();
    assert!(first_requests.is_empty());
    assert_eq!(second_requests.len(), 1);
    assert_eq!(second_requests[0].api_key, "second-key");
    assert_eq!(second_requests[0].authorization, "Bearer second-key");
}

#[tokio::test]
async fn malformed_dual_upstream_header_returns_bad_request() {
    let app = rust_sync_proxy::build_router(rust_sync_proxy::test_config());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent")
                .header(CONTENT_TYPE, "application/json")
                .header(
                    "x-goog-api-key",
                    "https://first.example|first-key,second-key-only",
                )
                .body(Body::from(
                    json!({
                        "generationConfig": {
                            "imageConfig": {
                                "imageSize": "4k"
                            }
                        },
                        "contents": [{
                            "parts": [{
                                "text": "hello"
                            }]
                        }]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json_body: Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json_body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("dual upstream")
    );
}

#[tokio::test]
async fn image_generations_forwards_reference_images_and_returns_uploaded_urls() {
    let state = TestState::default();
    let server = Router::new()
        .route("/v1/images/generations", post(mock_openai_image_generation))
        .route("/uguu", post(mock_legacy_upload))
        .with_state(state.clone());
    let server_addr = spawn_server(server).await;

    let mut config = rust_sync_proxy::test_config();
    config.upstream_base_url = format!("http://{server_addr}");
    config.upstream_api_key = "env-key".to_string();
    config.legacy_uguu_upload_url = format!("http://{server_addr}/uguu");
    let app = rust_sync_proxy::build_router(config);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/images/generations")
                .header(CONTENT_TYPE, "application/json")
                .header(
                    AUTHORIZATION,
                    format!("Bearer http://{server_addr}|real-upstream-key"),
                )
                .body(Body::from(
                    json!({
                        "model": "gpt-image-2",
                        "prompt": "draw cat",
                        "image": ["https://img.example/a.png"],
                        "response_format": "url"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json_body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json_body["created"], 1_776_663_103);
    assert_eq!(
        json_body["data"][0]["url"],
        "https://img.example.com/forwarded.png"
    );
    assert_eq!(json_body["usage"]["total_tokens"], 2048);

    let upstream_requests = state.upstream_requests.lock().await.clone();
    assert_eq!(upstream_requests.len(), 1);
    let upstream_json: Value = serde_json::from_slice(&upstream_requests[0].body).unwrap();
    assert_eq!(
        upstream_json["reference_images"],
        json!(["https://img.example/a.png"])
    );
    assert_eq!(upstream_json["response_format"], "b64_json");
    assert!(upstream_json.get("image").is_none());
    assert!(upstream_json.get("images").is_none());
    assert_eq!(upstream_requests[0].api_key, "");
    assert_eq!(
        upstream_requests[0].authorization,
        "Bearer real-upstream-key"
    );

    assert_eq!(*state.upload_count.lock().await, 1);
}

#[tokio::test]
async fn image_generations_returns_bad_gateway_when_upstream_data_is_empty() {
    let state = TestState::default();
    let server = Router::new()
        .route(
            "/v1/images/generations",
            post(mock_openai_image_generation_empty_data),
        )
        .route("/uguu", post(mock_legacy_upload))
        .with_state(state.clone());
    let server_addr = spawn_server(server).await;

    let mut config = rust_sync_proxy::test_config();
    config.upstream_base_url = format!("http://{server_addr}");
    config.upstream_api_key = "env-key".to_string();
    config.legacy_uguu_upload_url = format!("http://{server_addr}/uguu");
    let app = rust_sync_proxy::build_router(config);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/images/generations")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "model": "gpt-image-2",
                        "prompt": "draw cat",
                        "images": ["https://img.example/a.png"]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json_body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json_body["error"]["message"],
        "upstream response missing data"
    );
    assert_eq!(*state.upload_count.lock().await, 0);
}

async fn mock_generate_content(
    State(state): State<TestState>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Json<Value> {
    let api_key = headers
        .get("x-goog-api-key")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let authorization = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();

    state
        .upstream_requests
        .lock()
        .await
        .push(CapturedUpstreamRequest {
            body: body.to_vec(),
            query: uri.query().unwrap_or_default().to_string(),
            api_key,
            authorization,
        });

    Json(json!({
        "thoughtSignature": "secret",
        "candidates": [{
            "finishReason": "STOP",
            "content": {
                "parts": [
                    {"text": "kept"},
                    {"inlineData": {"mimeType": "image/png", "data": "AQID"}}
                ]
            }
        }]
    }))
}

async fn mock_openai_image_generation(
    State(state): State<TestState>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Json<Value> {
    let authorization = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();

    state
        .upstream_requests
        .lock()
        .await
        .push(CapturedUpstreamRequest {
            body: body.to_vec(),
            query: uri.query().unwrap_or_default().to_string(),
            api_key: headers
                .get("x-goog-api-key")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_string(),
            authorization,
        });

    Json(json!({
        "created": 1_776_663_103,
        "data": [{
            "b64_json": "iVBORw0KGgo="
        }]
    }))
}

async fn mock_openai_image_generation_empty_data(
    State(state): State<TestState>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Json<Value> {
    let _ = mock_openai_image_generation(State(state), headers, uri, body).await;
    Json(json!({
        "created": 1_776_663_103,
        "data": []
    }))
}

async fn mock_generate_content_rate_limited_json() -> (StatusCode, Json<Value>) {
    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(json!({
            "error": {
                "message": "rate limited"
            }
        })),
    )
}

async fn mock_legacy_upload(
    State(state): State<TestState>,
    headers: HeaderMap,
    _body: Bytes,
) -> Json<Value> {
    *state.upload_count.lock().await += 1;
    state.upload_content_types.lock().await.push(
        headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string(),
    );
    state.upload_user_agents.lock().await.push(
        headers
            .get(USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string(),
    );

    Json(json!({
        "success": true,
        "files": [{
            "url": "https://img.example.com/forwarded.png"
        }]
    }))
}

async fn spawn_server(app: Router) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    address
}

async fn spawn_truncated_body_server() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request_buf = vec![0u8; 4096];
        let _ = socket.read(&mut request_buf).await;
        let response = concat!(
            "HTTP/1.1 200 OK\r\n",
            "Content-Type: application/json\r\n",
            "Content-Length: 80\r\n",
            "Connection: close\r\n",
            "\r\n",
            "{\"candidates\":[{\"content\":{\"parts\":[]}}]}"
        );
        let _ = socket.write_all(response.as_bytes()).await;
    });
    address
}
