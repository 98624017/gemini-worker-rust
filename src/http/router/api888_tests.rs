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
async fn api888_seedream_uses_sync_json_and_validates_reference_limit() {
    const RESULT_URL: &str = "https://images.example/generated.png";
    let captured = Arc::new(Mutex::new(Vec::new()));
    let upstream = Router::new().route(
        "/v1/images/generations",
        post({
            let captured = captured.clone();
            move |headers: HeaderMap, Json(body): Json<Value>| async move {
                captured.lock().await.push((headers, body));
                Json(json!({"created": 1790729428_i64, "data": [{"url": RESULT_URL}]}))
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });

    let mut config = crate::test_config();
    config
        .openai_image_edits_upstream_domains
        .push("888api.xin".to_string());
    let client = reqwest::Client::builder()
        .no_proxy()
        .resolve("888api.xin", address)
        .resolve("images.example", address)
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let state = AppState {
        config: Arc::new(config.clone()),
        upstream_client: client.clone(),
        image_client: client.clone(),
        uploader: Arc::new(Uploader::new(client, config)),
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
                        "Bearer http://888api.xin:{}/v1/|api888-test-key",
                        address.port()
                    ),
                )
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
    };

    let prompt = "poster with a geometric pattern";
    let text = json!({"model": "seedream-5.5", "prompt": prompt, "size": "1K", "image": []});
    let images: Vec<_> = (0..11)
        .map(|index| format!("https://images.example/ref-{index}.png"))
        .collect();
    let mut reference = json!({
        "model": "seedream-5.5", "prompt": prompt, "size": "2K", "aspect_ratio": "3:2",
        "reference_images": images[..3], "images": images[3..7], "image": images[7..10],
        "watermark": false, "background": "transparent", "output_format": "png"
    });

    for request in [text, reference.clone()] {
        let response = post_image(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["created"], 1790729428_i64);
        assert_eq!(body["data"], json!([{"url": RESULT_URL}]));
        assert_eq!(body["usage"]["total_tokens"], 2048);
    }

    reference["image"] = json!(images[7..]);
    let response = post_image(reference).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["error"]["kind"], "too_many_reference_images");

    let invalid_count = json!({"model": "seedream-5.5", "prompt": prompt, "n": 2});
    assert_eq!(
        post_image(invalid_count).await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );

    let captured = captured.lock().await;
    assert_eq!(captured.len(), 2);
    for (headers, _) in captured.iter() {
        assert_eq!(headers[AUTHORIZATION], "Bearer api888-test-key");
    }
    assert_eq!(
        captured[0].1,
        json!({
            "model": "seedream-5.5", "prompt": prompt, "size": "2048x2048",
            "n": 1, "response_format": "url"
        })
    );
    assert_eq!(
        captured[1].1,
        json!({
            "model": "seedream-5.5", "prompt": prompt, "size": "2496x1664",
            "n": 1, "response_format": "url", "images": images[..10],
            "background": "transparent", "output_format": "png"
        })
    );
    server.abort();
}
