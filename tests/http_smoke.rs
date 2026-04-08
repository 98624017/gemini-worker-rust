use axum::body::Body;
use http::{Request, StatusCode};
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
