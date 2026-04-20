use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{error::Error as StdError, fmt};

use anyhow::{Result, anyhow};
use axum::body::{Body, to_bytes};
use axum::extract::{Path, Request, State};
use axum::http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use serde_json::{Value, json};
use tokio_util::io::ReaderStream;
use url::{Url, form_urlencoded};

use crate::admin::{self, AdminLogEntry, AdminState, SanitizedAdminLog};
use crate::blob_runtime::{BlobRuntime, BlobRuntimeConfig};
use crate::cache::InlineDataUrlFetchService;
use crate::config::Config;
use crate::request_encode::encode_request_body;
use crate::request_materialize::{
    RequestMaterializeServices, materialize_request_images_with_services,
};
use crate::request_rewrite::rewrite_aiapidev_request_body;
use crate::response_materialize::{finalize_output_urls, optimize_inline_data_images};
use crate::response_rewrite::{
    OutputMode, keep_largest_inline_image, normalize_aiapidev_task_response,
    normalize_special_markdown_image_response, remove_thought_signatures,
};
use crate::upload::{Uploader, wrap_external_proxy_url};
use crate::upstream::{
    ResolvedUpstream, is_aiapidev_base_url, resolve_upstream_for_request_from_header_map,
    rewrite_aiapidev_model_path,
};

const MAX_REQUEST_BODY_BYTES: usize = 20 * 1024 * 1024;
const AIAPIDEV_POLL_INTERVAL: Duration = Duration::from_secs(1);
const AIAPIDEV_MAX_POLL_TIME: Duration = Duration::from_secs(450);
const AIAPIDEV_MAX_CONSECUTIVE_POLL_FAILURES: usize = 5;

#[cfg_attr(not(test), allow(dead_code))]
fn proxy_error_json(code: u16, message: &str, source: &str, stage: &str, kind: &str) -> Value {
    json!({
        "error": {
            "code": code,
            "message": message,
            "source": source,
            "stage": stage,
            "kind": kind
        }
    })
}

fn proxy_error_response(status: StatusCode, message: &str, stage: &str, kind: &str) -> Response {
    (
        status,
        Json(proxy_error_json(
            status.as_u16(),
            message,
            "proxy",
            stage,
            kind,
        )),
    )
        .into_response()
}

#[derive(Debug)]
struct StructuredProxyError {
    message: &'static str,
    stage: &'static str,
    kind: &'static str,
    detail: String,
}

impl StructuredProxyError {
    fn new(
        message: &'static str,
        stage: &'static str,
        kind: &'static str,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            message,
            stage,
            kind,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for StructuredProxyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.detail)
    }
}

impl StdError for StructuredProxyError {}

fn classify_standard_proxy_error_detail(err: &anyhow::Error) -> Option<StructuredProxyError> {
    if let Some(structured) = err.downcast_ref::<StructuredProxyError>() {
        return Some(StructuredProxyError::new(
            structured.message,
            structured.stage,
            structured.kind,
            structured.detail.clone(),
        ));
    }

    let reqwest_error = err.downcast_ref::<reqwest::Error>()?;
    let (message, kind) = if reqwest_error.is_timeout() {
        ("upstream request timed out", "upstream_timeout")
    } else if reqwest_error.is_connect() {
        ("failed to connect to upstream", "upstream_connect_failed")
    } else if reqwest_error.is_body() {
        (
            "failed while sending upstream request body",
            "upstream_request_body_failed",
        )
    } else if reqwest_error.is_request() {
        ("failed to send upstream request", "upstream_request_failed")
    } else {
        ("upstream transport error", "upstream_transport_error")
    };

    Some(StructuredProxyError::new(
        message,
        "send_upstream_request",
        kind,
        reqwest_error.to_string(),
    ))
}

fn classify_standard_proxy_error(err: &anyhow::Error) -> Option<Value> {
    classify_standard_proxy_error_detail(err).map(|structured| {
        proxy_error_json(
            502,
            structured.message,
            "proxy",
            structured.stage,
            structured.kind,
        )
    })
}

fn apply_structured_proxy_error(entry: &mut AdminLogEntry, structured: &StructuredProxyError) {
    entry.error_source = "proxy".to_string();
    entry.error_stage = structured.stage.to_string();
    entry.error_kind = structured.kind.to_string();
    entry.error_message = structured.message.to_string();
    entry.error_detail = structured.detail.clone();
}

fn build_structured_proxy_error_response(structured: &StructuredProxyError) -> Response {
    proxy_error_response(
        StatusCode::BAD_GATEWAY,
        structured.message,
        structured.stage,
        structured.kind,
    )
}

struct RequestCacheTracking {
    observer: Option<Arc<dyn Fn(&str, bool) + Send + Sync>>,
    hit_urls: Arc<std::sync::Mutex<Vec<String>>>,
}

#[derive(Clone, Debug, Default)]
struct RequestLogSnapshot {
    request_raw: Option<SanitizedAdminLog>,
}

impl RequestLogSnapshot {
    fn from_request_body(raw: &[u8], enabled: bool) -> Self {
        Self {
            request_raw: sanitize_request_body_for_log(raw, enabled),
        }
    }

    fn apply_to_entry(&self, entry: &mut AdminLogEntry) {
        if let Some(value) = &self.request_raw {
            entry.request_raw = value.pretty.clone();
            entry.request_raw_images = value.image_urls.clone();
        }
    }

    fn base_entry(&self) -> AdminLogEntry {
        let mut entry = AdminLogEntry::default();
        self.apply_to_entry(&mut entry);
        entry
    }
}

#[cfg(test)]
static REQUEST_LOG_SANITIZE_CALLS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

fn sanitize_request_body_for_log(raw: &[u8], enabled: bool) -> Option<SanitizedAdminLog> {
    #[cfg(test)]
    REQUEST_LOG_SANITIZE_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    admin::maybe_sanitize_json_for_log(raw, enabled)
}

#[derive(Debug)]
struct ForwardRequestFailure {
    error: anyhow::Error,
    admin_entry: AdminLogEntry,
}

impl ForwardRequestFailure {
    fn new(error: impl Into<anyhow::Error>, admin_entry: AdminLogEntry) -> Self {
        Self {
            error: error.into(),
            admin_entry,
        }
    }
}

fn build_request_cache_tracking(
    admin_stats: Option<Arc<crate::admin::AdminStats>>,
) -> RequestCacheTracking {
    let hit_urls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observer = admin_stats.map(|stats| {
        let hit_urls = Arc::clone(&hit_urls);
        Arc::new(move |raw_url: &str, from_cache: bool| {
            if from_cache {
                stats
                    .cache_hits
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                hit_urls.lock().unwrap().push(raw_url.to_string());
            }
        }) as Arc<dyn Fn(&str, bool) + Send + Sync>
    });

    RequestCacheTracking { observer, hit_urls }
}

fn update_request_cache_hits(entry: &mut AdminLogEntry, tracking: &RequestCacheTracking) {
    entry.request_raw_image_cache_hits = tracking.hit_urls.lock().unwrap().clone();
}

fn build_upstream_client(config: &Config) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(config.upstream_timeout)
        .connect_timeout(config.upstream_connect_timeout)
        .tcp_keepalive(config.upstream_tcp_keepalive)
        .pool_idle_timeout(config.upstream_pool_idle_timeout)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

fn apply_admin_error_fields(
    entry: &mut AdminLogEntry,
    status: StatusCode,
    response_json: &Value,
    sanitized_response: &str,
) {
    let Some(error) = response_json.get("error").and_then(Value::as_object) else {
        return;
    };

    entry.error_source = error
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    entry.error_stage = error
        .get("stage")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    entry.error_kind = error
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    entry.error_message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    if entry.error_source == "upstream" {
        entry.upstream_status_code = status.as_u16();
        entry.upstream_error_body = sanitized_response.to_string();
    }
}

fn annotate_upstream_error_json(status: StatusCode, body_bytes: &[u8]) -> Option<Vec<u8>> {
    let mut body: Value = serde_json::from_slice(body_bytes).ok()?;
    let error = body.get_mut("error")?.as_object_mut()?;

    error
        .entry("code".to_string())
        .or_insert_with(|| json!(status.as_u16()));
    error
        .entry("source".to_string())
        .or_insert_with(|| json!("upstream"));
    error
        .entry("stage".to_string())
        .or_insert_with(|| json!("upstream_response"));
    error
        .entry("kind".to_string())
        .or_insert_with(|| json!("upstream_error"));

    serde_json::to_vec(&body).ok()
}

#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
    upstream_client: reqwest::Client,
    image_client: reqwest::Client,
    uploader: Arc<Uploader>,
    admin: Option<Arc<AdminState>>,
    request_inline_data_fetch_service: Option<Arc<InlineDataUrlFetchService>>,
    response_inline_data_fetch_service: Option<Arc<InlineDataUrlFetchService>>,
    blob_runtime: Arc<BlobRuntime>,
}

#[derive(Clone, Copy, Debug, Default)]
struct ResponseStageDurations {
    response_process_ms: i64,
    upload_ms: i64,
}

pub fn build_router(config: Config) -> Router {
    let admin = if config.admin_password.trim().is_empty() {
        None
    } else {
        Some(Arc::new(AdminState::new(config.admin_password.clone())))
    };
    let image_client = build_http_client(
        config.image_fetch_timeout,
        config.image_tls_handshake_timeout,
        config.image_fetch_insecure_skip_verify,
    );
    let upload_client = build_http_client(
        config.upload_timeout,
        config.upload_tls_handshake_timeout,
        config.upload_insecure_skip_verify,
    );
    let request_inline_data_fetch_service = InlineDataUrlFetchService::from_config(
        &config,
        image_client.clone(),
        crate::image_io::REQUEST_MAX_IMAGE_BYTES,
        false,
    );
    let response_inline_data_fetch_service = InlineDataUrlFetchService::from_response_config(
        &config,
        image_client.clone(),
        crate::image_io::DEFAULT_MAX_IMAGE_BYTES,
        false,
    );
    let upstream_client = build_upstream_client(&config);
    let blob_runtime = Arc::new(BlobRuntime::new(BlobRuntimeConfig {
        inline_max_bytes: config.blob_inline_max_bytes,
        request_hot_budget_bytes: config.blob_request_hot_budget_bytes,
        global_hot_budget_bytes: config.blob_global_hot_budget_bytes,
        spill_dir: config.blob_spill_dir.clone().into(),
    }));
    let state = AppState {
        uploader: Arc::new(Uploader::new(upload_client, config.clone())),
        config: Arc::new(config),
        upstream_client,
        image_client,
        admin,
        request_inline_data_fetch_service,
        response_inline_data_fetch_service,
        blob_runtime,
    };

    Router::new()
        .route("/admin", get(admin_root))
        .route("/admin/", get(admin_root))
        .route("/admin/logs", get(admin_logs_page))
        .route("/admin/api/logs", get(admin_logs_api))
        .route("/admin/api/stats", get(admin_stats_api))
        .route("/v1/images/generations", post(image_generations_action))
        .route("/v1beta/models/{*rest}", post(model_action))
        .with_state(state)
}

async fn image_generations_action(State(state): State<AppState>, request: Request) -> Response {
    let started_at = Instant::now();
    let created_at = admin::now_rfc3339();
    let request_method = request.method().to_string();
    let request_path = request.uri().path().to_string();
    let request_query = request.uri().query().unwrap_or_default().to_string();
    let remote_addr = request
        .headers()
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();

    let request_parse_started = Instant::now();
    let (parts, body) = request.into_parts();
    let request_body = match to_bytes(body, MAX_REQUEST_BODY_BYTES).await {
        Ok(body) => body,
        Err(err) => {
            let response = (
                StatusCode::BAD_GATEWAY,
                Json(proxy_error_json(
                    502,
                    "failed to read request body",
                    "proxy",
                    "read_request_body",
                    "request_body_read_failed",
                )),
            )
                .into_response();
            return finalize_admin_response(
                &state,
                response,
                AdminLogEntry {
                    created_at,
                    method: request_method,
                    path: request_path,
                    query: request_query,
                    remote_addr,
                    is_stream: false,
                    status_code: StatusCode::BAD_GATEWAY.as_u16(),
                    duration_ms: started_at.elapsed().as_millis() as i64,
                    request_parse_ms: request_parse_started.elapsed().as_millis() as i64,
                    error_source: "proxy".to_string(),
                    error_stage: "read_request_body".to_string(),
                    error_kind: "request_body_read_failed".to_string(),
                    error_message: "failed to read request body".to_string(),
                    error_detail: err.to_string(),
                    ..Default::default()
                },
            )
            .await;
        }
    };
    let request_log = RequestLogSnapshot::from_request_body(&request_body, state.admin.is_some());

    let parsed_body: Value = match serde_json::from_slice(&request_body) {
        Ok(body) => body,
        Err(err) => {
            let mut admin_entry = AdminLogEntry {
                created_at,
                method: request_method,
                path: request_path,
                query: request_query,
                remote_addr,
                is_stream: false,
                status_code: StatusCode::BAD_GATEWAY.as_u16(),
                duration_ms: started_at.elapsed().as_millis() as i64,
                request_parse_ms: request_parse_started.elapsed().as_millis() as i64,
                error_source: "proxy".to_string(),
                error_stage: "parse_request_json".to_string(),
                error_kind: "invalid_json".to_string(),
                error_message: "invalid request json body".to_string(),
                error_detail: format!("invalid request json body: {err}"),
                ..Default::default()
            };
            request_log.apply_to_entry(&mut admin_entry);
            let response = (
                StatusCode::BAD_GATEWAY,
                Json(proxy_error_json(
                    502,
                    "invalid request json body",
                    "proxy",
                    "parse_request_json",
                    "invalid_json",
                )),
            )
                .into_response();
            return finalize_admin_response(&state, response, admin_entry).await;
        }
    };

    let normalized_body = match crate::openai_image::normalize_request_body(parsed_body) {
        Ok(body) => body,
        Err(err) => {
            let mut admin_entry = AdminLogEntry {
                created_at,
                method: request_method,
                path: request_path,
                query: request_query,
                remote_addr,
                is_stream: false,
                status_code: StatusCode::BAD_REQUEST.as_u16(),
                duration_ms: started_at.elapsed().as_millis() as i64,
                request_parse_ms: request_parse_started.elapsed().as_millis() as i64,
                error_source: "proxy".to_string(),
                error_stage: "normalize_openai_image_request".to_string(),
                error_kind: "invalid_request".to_string(),
                error_message: "invalid openai image request".to_string(),
                error_detail: err.to_string(),
                ..Default::default()
            };
            request_log.apply_to_entry(&mut admin_entry);
            let response = (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": {"code": 400, "message": err.to_string()}})),
            )
                .into_response();
            return finalize_admin_response(&state, response, admin_entry).await;
        }
    };

    let resolved = match resolve_upstream_for_request_from_header_map(
        &parts.headers,
        &normalized_body,
        &state.config.upstream_base_url,
        &state.config.upstream_api_key,
    ) {
        Ok(resolved) => resolved,
        Err(err) => {
            let status = err.status_code();
            let mut admin_entry = AdminLogEntry {
                created_at,
                method: request_method,
                path: request_path,
                query: request_query,
                remote_addr,
                is_stream: false,
                status_code: status.as_u16(),
                duration_ms: started_at.elapsed().as_millis() as i64,
                request_parse_ms: request_parse_started.elapsed().as_millis() as i64,
                ..Default::default()
            };
            request_log.apply_to_entry(&mut admin_entry);
            let response = (
                status,
                Json(json!({"error": {"code": status.as_u16(), "message": err.to_string()}})),
            )
                .into_response();
            return finalize_admin_response(&state, response, admin_entry).await;
        }
    };
    let request_parse_ms = request_parse_started.elapsed().as_millis() as i64;

    match forward_openai_image_request(
        state.clone(),
        resolved,
        normalized_body,
        if request_query.is_empty() {
            None
        } else {
            Some(request_query.clone())
        },
        request_log.clone(),
    )
    .await
    {
        Ok((response, mut admin_entry)) => {
            admin_entry.created_at = created_at;
            admin_entry.method = request_method;
            admin_entry.path = request_path;
            admin_entry.query = request_query;
            admin_entry.remote_addr = remote_addr;
            admin_entry.is_stream = false;
            admin_entry.request_parse_ms += request_parse_ms;
            admin_entry.duration_ms = started_at.elapsed().as_millis() as i64;
            finalize_admin_response(&state, response, admin_entry).await
        }
        Err(failure) => {
            let mut admin_entry = failure.admin_entry;
            admin_entry.created_at = created_at;
            admin_entry.method = request_method;
            admin_entry.path = request_path;
            admin_entry.query = request_query;
            admin_entry.remote_addr = remote_addr;
            admin_entry.is_stream = false;
            admin_entry.status_code = StatusCode::BAD_GATEWAY.as_u16();
            admin_entry.request_parse_ms += request_parse_ms;
            admin_entry.duration_ms = started_at.elapsed().as_millis() as i64;
            let response = if let Some(structured) =
                classify_standard_proxy_error_detail(&failure.error)
            {
                apply_structured_proxy_error(&mut admin_entry, &structured);
                build_structured_proxy_error_response(&structured)
            } else if let Some(structured_error) = classify_standard_proxy_error(&failure.error) {
                (StatusCode::BAD_GATEWAY, Json(structured_error)).into_response()
            } else {
                (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({"error": {"code": 502, "message": failure.error.to_string()}})),
                )
                    .into_response()
            };
            finalize_admin_response(&state, response, admin_entry).await
        }
    }
}

async fn model_action(
    State(state): State<AppState>,
    Path(rest): Path<String>,
    request: Request,
) -> Response {
    let started_at = Instant::now();
    let created_at = admin::now_rfc3339();
    let request_method = request.method().to_string();
    let request_path = request.uri().path().to_string();
    let request_query = request.uri().query().unwrap_or_default().to_string();
    let remote_addr = request
        .headers()
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();

    if !rest.ends_with(":generateContent") {
        let response = (
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"code": 404, "message": "Not Found"}})),
        )
            .into_response();
        return finalize_admin_response(
            &state,
            response,
            AdminLogEntry {
                created_at,
                method: request_method,
                path: request_path,
                query: request_query,
                remote_addr,
                is_stream: false,
                status_code: StatusCode::NOT_FOUND.as_u16(),
                duration_ms: started_at.elapsed().as_millis() as i64,
                ..Default::default()
            },
        )
        .await;
    }

    let request_parse_started = Instant::now();
    let (parts, body) = request.into_parts();
    let request_body = match to_bytes(body, MAX_REQUEST_BODY_BYTES).await {
        Ok(body) => body,
        Err(err) => {
            let response = (
                StatusCode::BAD_GATEWAY,
                Json(proxy_error_json(
                    502,
                    "failed to read request body",
                    "proxy",
                    "read_request_body",
                    "request_body_read_failed",
                )),
            )
                .into_response();
            return finalize_admin_response(
                &state,
                response,
                AdminLogEntry {
                    created_at,
                    method: request_method,
                    path: request_path,
                    query: request_query,
                    remote_addr,
                    is_stream: false,
                    status_code: StatusCode::BAD_GATEWAY.as_u16(),
                    duration_ms: started_at.elapsed().as_millis() as i64,
                    request_parse_ms: request_parse_started.elapsed().as_millis() as i64,
                    error_source: "proxy".to_string(),
                    error_stage: "read_request_body".to_string(),
                    error_kind: "request_body_read_failed".to_string(),
                    error_message: "failed to read request body".to_string(),
                    error_detail: err.to_string(),
                    ..Default::default()
                },
            )
            .await;
        }
    };
    let request_log = RequestLogSnapshot::from_request_body(&request_body, state.admin.is_some());

    let parsed_body: Value = match serde_json::from_slice(&request_body) {
        Ok(body) => body,
        Err(err) => {
            let mut admin_entry = AdminLogEntry {
                created_at,
                method: request_method,
                path: request_path,
                query: request_query,
                remote_addr,
                is_stream: false,
                status_code: StatusCode::BAD_GATEWAY.as_u16(),
                duration_ms: started_at.elapsed().as_millis() as i64,
                request_parse_ms: request_parse_started.elapsed().as_millis() as i64,
                error_source: "proxy".to_string(),
                error_stage: "parse_request_json".to_string(),
                error_kind: "invalid_json".to_string(),
                error_message: "invalid request json body".to_string(),
                error_detail: format!("invalid request json body: {err}"),
                ..Default::default()
            };
            request_log.apply_to_entry(&mut admin_entry);
            let response = (
                StatusCode::BAD_GATEWAY,
                Json(proxy_error_json(
                    502,
                    "invalid request json body",
                    "proxy",
                    "parse_request_json",
                    "invalid_json",
                )),
            )
                .into_response();
            return finalize_admin_response(&state, response, admin_entry).await;
        }
    };

    let resolved = match resolve_upstream_for_request_from_header_map(
        &parts.headers,
        &parsed_body,
        &state.config.upstream_base_url,
        &state.config.upstream_api_key,
    ) {
        Ok(resolved) => resolved,
        Err(err) => {
            let status = err.status_code();
            let mut admin_entry = AdminLogEntry {
                created_at,
                method: request_method,
                path: request_path,
                query: request_query,
                remote_addr,
                is_stream: false,
                status_code: status.as_u16(),
                duration_ms: started_at.elapsed().as_millis() as i64,
                request_parse_ms: request_parse_started.elapsed().as_millis() as i64,
                ..Default::default()
            };
            request_log.apply_to_entry(&mut admin_entry);
            let response = (
                status,
                Json(json!({"error": {"code": status.as_u16(), "message": err.to_string()}})),
            )
                .into_response();
            return finalize_admin_response(&state, response, admin_entry).await;
        }
    };
    let request_parse_ms = request_parse_started.elapsed().as_millis() as i64;

    let target_path = format!("/v1beta/models/{rest}");
    let is_aiapidev_upstream = is_aiapidev_base_url(&resolved.base_url);
    let request = Request::from_parts(parts, Body::from(request_body));

    match forward_gemini_request(
        state.clone(),
        resolved,
        target_path,
        request,
        request_log.clone(),
    )
    .await
    {
        Ok((response, mut admin_entry)) => {
            admin_entry.created_at = created_at;
            admin_entry.method = request_method;
            admin_entry.path = request_path;
            admin_entry.query = request_query;
            admin_entry.remote_addr = remote_addr;
            admin_entry.is_stream = false;
            admin_entry.request_parse_ms += request_parse_ms;
            admin_entry.duration_ms = started_at.elapsed().as_millis() as i64;
            finalize_admin_response(&state, response, admin_entry).await
        }
        Err(failure) => {
            let mut admin_entry = failure.admin_entry;
            admin_entry.created_at = created_at;
            admin_entry.method = request_method;
            admin_entry.path = request_path;
            admin_entry.query = request_query;
            admin_entry.remote_addr = remote_addr;
            admin_entry.is_stream = false;
            admin_entry.status_code = StatusCode::BAD_GATEWAY.as_u16();
            admin_entry.request_parse_ms += request_parse_ms;
            admin_entry.duration_ms = started_at.elapsed().as_millis() as i64;
            let response = if !is_aiapidev_upstream {
                if let Some(structured) = classify_standard_proxy_error_detail(&failure.error) {
                    apply_structured_proxy_error(&mut admin_entry, &structured);
                    build_structured_proxy_error_response(&structured)
                } else if let Some(structured_error) = classify_standard_proxy_error(&failure.error)
                {
                    (StatusCode::BAD_GATEWAY, Json(structured_error)).into_response()
                } else {
                    (
                        StatusCode::BAD_GATEWAY,
                        Json(json!({"error": {"code": 502, "message": failure.error.to_string()}})),
                    )
                        .into_response()
                }
            } else {
                (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({"error": {"code": 502, "message": failure.error.to_string()}})),
                )
                    .into_response()
            };
            finalize_admin_response(&state, response, admin_entry).await
        }
    }
}

async fn forward_gemini_request(
    state: AppState,
    resolved: ResolvedUpstream,
    target_path: String,
    request: Request,
    request_log: RequestLogSnapshot,
) -> Result<(Response, AdminLogEntry), ForwardRequestFailure> {
    let request_parse_started = Instant::now();
    let content_type_header = request.headers().get(CONTENT_TYPE).cloned();
    let accept_header = request.headers().get(ACCEPT).cloned();
    let request_query = request.uri().query().map(ToOwned::to_owned);
    let admin_enabled = state.admin.is_some();
    let request_body = to_bytes(request.into_body(), MAX_REQUEST_BODY_BYTES)
        .await
        .map_err(|err| {
            ForwardRequestFailure::new(
                anyhow!("failed to read request body: {err}"),
                request_log.base_entry(),
            )
        })?;
    let mut admin_entry = request_log.base_entry();
    let body: Value = match serde_json::from_slice(&request_body) {
        Ok(body) => body,
        Err(err) => {
            admin_entry.request_parse_ms = request_parse_started.elapsed().as_millis() as i64;
            return Err(ForwardRequestFailure::new(
                StructuredProxyError::new(
                    "invalid request json body",
                    "parse_request_json",
                    "invalid_json",
                    format!("invalid request json body: {err}"),
                ),
                admin_entry,
            ));
        }
    };
    let output_mode = get_output_mode(request_query.as_deref(), &body);
    admin_entry.output_mode = match output_mode {
        OutputMode::Base64 => "base64".to_string(),
        OutputMode::Url => "url".to_string(),
    };
    let is_aiapidev = is_aiapidev_base_url(&resolved.base_url);
    let request_parse_ms = request_parse_started.elapsed().as_millis() as i64;
    admin_entry.request_parse_ms = request_parse_ms;

    let admin_stats = state.admin.as_ref().map(|admin| admin.stats());
    let cache_tracking = build_request_cache_tracking(admin_stats);

    if is_aiapidev {
        let external_proxy_prefix = state.config.resolved_external_image_proxy_prefix();
        let rewritten_body = rewrite_aiapidev_request_body(
            body,
            &external_proxy_prefix,
            &state.config.image_fetch_external_proxy_domains,
        );
        let target_path = rewrite_aiapidev_model_path(&target_path);
        let request_upstream = if admin_enabled {
            let request_upstream_bytes = serde_json::to_vec(&rewritten_body)
                .map_err(|err| ForwardRequestFailure::new(err, admin_entry.clone()))?;
            admin::maybe_sanitize_json_for_log(&request_upstream_bytes, true)
        } else {
            None
        };
        let upstream_build_started = Instant::now();
        let response = handle_aiapidev_response(
            &resolved,
            &target_path,
            request_query.as_deref(),
            rewritten_body,
            output_mode,
            &state.upstream_client,
            &state.image_client,
            state.response_inline_data_fetch_service.as_ref(),
            state.config.as_ref(),
        )
        .await;
        let upstream_build_ms = upstream_build_started.elapsed().as_millis() as i64;

        admin_entry.upstream_build_ms = upstream_build_ms;
        admin_entry.request_upstream = request_upstream
            .as_ref()
            .map(|value| value.pretty.clone())
            .unwrap_or_default();
        admin_entry.request_upstream_images = request_upstream
            .as_ref()
            .map(|value| value.image_urls.clone())
            .unwrap_or_default();
        admin_entry.status_code = response.status().as_u16();
        return Ok((response, admin_entry));
    }

    let request_image_prepare_started = Instant::now();
    let request_image_materialize_started = Instant::now();
    let materialized = match materialize_request_images_with_services(
        body,
        state.blob_runtime.as_ref(),
        &RequestMaterializeServices {
            image_client: state.image_client.clone(),
            max_image_bytes: crate::image_io::REQUEST_MAX_IMAGE_BYTES,
            allow_private_networks: false,
            enable_webp_optimization: state.config.enable_request_image_webp_optimization,
            fetch_service: state.request_inline_data_fetch_service.clone(),
            cache_observer: cache_tracking.observer.clone(),
        },
    )
    .await
    {
        Ok(materialized) => materialized,
        Err(err) => {
            admin_entry.request_image_prepare_ms =
                request_image_prepare_started.elapsed().as_millis() as i64;
            update_request_cache_hits(&mut admin_entry, &cache_tracking);
            return Err(ForwardRequestFailure::new(err, admin_entry));
        }
    };
    let request_image_materialize_ms =
        request_image_materialize_started.elapsed().as_millis() as i64;
    let request_encode_started = Instant::now();
    let encoded = match encode_request_body(
        materialized.request,
        materialized.replacements.clone(),
        state.blob_runtime.as_ref(),
    )
    .await
    {
        Ok(encoded) => encoded,
        Err(err) => {
            admin_entry.request_image_materialize_ms = request_image_materialize_ms;
            admin_entry.request_image_fetch_work_ms = materialized.fetch_work_ms;
            admin_entry.request_image_store_work_ms = materialized.store_work_ms;
            admin_entry.request_image_prepare_ms =
                request_image_prepare_started.elapsed().as_millis() as i64;
            update_request_cache_hits(&mut admin_entry, &cache_tracking);
            return Err(ForwardRequestFailure::new(err, admin_entry));
        }
    };
    let request_encode_ms = request_encode_started.elapsed().as_millis() as i64;
    let request_image_prepare_ms = request_image_prepare_started.elapsed().as_millis() as i64;
    admin_entry.request_image_prepare_ms = request_image_prepare_ms;
    admin_entry.request_image_materialize_ms = request_image_materialize_ms;
    admin_entry.request_image_fetch_work_ms = materialized.fetch_work_ms;
    admin_entry.request_image_store_work_ms = materialized.store_work_ms;
    admin_entry.request_encode_ms = request_encode_ms;

    let upstream_build_started = Instant::now();
    for replacement in &materialized.replacements {
        if let Err(err) = state.blob_runtime.remove(&replacement.blob).await {
            update_request_cache_hits(&mut admin_entry, &cache_tracking);
            return Err(ForwardRequestFailure::new(err, admin_entry));
        }
    }

    let request_upstream = if admin_enabled {
        let request_upstream_bytes = state
            .blob_runtime
            .read_bytes(&encoded.body_blob)
            .await
            .map_err(|err| {
                update_request_cache_hits(&mut admin_entry, &cache_tracking);
                ForwardRequestFailure::new(err, admin_entry.clone())
            })?;
        admin::maybe_sanitize_json_for_log(&request_upstream_bytes, true)
    } else {
        None
    };
    let upstream_url =
        build_upstream_url(&resolved.base_url, &target_path, request_query.as_deref()).map_err(
            |err| {
                update_request_cache_hits(&mut admin_entry, &cache_tracking);
                ForwardRequestFailure::new(err, admin_entry.clone())
            },
        )?;

    let reader = state
        .blob_runtime
        .open_reader(&encoded.body_blob)
        .await
        .map_err(|err| {
            update_request_cache_hits(&mut admin_entry, &cache_tracking);
            ForwardRequestFailure::new(err, admin_entry.clone())
        })?;
    let request_stream = ReaderStream::new(reader);
    let mut upstream_request = state
        .upstream_client
        .post(upstream_url)
        .body(reqwest::Body::wrap_stream(request_stream))
        .header("content-length", encoded.content_length.to_string());
    if let Some(value) = content_type_header {
        upstream_request = upstream_request.header(CONTENT_TYPE, value);
    }
    if let Some(value) = accept_header {
        upstream_request = upstream_request.header(ACCEPT, value);
    }
    upstream_request = upstream_request.header("x-goog-api-key", resolved.api_key.clone());
    upstream_request =
        upstream_request.header(AUTHORIZATION, format!("Bearer {}", resolved.api_key));

    let upstream_response = match upstream_request.send().await {
        Ok(response) => response,
        Err(err) => {
            if let Err(remove_err) = state.blob_runtime.remove(&encoded.body_blob).await {
                update_request_cache_hits(&mut admin_entry, &cache_tracking);
                return Err(ForwardRequestFailure::new(remove_err, admin_entry));
            }
            update_request_cache_hits(&mut admin_entry, &cache_tracking);
            return Err(ForwardRequestFailure::new(err, admin_entry));
        }
    };
    state
        .blob_runtime
        .remove(&encoded.body_blob)
        .await
        .map_err(|err| {
            update_request_cache_hits(&mut admin_entry, &cache_tracking);
            ForwardRequestFailure::new(err, admin_entry.clone())
        })?;
    let upstream_build_ms = upstream_build_started.elapsed().as_millis() as i64;
    admin_entry.upstream_build_ms = upstream_build_ms;
    update_request_cache_hits(&mut admin_entry, &cache_tracking);
    admin_entry.request_upstream = request_upstream
        .as_ref()
        .map(|value| value.pretty.clone())
        .unwrap_or_default();
    admin_entry.request_upstream_images = request_upstream
        .as_ref()
        .map(|value| value.image_urls.clone())
        .unwrap_or_default();

    let (response, response_durations) = match handle_non_stream_response(
        upstream_response,
        output_mode,
        &state.image_client,
        state.response_inline_data_fetch_service.as_ref(),
        state.uploader.as_ref(),
        state.blob_runtime.as_ref(),
        state.config.as_ref(),
    )
    .await
    {
        Ok(result) => result,
        Err(err) => return Err(ForwardRequestFailure::new(err, admin_entry)),
    };
    admin_entry.status_code = response.status().as_u16();
    admin_entry.response_process_ms = response_durations.response_process_ms;
    admin_entry.upload_ms = response_durations.upload_ms;
    Ok((response, admin_entry))
}

async fn forward_openai_image_request(
    state: AppState,
    resolved: ResolvedUpstream,
    request_body: Value,
    request_query: Option<String>,
    request_log: RequestLogSnapshot,
) -> Result<(Response, AdminLogEntry), ForwardRequestFailure> {
    let admin_enabled = state.admin.is_some();
    let mut admin_entry = request_log.base_entry();
    admin_entry.output_mode = "url".to_string();

    let request_upstream = if admin_enabled {
        let request_upstream_bytes = serde_json::to_vec(&request_body)
            .map_err(|err| ForwardRequestFailure::new(err, admin_entry.clone()))?;
        admin::maybe_sanitize_json_for_log(&request_upstream_bytes, true)
    } else {
        None
    };

    let upstream_build_started = Instant::now();
    let upstream_url = build_upstream_url(
        &resolved.base_url,
        "/v1/images/generations",
        request_query.as_deref(),
    )
    .map_err(|err| ForwardRequestFailure::new(err, admin_entry.clone()))?;
    let upstream_response = state
        .upstream_client
        .post(upstream_url)
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, format!("Bearer {}", resolved.api_key))
        .json(&request_body)
        .send()
        .await
        .map_err(|err| ForwardRequestFailure::new(err, admin_entry.clone()))?;

    admin_entry.upstream_build_ms = upstream_build_started.elapsed().as_millis() as i64;
    admin_entry.request_upstream = request_upstream
        .as_ref()
        .map(|value| value.pretty.clone())
        .unwrap_or_default();
    admin_entry.request_upstream_images = request_upstream
        .as_ref()
        .map(|value| value.image_urls.clone())
        .unwrap_or_default();

    let (response, response_durations) = handle_openai_image_response(
        upstream_response,
        state.uploader.as_ref(),
        state.config.as_ref(),
    )
    .await
    .map_err(|err| ForwardRequestFailure::new(err, admin_entry.clone()))?;
    admin_entry.status_code = response.status().as_u16();
    admin_entry.response_process_ms = response_durations.response_process_ms;
    admin_entry.upload_ms = response_durations.upload_ms;
    Ok((response, admin_entry))
}

async fn handle_non_stream_response(
    upstream_response: reqwest::Response,
    output_mode: OutputMode,
    image_client: &reqwest::Client,
    fetch_service: Option<&Arc<InlineDataUrlFetchService>>,
    uploader: &Uploader,
    blob_runtime: &BlobRuntime,
    config: &Config,
) -> Result<(Response, ResponseStageDurations)> {
    let response_started = Instant::now();
    let status = upstream_response.status();
    let content_type = upstream_response
        .headers()
        .get(CONTENT_TYPE)
        .cloned()
        .unwrap_or_else(|| HeaderValue::from_static("application/json"));
    let body_bytes = upstream_response.bytes().await.map_err(|err| {
        StructuredProxyError::new(
            "failed to read upstream response body",
            "read_upstream_body",
            "body_truncated",
            err.to_string(),
        )
    })?;

    if !status.is_success() {
        let response_body = annotate_upstream_error_json(status, &body_bytes)
            .map(Body::from)
            .unwrap_or_else(|| Body::from(body_bytes));
        let mut response = Response::new(response_body);
        *response.status_mut() = StatusCode::from_u16(status.as_u16())?;
        response.headers_mut().insert(CONTENT_TYPE, content_type);
        return Ok((
            response,
            ResponseStageDurations {
                response_process_ms: response_started.elapsed().as_millis() as i64,
                upload_ms: 0,
            },
        ));
    }

    let json_body: Value = match serde_json::from_slice(&body_bytes) {
        Ok(body) => body,
        Err(_) => {
            let mut response = Response::new(Body::from(body_bytes));
            *response.status_mut() = StatusCode::from_u16(status.as_u16())?;
            response.headers_mut().insert(CONTENT_TYPE, content_type);
            return Ok((
                response,
                ResponseStageDurations {
                    response_process_ms: response_started.elapsed().as_millis() as i64,
                    upload_ms: 0,
                },
            ));
        }
    };

    let mut final_json = normalize_special_markdown_image_response(
        json_body,
        output_mode,
        image_client,
        fetch_service,
        config,
    )
    .await?;
    remove_thought_signatures(&mut final_json);
    final_json = keep_largest_inline_image(final_json);
    optimize_inline_data_images(&mut final_json, config)?;
    let mut upload_ms = 0_i64;
    if output_mode == OutputMode::Url {
        let upload_started = Instant::now();
        finalize_output_urls(&mut final_json, blob_runtime, uploader, config).await?;
        upload_ms = upload_started.elapsed().as_millis() as i64;
    }
    let final_body = serde_json::to_vec(&final_json)?;
    let mut response = Response::new(Body::from(final_body));
    *response.status_mut() = StatusCode::from_u16(status.as_u16())?;
    response.headers_mut().insert(CONTENT_TYPE, content_type);
    let total_process_ms = response_started.elapsed().as_millis() as i64;
    Ok((
        response,
        ResponseStageDurations {
            response_process_ms: total_process_ms.saturating_sub(upload_ms),
            upload_ms,
        },
    ))
}

async fn handle_openai_image_response(
    upstream_response: reqwest::Response,
    uploader: &Uploader,
    config: &Config,
) -> Result<(Response, ResponseStageDurations)> {
    let response_started = Instant::now();
    let status = upstream_response.status();
    let content_type = upstream_response
        .headers()
        .get(CONTENT_TYPE)
        .cloned()
        .unwrap_or_else(|| HeaderValue::from_static("application/json"));
    let body_bytes = upstream_response.bytes().await.map_err(|err| {
        StructuredProxyError::new(
            "failed to read upstream response body",
            "read_upstream_body",
            "body_truncated",
            err.to_string(),
        )
    })?;

    if !status.is_success() {
        let response = raw_reqwest_response_with_body(status, content_type, body_bytes.to_vec());
        return Ok((
            response,
            ResponseStageDurations {
                response_process_ms: response_started.elapsed().as_millis() as i64,
                upload_ms: 0,
            },
        ));
    }

    let upstream_body: Value = serde_json::from_slice(&body_bytes).map_err(|err| {
        StructuredProxyError::new(
            "failed to parse upstream response json",
            "parse_upstream_response",
            "invalid_json",
            err.to_string(),
        )
    })?;
    let base64_images = extract_openai_b64_json_entries(&upstream_body)?;

    let upload_started = Instant::now();
    let mut uploaded = Vec::with_capacity(base64_images.len());
    for base64_image in base64_images {
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(base64_image.as_bytes())
            .map_err(|err| {
                StructuredProxyError::new(
                    "failed to decode upstream b64_json",
                    "decode_b64_json",
                    "invalid_base64",
                    err.to_string(),
                )
            })?;
        let mime_type = crate::image_io::sniff_image_mime_type(&decoded).unwrap_or("image/png");
        let upload_result = uploader
            .upload_inline_data_base64(Arc::from(base64_image.as_str()), mime_type)
            .await?;
        let final_url =
            build_openai_image_output_url(config, &upload_result.provider, &upload_result.url);
        uploaded.push(crate::openai_image::UploadedImage { url: final_url });
    }
    let upload_ms = upload_started.elapsed().as_millis() as i64;

    let fallback_created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let final_json =
        crate::openai_image::build_response_payload(upstream_body, &uploaded, fallback_created)?;
    let final_body = serde_json::to_vec(&final_json)?;
    let mut response = Response::new(Body::from(final_body));
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let total_process_ms = response_started.elapsed().as_millis() as i64;
    Ok((
        response,
        ResponseStageDurations {
            response_process_ms: total_process_ms.saturating_sub(upload_ms),
            upload_ms,
        },
    ))
}

async fn handle_aiapidev_response(
    resolved: &ResolvedUpstream,
    target_path: &str,
    request_query: Option<&str>,
    request_body: Value,
    output_mode: OutputMode,
    upstream_client: &reqwest::Client,
    image_client: &reqwest::Client,
    fetch_service: Option<&Arc<InlineDataUrlFetchService>>,
    config: &Config,
) -> Response {
    let upstream_url = match build_upstream_url(&resolved.base_url, target_path, request_query) {
        Ok(url) => url,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": {"code": 502, "message": err.to_string()}})),
            )
                .into_response();
        }
    };
    let create_response = upstream_client
        .post(upstream_url)
        .header(CONTENT_TYPE, "application/json")
        .header("x-goog-api-key", resolved.api_key.clone())
        .json(&request_body)
        .send()
        .await;

    let create_response = match create_response {
        Ok(response) => response,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": {"code": 502, "message": err.to_string()}})),
            )
                .into_response();
        }
    };

    if !create_response.status().is_success() {
        return raw_reqwest_response(create_response).await;
    }

    let created_task: Value = match create_response.json().await {
        Ok(body) => body,
        Err(_) => {
            return proxy_error_response(
                StatusCode::BAD_GATEWAY,
                "failed to parse aiapidev create response",
                "aiapidev_create_task",
                "invalid_json",
            );
        }
    };
    let request_id = created_task
        .get("requestId")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned);
    let Some(request_id) = request_id else {
        return (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": {"code": 502, "message": "aiapidev task create response missing requestId"}})),
        )
            .into_response();
    };

    let task_body = match poll_aiapidev_task(upstream_client, resolved, &request_id).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    let status = task_body
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();

    if status != "succeeded" {
        let message = task_body
            .get("errorMessage")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                task_body
                    .get("errorCode")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
            })
            .unwrap_or("aiapidev task failed");
        return (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": {"code": 502, "message": message}})),
        )
            .into_response();
    }

    let final_json = normalize_aiapidev_task_response(
        task_body,
        output_mode,
        image_client,
        fetch_service,
        config,
    )
    .await;
    let mut final_json = match final_json {
        Ok(body) => body,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": {"code": 502, "message": err.to_string()}})),
            )
                .into_response();
        }
    };
    remove_thought_signatures(&mut final_json);
    final_json = keep_largest_inline_image(final_json);
    if let Err(err) = optimize_inline_data_images(&mut final_json, config) {
        return (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": {"code": 502, "message": err.to_string()}})),
        )
            .into_response();
    }

    match serde_json::to_vec(&final_json) {
        Ok(final_body) => {
            let mut response = Response::new(Body::from(final_body));
            *response.status_mut() = StatusCode::OK;
            response
                .headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
            response
        }
        Err(err) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": {"code": 502, "message": err.to_string()}})),
        )
            .into_response(),
    }
}

async fn poll_aiapidev_task(
    upstream_client: &reqwest::Client,
    resolved: &ResolvedUpstream,
    request_id: &str,
) -> std::result::Result<Value, Response> {
    let deadline = Instant::now() + AIAPIDEV_MAX_POLL_TIME;
    let mut consecutive_failures = 0usize;
    loop {
        let task_path = format!("/v1beta/tasks/{request_id}");
        let task_url = match build_upstream_url(&resolved.base_url, &task_path, None) {
            Ok(url) => url,
            Err(err) => {
                return Err((
                    StatusCode::BAD_GATEWAY,
                    Json(json!({"error": {"code": 502, "message": err.to_string()}})),
                )
                    .into_response());
            }
        };
        let response = upstream_client
            .get(task_url)
            .header("x-goog-api-key", resolved.api_key.clone())
            .send()
            .await;

        let response = match response {
            Ok(response) => response,
            Err(err) => {
                consecutive_failures += 1;
                if consecutive_failures >= AIAPIDEV_MAX_CONSECUTIVE_POLL_FAILURES {
                    return Err((
                        StatusCode::BAD_GATEWAY,
                        Json(json!({"error": {"code": 502, "message": err.to_string()}})),
                    )
                        .into_response());
                }
                if Instant::now() >= deadline {
                    return Err(proxy_error_response(
                        StatusCode::BAD_GATEWAY,
                        "aiapidev task poll timed out",
                        "aiapidev_poll_task",
                        "timeout",
                    ));
                }
                tokio::time::sleep(AIAPIDEV_POLL_INTERVAL).await;
                continue;
            }
        };

        if !response.status().is_success() {
            if !is_aiapidev_retryable_poll_status(response.status()) {
                return Err(raw_reqwest_response(response).await);
            }
            consecutive_failures += 1;
            if consecutive_failures >= AIAPIDEV_MAX_CONSECUTIVE_POLL_FAILURES {
                return Err(raw_reqwest_response(response).await);
            }
            if Instant::now() >= deadline {
                return Err(proxy_error_response(
                    StatusCode::BAD_GATEWAY,
                    "aiapidev task poll timed out",
                    "aiapidev_poll_task",
                    "timeout",
                ));
            }
            tokio::time::sleep(AIAPIDEV_POLL_INTERVAL).await;
            continue;
        }
        consecutive_failures = 0;

        let task_body: Value = match response.json().await {
            Ok(body) => body,
            Err(_) => {
                return Err(proxy_error_response(
                    StatusCode::BAD_GATEWAY,
                    "failed to parse aiapidev poll response",
                    "aiapidev_parse_task_response",
                    "invalid_json",
                ));
            }
        };
        let status = task_body
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if is_aiapidev_terminal_status(&status) {
            return Ok(task_body);
        }
        if Instant::now() >= deadline {
            return Err(proxy_error_response(
                StatusCode::BAD_GATEWAY,
                "aiapidev task poll timed out",
                "aiapidev_poll_task",
                "timeout",
            ));
        }
        tokio::time::sleep(AIAPIDEV_POLL_INTERVAL).await;
    }
}

fn is_aiapidev_terminal_status(status: &str) -> bool {
    matches!(
        status,
        "succeeded" | "failed" | "error" | "cancelled" | "canceled" | "timeout"
    )
}

fn is_aiapidev_retryable_poll_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::REQUEST_TIMEOUT
            | StatusCode::TOO_EARLY
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

async fn raw_reqwest_response(upstream_response: reqwest::Response) -> Response {
    let status = upstream_response.status();
    let content_type = upstream_response
        .headers()
        .get(CONTENT_TYPE)
        .cloned()
        .unwrap_or_else(|| HeaderValue::from_static("application/json"));
    let body_bytes = match upstream_response.bytes().await {
        Ok(body) => body,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": {"code": 502, "message": err.to_string()}})),
            )
                .into_response();
        }
    };

    raw_reqwest_response_with_body(status, content_type, body_bytes.to_vec())
}

fn raw_reqwest_response_with_body(
    status: reqwest::StatusCode,
    content_type: HeaderValue,
    body_bytes: Vec<u8>,
) -> Response {
    let mut response = Response::new(Body::from(body_bytes));
    *response.status_mut() =
        StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    response.headers_mut().insert(CONTENT_TYPE, content_type);
    response
}

async fn admin_root(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(response) = authorize_admin(&state, &headers).await {
        return response;
    }
    let mut response = StatusCode::FOUND.into_response();
    response
        .headers_mut()
        .insert("location", "/admin/logs".parse().unwrap());
    response
}

async fn admin_logs_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    authorize_admin(&state, &headers)
        .await
        .unwrap_or_else(|| admin::admin_logs_page().into_response())
}

async fn admin_logs_api(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(response) = authorize_admin(&state, &headers).await {
        return response;
    }
    match state.admin.as_ref() {
        Some(admin) => admin::admin_logs_response(admin).await,
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn admin_stats_api(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(response) = authorize_admin(&state, &headers).await {
        return response;
    }
    match state.admin.as_ref() {
        Some(admin) => admin::admin_stats_response(admin, state.blob_runtime.stats_snapshot()),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn authorize_admin(state: &AppState, headers: &HeaderMap) -> Option<Response> {
    let Some(admin) = state.admin.as_ref() else {
        return Some(StatusCode::NOT_FOUND.into_response());
    };
    if !admin.check_basic_auth(headers) {
        return Some(admin.unauthorized_response());
    }
    None
}

async fn finalize_admin_response(
    state: &AppState,
    response: Response,
    mut entry: AdminLogEntry,
) -> Response {
    log_slow_request(state.config.as_ref(), &entry);

    let Some(admin_state) = state.admin.as_ref() else {
        return response;
    };

    let (parts, body) = response.into_parts();
    let body_bytes = to_bytes(body, usize::MAX).await.unwrap_or_default();

    if !body_bytes.is_empty() {
        let sanitized = admin::sanitize_json_for_log(&body_bytes);
        let response_downstream = sanitized.pretty;
        entry.response_images = sanitized.image_urls;
        entry.response_downstream = response_downstream.clone();
        if let Ok(value) = serde_json::from_slice::<Value>(&body_bytes) {
            entry.finish_reason = admin::extract_finish_reason(&value).unwrap_or_default();
            apply_admin_error_fields(&mut entry, parts.status, &value, &response_downstream);
        }
    }

    entry.status_code = parts.status.as_u16();

    let stats = admin_state.stats();
    admin::apply_admin_stats(stats.as_ref(), &entry);
    admin_state.record(entry).await;

    Response::from_parts(parts, Body::from(body_bytes))
}

fn build_http_client(
    timeout: Duration,
    tls_handshake_timeout: Duration,
    insecure_skip_verify: bool,
) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(tls_handshake_timeout)
        .danger_accept_invalid_certs(insecure_skip_verify)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

fn log_slow_request(config: &Config, entry: &AdminLogEntry) {
    if config.slow_log_threshold.is_zero() {
        return;
    }
    if entry.duration_ms < config.slow_log_threshold.as_millis() as i64 {
        return;
    }

    tracing::warn!(
        path = entry.path,
        status_code = entry.status_code,
        duration_ms = entry.duration_ms,
        "slow request"
    );
}

fn get_output_mode(query: Option<&str>, body: &Value) -> OutputMode {
    if query_contains_output_url(query) {
        return OutputMode::Url;
    }

    if body
        .get("output")
        .and_then(Value::as_str)
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("url"))
    {
        return OutputMode::Url;
    }

    if body
        .pointer("/generationConfig/imageConfig/output")
        .and_then(Value::as_str)
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("url"))
    {
        return OutputMode::Url;
    }

    if body
        .pointer("/generation_config/image_config/output")
        .and_then(Value::as_str)
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("url"))
    {
        return OutputMode::Url;
    }

    OutputMode::Base64
}

fn build_upstream_url(base_url: &str, path: &str, query: Option<&str>) -> Result<String> {
    let mut parsed = Url::parse(base_url)?;
    parsed.set_path(path);

    let filtered_query = filter_query_without_output(query);
    parsed.set_query(filtered_query.as_deref());
    Ok(parsed.to_string())
}

fn filter_query_without_output(query: Option<&str>) -> Option<String> {
    let query = query?;
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    let mut has_any = false;
    for (key, value) in form_urlencoded::parse(query.as_bytes()) {
        if key == "output" {
            continue;
        }
        serializer.append_pair(key.as_ref(), value.as_ref());
        has_any = true;
    }
    if has_any {
        Some(serializer.finish())
    } else {
        None
    }
}

fn query_contains_output_url(query: Option<&str>) -> bool {
    query
        .into_iter()
        .flat_map(|query| form_urlencoded::parse(query.as_bytes()))
        .any(|(key, value)| key == "output" && value.trim().eq_ignore_ascii_case("url"))
}

fn extract_openai_b64_json_entries(body: &Value) -> Result<Vec<String>> {
    let data = body.get("data").and_then(Value::as_array).ok_or_else(|| {
        StructuredProxyError::new(
            "upstream response missing data",
            "rewrite_openai_image_response",
            "missing_data",
            "upstream response missing data array",
        )
    })?;
    if data.is_empty() {
        return Err(StructuredProxyError::new(
            "upstream response missing data",
            "rewrite_openai_image_response",
            "missing_data",
            "upstream response data array is empty",
        )
        .into());
    }

    data.iter()
        .map(|item| {
            item.get("b64_json")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(ToOwned::to_owned)
                .ok_or_else(|| {
                    StructuredProxyError::new(
                        "upstream response missing b64_json",
                        "rewrite_openai_image_response",
                        "missing_b64_json",
                        "upstream response missing data[].b64_json",
                    )
                    .into()
                })
        })
        .collect()
}

fn build_openai_image_output_url(config: &Config, provider: &str, target_url: &str) -> String {
    let external_proxy_prefix = config.resolved_external_image_proxy_prefix();
    if config.proxy_standard_output_urls
        && !provider.eq_ignore_ascii_case("r2")
        && !external_proxy_prefix.trim().is_empty()
    {
        return wrap_external_proxy_url(&external_proxy_prefix, target_url);
    }
    target_url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::extract::{Path as AxumPath, State as AxumState};
    use axum::http::Uri;
    use axum::http::header::AUTHORIZATION;
    use axum::http::header::CONTENT_TYPE;
    use axum::response::IntoResponse;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use serde_json::json;
    use std::collections::VecDeque;
    use std::convert::Infallible;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;
    use tower::make::Shared;
    use tower::service_fn;

    #[derive(Clone, Default)]
    struct AiapidevMockState {
        create_paths: Arc<Mutex<Vec<String>>>,
        create_headers: Arc<Mutex<Vec<HeaderMap>>>,
        create_bodies: Arc<Mutex<Vec<Value>>>,
        poll_headers: Arc<Mutex<Vec<HeaderMap>>>,
        poll_responses: Arc<Mutex<VecDeque<MockPollResponse>>>,
    }

    #[derive(Clone)]
    enum MockPollResponse {
        Success(Value),
        Error(StatusCode, Value),
    }

    impl Default for MockPollResponse {
        fn default() -> Self {
            Self::Success(json!({
                "requestId": "req_demo",
                "status": "succeeded",
                "result": {
                    "items": [{
                        "url": "https://pub.example.com/result.png",
                        "type": "image"
                    }]
                }
            }))
        }
    }

    #[test]
    fn aiapidev_poll_timeout_is_450_seconds() {
        assert_eq!(AIAPIDEV_MAX_POLL_TIME, Duration::from_secs(450));
    }

    #[test]
    fn proxy_error_json_contains_structured_fields() {
        let body = proxy_error_json(
            502,
            "failed to decode upstream response body",
            "proxy",
            "decode_upstream_body",
            "body_decode_failed",
        );

        assert_eq!(body["error"]["code"], 502);
        assert_eq!(
            body["error"]["message"],
            "failed to decode upstream response body"
        );
        assert_eq!(body["error"]["source"], "proxy");
        assert_eq!(body["error"]["stage"], "decode_upstream_body");
        assert_eq!(body["error"]["kind"], "body_decode_failed");
    }

    #[tokio::test]
    async fn aiapidev_flow_polls_task_and_rewrites_result() {
        let state = AiapidevMockState::default();
        let app = Router::new()
            .route(
                "/v1beta/models/nanobananapro:generateContent",
                post(mock_aiapidev_create),
            )
            .route("/v1beta/tasks/{request_id}", get(mock_aiapidev_poll))
            .with_state(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let resolved = ResolvedUpstream {
            base_url: format!("http://{address}"),
            api_key: "special-key".to_string(),
        };
        let request_body = json!({
            "contents": [{
                "role": "user",
                "parts": [{
                    "text": "两张图片合并"
                }]
            }]
        });

        let response = handle_aiapidev_response(
            &resolved,
            "/v1beta/models/nanobananapro:generateContent",
            Some("output=url"),
            request_body,
            OutputMode::Url,
            &reqwest::Client::new(),
            &reqwest::Client::new(),
            None,
            &crate::test_config(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json_body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            json_body["candidates"][0]["content"]["parts"][0]["inlineData"]["data"],
            "https://pub.example.com/result.png"
        );

        let create_paths = state.create_paths.lock().await.clone();
        assert_eq!(
            create_paths.as_slice(),
            ["/v1beta/models/nanobananapro:generateContent"]
        );

        let create_headers = state.create_headers.lock().await;
        assert_eq!(
            create_headers[0]
                .get("x-goog-api-key")
                .and_then(|value| value.to_str().ok()),
            Some("special-key")
        );
        assert!(create_headers[0].get(AUTHORIZATION).is_none());
        drop(create_headers);

        let poll_headers = state.poll_headers.lock().await;
        assert_eq!(
            poll_headers[0]
                .get("x-goog-api-key")
                .and_then(|value| value.to_str().ok()),
            Some("special-key")
        );
        assert!(poll_headers[0].get(AUTHORIZATION).is_none());
        drop(poll_headers);

        let create_bodies = state.create_bodies.lock().await;
        assert_eq!(
            create_bodies[0]["contents"][0]["parts"][0]["text"],
            "两张图片合并"
        );
    }

    #[tokio::test]
    async fn forward_gemini_request_rewrites_aiapidev_base64_inline_data_before_create_call() {
        let mock_state = AiapidevMockState::default();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let proxy_state = mock_state.clone();
        tokio::spawn(async move {
            let service = service_fn(move |request| {
                let state = proxy_state.clone();
                async move { Ok::<_, Infallible>(mock_aiapidev_proxy(state, request).await) }
            });
            axum::serve(listener, Shared::new(service)).await.unwrap();
        });

        let resolved = ResolvedUpstream {
            base_url: "http://www.aiapidev.com".to_string(),
            api_key: "special-key".to_string(),
        };
        let upstream_client = reqwest::Client::builder()
            .proxy(reqwest::Proxy::http(format!("http://127.0.0.1:{}", address.port())).unwrap())
            .build()
            .unwrap();
        let config = crate::test_config();
        let state = AppState {
            config: Arc::new(config.clone()),
            upstream_client,
            image_client: reqwest::Client::new(),
            uploader: Arc::new(Uploader::new(reqwest::Client::new(), config)),
            admin: None,
            request_inline_data_fetch_service: None,
            response_inline_data_fetch_service: None,
            blob_runtime: Arc::new(crate::test_blob_runtime(8 * 1024 * 1024)),
        };
        let request = Request::builder()
            .method("POST")
            .uri("/v1beta/models/gemini-3-pro-image-preview:generateContent?output=url")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({
                    "contents": [{
                        "role": "user",
                        "parts": [{
                            "inlineData": {
                                "data": "iVBORw0KGgoAAAANSUhEUgAAAAUA",
                                "mimeType": "image/png"
                            }
                        }]
                    }]
                })
                .to_string(),
            ))
            .unwrap();

        let (response, admin_entry) = forward_gemini_request(
            state,
            resolved,
            "/v1beta/models/gemini-3-pro-image-preview:generateContent".to_string(),
            request,
            RequestLogSnapshot::default(),
        )
        .await
        .unwrap();

        let status = response.status();
        let response_body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            status,
            StatusCode::OK,
            "unexpected body: {}",
            String::from_utf8_lossy(&response_body)
        );
        assert_eq!(admin_entry.output_mode, "url");

        let create_bodies = mock_state.create_bodies.lock().await;
        assert_eq!(
            create_bodies[0]["contents"][0]["parts"][0]["inline_data"],
            json!({
                "data": "iVBORw0KGgoAAAANSUhEUgAAAAUA",
                "mime_type": "image/png"
            })
        );
        assert!(
            create_bodies[0]["contents"][0]["parts"][0]
                .get("inlineData")
                .is_none()
        );
    }

    #[test]
    fn request_cache_tracking_records_per_request_cache_hits() {
        let stats = Arc::new(crate::admin::AdminStats::default());
        let tracking = build_request_cache_tracking(Some(Arc::clone(&stats)));
        let observer = tracking.observer.as_ref().unwrap();

        observer("https://img.example/first.png", false);
        observer("https://img.example/first.png", true);
        observer("https://img.example/second.png", true);

        assert_eq!(
            stats.cache_hits.load(std::sync::atomic::Ordering::Relaxed),
            2
        );
        assert_eq!(
            tracking.hit_urls.lock().unwrap().clone(),
            vec![
                "https://img.example/first.png".to_string(),
                "https://img.example/second.png".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn model_action_admin_success_sanitizes_request_body_once() {
        let upstream_addr = spawn_generate_content_server(json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "text": "ok"
                    }]
                }
            }]
        }))
        .await;

        let mut config = crate::test_config();
        config.admin_password = "pw".to_string();
        config.upstream_base_url = format!("http://{upstream_addr}");
        config.upstream_api_key = "env-key".to_string();
        let admin = Arc::new(AdminState::new("pw".to_string()));
        let state = test_app_state(config.clone(), Some(Arc::clone(&admin)));

        let response = model_action(
            State(state),
            Path("demo:generateContent".to_string()),
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent")
                .header(CONTENT_TYPE, "application/json")
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
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let logs = admin.snapshot_logs().await;
        assert_eq!(logs.len(), 1);
        assert!(logs[0].request_raw.contains("\"text\": \"hello\""));
    }

    #[tokio::test]
    async fn model_action_failure_preserves_request_cache_hits_in_admin_log() {
        let image_request_count = Arc::new(AtomicUsize::new(0));
        let image_addr = spawn_image_server(Arc::clone(&image_request_count)).await;
        let image_url = format!("http://{image_addr}/image.png");

        let mut config = crate::test_config();
        config.admin_password = "pw".to_string();
        config.upstream_base_url = "http://127.0.0.1:9".to_string();
        config.upstream_api_key = "env-key".to_string();
        config.inline_data_url_memory_cache_max_bytes = 1024;
        config.inline_data_url_background_fetch_wait_timeout = Duration::from_millis(100);
        config.inline_data_url_background_fetch_total_timeout = Duration::from_millis(500);
        let request_fetch_service = crate::cache::InlineDataUrlFetchService::from_config(
            &config,
            reqwest::Client::new(),
            crate::image_io::REQUEST_MAX_IMAGE_BYTES,
            true,
        )
        .unwrap();
        let first_fetch = request_fetch_service.fetch(&image_url).await.unwrap();
        assert!(!first_fetch.from_cache);

        let admin = Arc::new(AdminState::new("pw".to_string()));
        let mut state = test_app_state(config, Some(Arc::clone(&admin)));
        state.request_inline_data_fetch_service = Some(request_fetch_service);

        let response = model_action(
            State(state),
            Path("demo:generateContent".to_string()),
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/demo:generateContent")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "contents": [{
                            "parts": [{
                                "inlineData": {
                                    "data": image_url
                                }
                            }]
                        }]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let logs = admin.snapshot_logs().await;
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].request_raw_image_cache_hits, vec![image_url]);
        assert_eq!(
            admin
                .stats()
                .cache_hits
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        assert_eq!(image_request_count.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn aiapidev_poll_failure_preserves_upstream_status_and_body() {
        let state = AiapidevMockState::default();
        let app = Router::new()
            .route(
                "/v1beta/models/nanobananapro:generateContent",
                post(mock_aiapidev_create),
            )
            .route(
                "/v1beta/tasks/{request_id}",
                get(mock_aiapidev_poll_rate_limited),
            )
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let resolved = ResolvedUpstream {
            base_url: format!("http://{address}"),
            api_key: "special-key".to_string(),
        };

        let response = handle_aiapidev_response(
            &resolved,
            "/v1beta/models/nanobananapro:generateContent",
            None,
            json!({"contents": []}),
            OutputMode::Url,
            &reqwest::Client::new(),
            &reqwest::Client::new(),
            None,
            &crate::test_config(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json_body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_body["error"]["message"], "rate limited");
    }

    #[tokio::test]
    async fn aiapidev_create_invalid_json_returns_structured_proxy_error() {
        let app = Router::new().route(
            "/v1beta/models/nanobananapro:generateContent",
            post(mock_aiapidev_create_invalid_json),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let resolved = ResolvedUpstream {
            base_url: format!("http://{address}"),
            api_key: "special-key".to_string(),
        };

        let response = handle_aiapidev_response(
            &resolved,
            "/v1beta/models/nanobananapro:generateContent",
            None,
            json!({"contents": []}),
            OutputMode::Url,
            &reqwest::Client::new(),
            &reqwest::Client::new(),
            None,
            &crate::test_config(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json_body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            json_body["error"]["message"],
            "failed to parse aiapidev create response"
        );
        assert_eq!(json_body["error"]["source"], "proxy");
        assert_eq!(json_body["error"]["stage"], "aiapidev_create_task");
        assert_eq!(json_body["error"]["kind"], "invalid_json");
    }

    #[tokio::test]
    async fn aiapidev_poll_invalid_json_returns_structured_proxy_error() {
        let app = Router::new()
            .route(
                "/v1beta/models/nanobananapro:generateContent",
                post(mock_aiapidev_create),
            )
            .route(
                "/v1beta/tasks/{request_id}",
                get(mock_aiapidev_poll_invalid_json),
            )
            .with_state(AiapidevMockState::default());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let resolved = ResolvedUpstream {
            base_url: format!("http://{address}"),
            api_key: "special-key".to_string(),
        };

        let response = handle_aiapidev_response(
            &resolved,
            "/v1beta/models/nanobananapro:generateContent",
            None,
            json!({"contents": []}),
            OutputMode::Url,
            &reqwest::Client::new(),
            &reqwest::Client::new(),
            None,
            &crate::test_config(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json_body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            json_body["error"]["message"],
            "failed to parse aiapidev poll response"
        );
        assert_eq!(json_body["error"]["source"], "proxy");
        assert_eq!(json_body["error"]["stage"], "aiapidev_parse_task_response");
        assert_eq!(json_body["error"]["kind"], "invalid_json");
    }

    #[tokio::test]
    async fn aiapidev_poll_retryable_failure_recovers_before_failure_limit() {
        let state = AiapidevMockState {
            poll_responses: Arc::new(Mutex::new(VecDeque::from(vec![
                MockPollResponse::Error(
                    StatusCode::TOO_MANY_REQUESTS,
                    json!({"error": {"message": "rate limited once"}}),
                ),
                MockPollResponse::Error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    json!({"error": {"message": "upstream busy"}}),
                ),
                MockPollResponse::default(),
            ]))),
            ..Default::default()
        };
        let app = Router::new()
            .route(
                "/v1beta/models/nanobananapro:generateContent",
                post(mock_aiapidev_create),
            )
            .route(
                "/v1beta/tasks/{request_id}",
                get(mock_aiapidev_poll_from_sequence),
            )
            .with_state(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let resolved = ResolvedUpstream {
            base_url: format!("http://{address}"),
            api_key: "special-key".to_string(),
        };

        let response = handle_aiapidev_response(
            &resolved,
            "/v1beta/models/nanobananapro:generateContent",
            Some("output=url"),
            json!({"contents": []}),
            OutputMode::Url,
            &reqwest::Client::new(),
            &reqwest::Client::new(),
            None,
            &crate::test_config(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json_body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            json_body["candidates"][0]["content"]["parts"][0]["inlineData"]["data"],
            "https://pub.example.com/result.png"
        );
        assert_eq!(state.poll_headers.lock().await.len(), 3);
    }

    #[tokio::test]
    async fn aiapidev_poll_returns_last_retryable_response_after_five_failures() {
        let state = AiapidevMockState {
            poll_responses: Arc::new(Mutex::new(VecDeque::from(vec![
                MockPollResponse::Error(
                    StatusCode::TOO_MANY_REQUESTS,
                    json!({"error": {"message": "rate limited 1"}}),
                ),
                MockPollResponse::Error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    json!({"error": {"message": "busy 2"}}),
                ),
                MockPollResponse::Error(
                    StatusCode::BAD_GATEWAY,
                    json!({"error": {"message": "gateway 3"}}),
                ),
                MockPollResponse::Error(
                    StatusCode::GATEWAY_TIMEOUT,
                    json!({"error": {"message": "timeout 4"}}),
                ),
                MockPollResponse::Error(
                    StatusCode::TOO_MANY_REQUESTS,
                    json!({"error": {"message": "rate limited 5"}}),
                ),
            ]))),
            ..Default::default()
        };
        let app = Router::new()
            .route(
                "/v1beta/models/nanobananapro:generateContent",
                post(mock_aiapidev_create),
            )
            .route(
                "/v1beta/tasks/{request_id}",
                get(mock_aiapidev_poll_from_sequence),
            )
            .with_state(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let resolved = ResolvedUpstream {
            base_url: format!("http://{address}"),
            api_key: "special-key".to_string(),
        };

        let response = handle_aiapidev_response(
            &resolved,
            "/v1beta/models/nanobananapro:generateContent",
            None,
            json!({"contents": []}),
            OutputMode::Url,
            &reqwest::Client::new(),
            &reqwest::Client::new(),
            None,
            &crate::test_config(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json_body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json_body["error"]["message"], "rate limited 5");
        assert_eq!(state.poll_headers.lock().await.len(), 5);
    }

    async fn mock_aiapidev_create(
        AxumState(state): AxumState<AiapidevMockState>,
        headers: HeaderMap,
        request: Request,
    ) -> Json<Value> {
        let path = request.uri().path().to_string();
        let body = to_bytes(request.into_body(), usize::MAX).await.unwrap();
        let parsed: Value = serde_json::from_slice(&body).unwrap();

        state.create_paths.lock().await.push(path);
        state.create_headers.lock().await.push(headers);
        state.create_bodies.lock().await.push(parsed);

        Json(json!({
            "requestId": "req_demo",
            "status": "created"
        }))
    }

    async fn mock_aiapidev_create_invalid_json() -> Response {
        (
            StatusCode::OK,
            [(CONTENT_TYPE, HeaderValue::from_static("application/json"))],
            "not-json",
        )
            .into_response()
    }

    async fn mock_aiapidev_poll(
        AxumState(state): AxumState<AiapidevMockState>,
        AxumPath(_request_id): AxumPath<String>,
        headers: HeaderMap,
    ) -> Json<Value> {
        state.poll_headers.lock().await.push(headers);
        Json(json!({
            "requestId": "req_demo",
            "status": "succeeded",
            "result": {
                "items": [{
                    "url": "https://pub.example.com/result.png",
                    "type": "image"
                }]
            }
        }))
    }

    async fn mock_aiapidev_poll_invalid_json(
        AxumState(state): AxumState<AiapidevMockState>,
        AxumPath(_request_id): AxumPath<String>,
        headers: HeaderMap,
    ) -> Response {
        state.poll_headers.lock().await.push(headers);
        (
            StatusCode::OK,
            [(CONTENT_TYPE, HeaderValue::from_static("application/json"))],
            "not-json",
        )
            .into_response()
    }

    async fn mock_aiapidev_poll_rate_limited(
        AxumState(state): AxumState<AiapidevMockState>,
        AxumPath(_request_id): AxumPath<String>,
        headers: HeaderMap,
    ) -> (StatusCode, Json<Value>) {
        state.poll_headers.lock().await.push(headers);
        (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "error": {
                    "message": "rate limited"
                }
            })),
        )
    }

    async fn mock_aiapidev_poll_from_sequence(
        AxumState(state): AxumState<AiapidevMockState>,
        AxumPath(_request_id): AxumPath<String>,
        headers: HeaderMap,
    ) -> Response {
        state.poll_headers.lock().await.push(headers);
        let next = state
            .poll_responses
            .lock()
            .await
            .pop_front()
            .unwrap_or_default();
        match next {
            MockPollResponse::Success(body) => Json(body).into_response(),
            MockPollResponse::Error(status, body) => (status, Json(body)).into_response(),
        }
    }

    async fn mock_aiapidev_proxy(state: AiapidevMockState, request: Request) -> Response {
        let path = extract_proxy_path(request.uri());
        let headers = request.headers().clone();

        if path == "/v1beta/models/nanobananapro:generateContent" {
            let body = to_bytes(request.into_body(), usize::MAX).await.unwrap();
            let parsed: Value = serde_json::from_slice(&body).unwrap();
            state.create_paths.lock().await.push(path);
            state.create_headers.lock().await.push(headers);
            state.create_bodies.lock().await.push(parsed);
            return Json(json!({
                "requestId": "req_demo",
                "status": "created"
            }))
            .into_response();
        }

        if path == "/v1beta/tasks/req_demo" {
            state.poll_headers.lock().await.push(headers);
            return Json(json!({
                "requestId": "req_demo",
                "status": "succeeded",
                "result": {
                    "items": [{
                        "url": "https://pub.example.com/result.png",
                        "type": "image"
                    }]
                }
            }))
            .into_response();
        }

        StatusCode::NOT_FOUND.into_response()
    }

    fn extract_proxy_path(uri: &Uri) -> String {
        let raw = uri.to_string();
        if let Ok(parsed) = Url::parse(&raw) {
            return parsed.path().to_string();
        }
        uri.path().to_string()
    }

    fn test_app_state(config: Config, admin: Option<Arc<AdminState>>) -> AppState {
        let uploader_config = config.clone();
        AppState {
            upstream_client: build_upstream_client(&config),
            image_client: reqwest::Client::new(),
            uploader: Arc::new(Uploader::new(reqwest::Client::new(), uploader_config)),
            admin,
            request_inline_data_fetch_service: None,
            response_inline_data_fetch_service: None,
            blob_runtime: Arc::new(crate::test_blob_runtime(8 * 1024 * 1024)),
            config: Arc::new(config),
        }
    }

    async fn spawn_generate_content_server(body: Value) -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let app = Router::new().route(
                "/v1beta/models/demo:generateContent",
                post(move || {
                    let body = body.clone();
                    async move { Json(body) }
                }),
            );
            axum::serve(listener, app).await.unwrap();
        });

        address
    }

    async fn spawn_image_server(request_count: Arc<AtomicUsize>) -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let app = Router::new().route(
                "/image.png",
                get(move || {
                    let request_count = Arc::clone(&request_count);
                    async move {
                        request_count.fetch_add(1, Ordering::Relaxed);
                        (
                            StatusCode::OK,
                            [(CONTENT_TYPE, HeaderValue::from_static("image/png"))],
                            vec![137, 80, 78, 71, 13, 10, 26, 10],
                        )
                    }
                }),
            );
            axum::serve(listener, app).await.unwrap();
        });

        address
    }
}
