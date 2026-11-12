use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    routing::post,
};
use serde_json::{Value, json};
use tower::ServiceExt;

#[tokio::test]
async fn invalid_base64_tail_is_rejected_before_any_upload() {
    for data_url in [false, true] {
        let invalid = format!("{}!", "A".repeat(64 * 1024));
        let upstream = if data_url {
            json!({"data": [{"url": format!("data:image/png;base64,{invalid}")}]})
        } else {
            json!({"data": [{"b64_json": invalid}]})
        };
        let uploads = Arc::new(AtomicUsize::new(0));
        let counter = uploads.clone();
        let server = Router::new()
            .route(
                "/v1/images/generations",
                post(move || {
                    let response = upstream.clone();
                    async move { Json(response) }
                }),
            )
            .route(
                "/upload",
                post(move || {
                    counter.fetch_add(1, Ordering::Relaxed);
                    async { StatusCode::INTERNAL_SERVER_ERROR }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
        let mut config = rust_sync_proxy::test_config();
        config.upstream_base_url = base.clone();
        config.legacy_uguu_upload_url = format!("{base}/upload");
        config.legacy_kefan_upload_url = format!("{base}/upload");
        let response = rust_sync_proxy::build_router(config)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/images/generations")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"model":"gpt-image-2","prompt":"test","response_format":"url"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let error: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["error"]["kind"], "invalid_base64");
        assert_eq!(
            error["error"]["stage"],
            if data_url {
                "decode_data_url"
            } else {
                "decode_b64_json"
            }
        );
        assert_eq!(uploads.load(Ordering::Relaxed), 0);
        server.abort();
        let _ = server.await;
    }
}
