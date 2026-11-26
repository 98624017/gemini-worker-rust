use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tower::ServiceExt;

use super::{AppState, image_generations_action};
use crate::upload::Uploader;

#[tokio::test]
async fn ai666_seedream_preserves_the_sync_image_contract() {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let upstream = Router::new().route(
        "/v1/images/generations",
        post({
            let captured = captured.clone();
            move |headers: HeaderMap, Json(body): Json<Value>| async move {
                captured.lock().await.push((headers, body));
                Json(json!({
                    "created": 1790729428_i64,
                    "data": [{"url": "https://images.example/generated.png?token=a&part=b"}],
                    "usage": {"total_tokens": 20}
                }))
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });

    let mut config = crate::test_config();
    config.openai_image_upstream_url_proxy_prefix = "https://proxy.example/image?url=".to_string();
    let state = AppState {
        config: Arc::new(config.clone()),
        upstream_client: reqwest::Client::builder()
            .no_proxy()
            .resolve("us.ai666.live", address)
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
        image_client: reqwest::Client::new(),
        uploader: Arc::new(Uploader::new(reqwest::Client::new(), config)),
        admin: None,
        request_inline_data_fetch_service: None,
        response_inline_data_fetch_service: None,
        blob_runtime: Arc::new(crate::test_blob_runtime(1024)),
        upstream_block_cache: None,
    };
    let app = Router::new()
        .route("/v1/images/generations", post(image_generations_action))
        .with_state(state);
    let post_image = |body: Value| {
        app.clone().oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/images/generations")
                .header(CONTENT_TYPE, "application/json")
                .header(
                    AUTHORIZATION,
                    format!(
                        "Bearer http://us.ai666.live:{}/v1/|ai666-test-key",
                        address.port()
                    ),
                )
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
    };
    let prompt = "poster with a geometric pattern";
    let text = json!({"model": "seedream-5.5", "prompt": prompt, "size": "1K"});
    let images: Vec<_> = (0..11)
        .map(|index| format!("https://images.example/ref-{index}.png"))
        .collect();
    let mut reference = json!({
        "model": "seedream-5.5", "prompt": prompt, "size": "2k", "aspect_ratio": "3:2",
        "reference_images": images[..3], "images": images[3..7], "image": images[7..10],
        "response_format": "url"
    });

    for request in [text, reference.clone()] {
        let response = post_image(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["created"], 1790729428_i64);
        assert_eq!(
            body["data"],
            json!([{
                "url": "https://proxy.example/image?url=https%3A%2F%2Fimages.example%2Fgenerated.png%3Ftoken%3Da%26part%3Db"
            }])
        );
        assert_eq!(body["usage"]["total_tokens"], 2048);
    }
    reference["image"] = json!(images[7..]);
    assert_eq!(
        post_image(reference).await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );

    let captured = captured.lock().await;
    assert_eq!(captured.len(), 2);
    for (headers, body) in captured.iter() {
        assert_eq!(headers[AUTHORIZATION], "Bearer ai666-test-key");
        assert_eq!(body["model"], "seedream-5.5");
        assert_eq!(body["prompt"], prompt);
        assert_eq!(body["response_format"], "url");
        for removed in ["size", "image", "reference_images"] {
            assert!(body.get(removed).is_none(), "unexpected field {removed}");
        }
    }
    let text_body = &captured[0].1;
    assert_eq!(text_body["resolution"], "1K");
    assert!(text_body.get("images").is_none());
    let reference_body = &captured[1].1;
    assert_eq!(reference_body["resolution"], "2K");
    assert_eq!(reference_body["aspect_ratio"], "3:2");
    assert_eq!(reference_body["images"], json!(images[..10]));
    server.abort();
}
