use axum::body::Body;
use axum::http::{StatusCode, header::CONTENT_TYPE};
use axum::routing::post;
use axum::{Json, Router};
use http::Request;
use serde_json::json;
use tokio::net::TcpListener;
use tower::ServiceExt;

#[tokio::test]
async fn unknown_route_returns_404() {
    let app = rust_sync_proxy::build_router(rust_sync_proxy::test_config());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/not-found")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn generate_content_output_url_smoke_still_passes_after_blob_runtime_refactor() {
    let upstream = Router::new().route(
        "/v1beta/models/demo:generateContent",
        post(mock_generate_content),
    );
    let upstream_addr = spawn_server(upstream).await;

    let upload_server = Router::new().route("/uguu", post(mock_upload));
    let upload_addr = spawn_server(upload_server).await;

    let mut config = rust_sync_proxy::test_config();
    config.upstream_base_url = format!("http://{}", upstream_addr);
    config.upstream_api_key = "env-key".to_string();
    config.external_image_proxy_prefix = "https://proxy.example.com/fetch?url=".to_string();
    config.legacy_uguu_upload_url = format!("http://{upload_addr}/uguu");

    let app = rust_sync_proxy::build_router(config);
    let request_body = json!({
        "output": "url",
        "contents": [{
            "parts": [{
                "text": "hello"
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
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json_body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json_body["candidates"][0]["content"]["parts"][0]["inlineData"]["data"],
        "https://proxy.example.com/fetch?url=https%3A%2F%2Fimg.example.com%2Fsmoke.png"
    );
}

async fn mock_generate_content() -> Json<serde_json::Value> {
    Json(json!({
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
    }))
}

async fn mock_upload() -> Json<serde_json::Value> {
    Json(json!({
        "success": true,
        "files": [{
            "url": "https://img.example.com/smoke.png"
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
