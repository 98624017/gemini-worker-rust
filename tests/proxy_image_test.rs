use axum::body::Body;
use http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn rejects_loopback_proxy_target() {
    let app = rust_sync_proxy::build_router(rust_sync_proxy::test_config());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/proxy/image?url=http://127.0.0.1/test.png")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
