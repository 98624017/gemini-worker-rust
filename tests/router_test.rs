use axum::body::Body;
use http::{Method, Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn generate_content_route_accepts_post_only() {
    let app = rust_sync_proxy::build_router(rust_sync_proxy::test_config());
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1beta/models/demo:generateContent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
}
