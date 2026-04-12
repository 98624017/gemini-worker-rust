use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
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
fn maybe_sanitize_json_for_log_skips_when_admin_is_disabled() {
    let sanitized = rust_sync_proxy::admin::maybe_sanitize_json_for_log(
        br#"{"inlineData":{"data":"AQID"}}"#,
        false,
    );
    assert!(sanitized.is_none());
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

#[tokio::test]
async fn admin_stats_track_model_requests() {
    let admin = rust_sync_proxy::admin::AdminState::new("pw".to_string());
    rust_sync_proxy::admin::apply_admin_stats(
        admin.stats().as_ref(),
        &rust_sync_proxy::admin::AdminLogEntry {
            path: "/v1beta/models/demo:generateContent".to_string(),
            status_code: 200,
            duration_ms: 12,
            ..Default::default()
        },
    );

    let response = rust_sync_proxy::admin::admin_stats_response(
        &admin,
        rust_sync_proxy::blob_runtime::BlobRuntimeStatsSnapshot::default(),
    );
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["totalRequests"], 1);
    assert_eq!(json["errorRequests"], 0);
}

#[tokio::test]
async fn admin_stats_include_spill_metrics() {
    let admin = rust_sync_proxy::admin::AdminState::new("pw".to_string());
    let response = rust_sync_proxy::admin::admin_stats_response(
        &admin,
        rust_sync_proxy::blob_runtime::BlobRuntimeStatsSnapshot {
            spill_count: 3,
            spill_bytes_total: 99,
        },
    );

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["spillCount"], 3);
    assert_eq!(json["spillBytesTotal"], 99);
}

#[tokio::test]
async fn admin_logs_capture_structured_proxy_error_fields() {
    let mut config = rust_sync_proxy::test_config();
    config.admin_password = "pw".to_string();

    let app = rust_sync_proxy::build_router(config);
    let invalid_json_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"contents":[{"parts":[{"text":"hello"}]}]"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid_json_response.status(), StatusCode::BAD_GATEWAY);

    let auth = format!("Basic {}", STANDARD.encode("user:pw"));
    let logs_response = app
        .oneshot(
            Request::builder()
                .uri("/admin/api/logs")
                .header("authorization", auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(logs_response.status(), StatusCode::OK);
    let body = to_bytes(logs_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let item = &json["items"][0];
    assert_eq!(item["errorSource"], "proxy");
    assert_eq!(item["errorStage"], "parse_request_json");
    assert_eq!(item["errorKind"], "invalid_json");
    assert_eq!(item["errorMessage"], "invalid request json body");
}

#[tokio::test]
async fn admin_logs_capture_upstream_error_fields() {
    let server = Router::new().route(
        "/v1beta/models/demo:generateContent",
        post(mock_upstream_rate_limited),
    );
    let server_addr = spawn_admin_test_server(server).await;

    let mut config = rust_sync_proxy::test_config();
    config.admin_password = "pw".to_string();
    config.upstream_base_url = format!("http://{server_addr}");

    let app = rust_sync_proxy::build_router(config);
    let upstream_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent")
                .header("content-type", "application/json")
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
    assert_eq!(upstream_response.status(), StatusCode::TOO_MANY_REQUESTS);

    let auth = format!("Basic {}", STANDARD.encode("user:pw"));
    let logs_response = app
        .oneshot(
            Request::builder()
                .uri("/admin/api/logs")
                .header("authorization", auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(logs_response.status(), StatusCode::OK);
    let body = to_bytes(logs_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let item = &json["items"][0];
    assert_eq!(item["errorSource"], "upstream");
    assert_eq!(item["errorStage"], "upstream_response");
    assert_eq!(item["errorKind"], "upstream_error");
    assert_eq!(item["errorMessage"], "rate limited");
    assert_eq!(item["upstreamStatusCode"], 429);
    assert!(
        item["upstreamErrorBody"]
            .as_str()
            .unwrap_or_default()
            .contains("rate limited")
    );
}

#[tokio::test]
async fn admin_logs_capture_structured_upstream_connect_failures() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let mut config = rust_sync_proxy::test_config();
    config.admin_password = "pw".to_string();
    config.upstream_base_url = format!("http://{addr}");

    let app = rust_sync_proxy::build_router(config);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent")
                .header("content-type", "application/json")
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
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["source"], "proxy");
    assert_eq!(json["error"]["stage"], "send_upstream_request");
    assert_eq!(json["error"]["kind"], "upstream_connect_failed");

    let auth = format!("Basic {}", STANDARD.encode("user:pw"));
    let logs_response = app
        .oneshot(
            Request::builder()
                .uri("/admin/api/logs")
                .header("authorization", auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(logs_response.status(), StatusCode::OK);
    let body = to_bytes(logs_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let item = &json["items"][0];
    assert_eq!(item["errorSource"], "proxy");
    assert_eq!(item["errorStage"], "send_upstream_request");
    assert_eq!(item["errorKind"], "upstream_connect_failed");
    assert_eq!(item["errorMessage"], "failed to connect to upstream");
    let error_detail = item["errorDetail"].as_str().unwrap_or_default();
    assert!(!error_detail.is_empty());
    assert!(error_detail.contains(&addr.to_string()));
}

#[tokio::test]
async fn admin_logs_page_contains_chartjs_and_theme_toggle() {
    let html = fetch_admin_logs_page_html().await;

    // Verify Chart.js is inlined
    assert!(
        html.contains("Chart"),
        "HTML should contain inlined Chart.js"
    );

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

#[tokio::test]
async fn admin_logs_page_contains_error_detail_section() {
    let html = fetch_admin_logs_page_html().await;
    assert!(
        html.contains("error detail"),
        "HTML should contain error detail section"
    );
    assert!(
        html.contains("upstream status"),
        "HTML should contain upstream status label"
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

async fn mock_upstream_rate_limited() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(json!({
            "error": {
                "message": "rate limited"
            }
        })),
    )
}

async fn spawn_admin_test_server(app: Router) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    address
}
