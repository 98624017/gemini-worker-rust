use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::json;
use tokio::net::TcpListener;
use tower::ServiceExt;

#[test]
fn admin_log_omits_base64_payloads() {
    let sanitized =
        rust_sync_proxy::admin::sanitize_json_for_log(br#"{"inlineData":{"data":"QUJDREVGRw=="}}"#);
    assert!(sanitized.pretty.contains("[base64 omitted len=12]"));
}

#[test]
fn admin_log_collects_proxy_and_http_image_urls() {
    let sanitized = rust_sync_proxy::admin::sanitize_json_for_log(
        br#"{"parts":[{"inlineData":{"data":"https://img.example/a.png"}},{"inline_data":{"data":"/proxy/image?u=abc"}}]}"#,
    );
    assert_eq!(
        sanitized.image_urls,
        vec![
            "https://img.example/a.png".to_string(),
            "/proxy/image?u=abc".to_string()
        ]
    );
}

#[test]
fn extract_finish_reason_returns_first_candidate_reason() {
    let body: serde_json::Value = serde_json::from_str(
        r#"{"candidates":[{"finishReason":"STOP"},{"finishReason":"OTHER"}]}"#,
    )
    .unwrap();
    assert_eq!(
        rust_sync_proxy::admin::extract_finish_reason(&body).as_deref(),
        Some("STOP")
    );
}

#[tokio::test]
async fn admin_routes_require_basic_auth_and_return_logs() {
    let mut config = rust_sync_proxy::test_config();
    config.admin_password = "pw".to_string();

    let app = rust_sync_proxy::build_router(config.clone());
    let unauthorized = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/api/logs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let app = rust_sync_proxy::build_router(config);
    let auth = format!("Basic {}", STANDARD.encode("user:pw"));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/admin/api/logs")
                .header("authorization", auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["items"], serde_json::json!([]));
}

#[derive(Clone, Default)]
struct UpstreamCapture;

#[tokio::test]
async fn admin_stats_track_model_requests() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = listener.local_addr().unwrap();
    let upstream = Router::new()
        .route(
            "/v1beta/models/demo:generateContent",
            post(mock_generate_content),
        )
        .with_state(UpstreamCapture);
    tokio::spawn(async move {
        axum::serve(listener, upstream).await.unwrap();
    });

    let mut config = rust_sync_proxy::test_config();
    config.admin_password = "pw".to_string();
    config.upstream_base_url = format!("http://{}", upstream_addr);
    config.upstream_api_key = "env-key".to_string();
    let app = rust_sync_proxy::build_router(config);

    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"contents":[{"parts":[{"text":"hello"}]}]}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    let auth = format!("Basic {}", STANDARD.encode("user:pw"));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/admin/api/stats")
                .header("authorization", auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["totalRequests"], 1);
    assert_eq!(json["errorRequests"], 0);
}

async fn mock_generate_content(
    State(_capture): State<UpstreamCapture>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    assert!(!body.is_empty());
    Json(json!({
        "candidates": [{
            "finishReason": "STOP",
            "content": { "parts": [{ "text": "ok" }] }
        }]
    }))
}

#[tokio::test]
async fn admin_logs_page_contains_chartjs_and_theme_toggle() {
    let html = fetch_admin_logs_page_html().await;

    // Verify Chart.js is inlined
    assert!(html.contains("Chart"), "HTML should contain inlined Chart.js");

    // Verify theme toggle exists
    assert!(
        html.contains("themeToggle"),
        "HTML should contain theme toggle button"
    );

    // Verify CSS variables are used
    assert!(
        html.contains("--bg-primary"),
        "HTML should use CSS variables"
    );

    // Verify keyboard navigation code exists
    assert!(
        html.contains("keydown"),
        "HTML should contain keyboard navigation"
    );
}

#[tokio::test]
async fn admin_logs_page_only_previews_proxy_images() {
    let html = fetch_admin_logs_page_html().await;
    assert!(
        html.contains("startsWith('/proxy/image')"),
        "HTML should only auto-preview proxy image URLs"
    );
    assert!(
        html.contains("external image"),
        "HTML should keep external image URLs inert until explicit open"
    );
}

#[tokio::test]
async fn admin_logs_page_lazy_renders_log_details() {
    let html = fetch_admin_logs_page_html().await;
    assert!(
        html.contains("function buildDetailMarkup(item)"),
        "HTML should build detail markup lazily"
    );
    assert!(
        html.contains("if (!d.dataset.rendered)"),
        "HTML should defer detail rendering until expansion"
    );
}

#[tokio::test]
async fn admin_logs_page_preserves_system_theme_without_override() {
    let html = fetch_admin_logs_page_html().await;
    assert!(
        html.contains("function readThemeOverride()"),
        "HTML should keep override state separate from system theme"
    );
    assert!(
        html.contains("applyTheme(resolveTheme(), false);"),
        "initial theme application should not persist auto-detected theme"
    );
    assert!(
        html.contains("if (persist) localStorage.setItem('theme', theme);"),
        "theme persistence should happen only after explicit override"
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
