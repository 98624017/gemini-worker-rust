use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use tower::ServiceExt;

#[tokio::test]
async fn admin_logs_page_locks_task1_initial_shell_contracts() {
    let html = fetch_admin_logs_page_html().await;

    assert!(
        html.contains("<section class=\"charts-section collapsed\" id=\"chartsSection\">"),
        "HTML should default charts section to collapsed"
    );
    assert!(
        html.contains("id=\"chartsToggle\" type=\"button\" aria-expanded=\"false\""),
        "HTML should default charts toggle aria-expanded to false"
    );
    assert!(
        html.contains("id=\"chartsPanel\" aria-hidden=\"true\""),
        "HTML should mark charts panel hidden by default"
    );
    assert!(
        html.contains("id=\"viewModeTabs\" role=\"group\" aria-label=\"content view mode\""),
        "HTML should expose view mode as button group semantics"
    );
    assert!(
        html.contains(
            "class=\"view-mode-tab active\" data-view=\"list\" aria-pressed=\"true\">列表视图</button>"
        ),
        "HTML should default list view as active"
    );
    assert!(
        html.contains(
            "class=\"view-mode-tab\" data-view=\"album\" aria-pressed=\"false\">相册视图</button>"
        ),
        "HTML should contain album view switch button"
    );
}

async fn fetch_admin_logs_page_html() -> String {
    let mut config = rust_sync_proxy::test_config();
    config.admin_password = "pw".to_string();

    let app = rust_sync_proxy::build_router(config);
    let auth = format!("Basic {}", STANDARD.encode("user:pw"));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/admin/logs")
                .header("authorization", auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    String::from_utf8_lossy(&body).into_owned()
}
