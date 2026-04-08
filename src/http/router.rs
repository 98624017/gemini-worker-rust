use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use axum::body::{Body, to_bytes};
use axum::extract::{Path, Request, State};
use axum::http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use url::{Url, form_urlencoded};

use crate::admin::{self, AdminLogEntry, AdminState};
use crate::cache::InlineDataUrlFetchService;
use crate::config::Config;
use crate::request_rewrite::{RewriteServices, rewrite_request_inline_data};
use crate::response_rewrite::{
    OutputMode, keep_largest_inline_image, normalize_special_markdown_image_response,
    remove_thought_signatures, rewrite_inline_data_base64_to_urls,
};
use crate::upload::Uploader;
use crate::upstream::{ResolvedUpstream, resolve_upstream_from_header_map};

const MAX_REQUEST_BODY_BYTES: usize = 20 * 1024 * 1024;

#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
    upstream_client: reqwest::Client,
    image_client: reqwest::Client,
    uploader: Arc<Uploader>,
    admin: Option<Arc<AdminState>>,
    inline_data_fetch_service: Option<Arc<InlineDataUrlFetchService>>,
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
    let inline_data_fetch_service = InlineDataUrlFetchService::from_config(
        &config,
        image_client.clone(),
        crate::image_io::DEFAULT_MAX_IMAGE_BYTES,
        false,
    );
    let state = AppState {
        uploader: Arc::new(Uploader::new(upload_client, config.clone())),
        config: Arc::new(config),
        upstream_client: reqwest::Client::new(),
        image_client,
        admin,
        inline_data_fetch_service,
    };

    Router::new()
        .route("/admin", get(admin_root))
        .route("/admin/", get(admin_root))
        .route("/admin/logs", get(admin_logs_page))
        .route("/admin/api/logs", get(admin_logs_api))
        .route("/admin/api/stats", get(admin_stats_api))
        .route("/v1beta/models/{*rest}", post(model_action))
        .with_state(state)
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

    let resolved = match resolve_upstream_from_header_map(
        request.headers(),
        &state.config.upstream_base_url,
        &state.config.upstream_api_key,
    ) {
        Ok(resolved) => resolved,
        Err(err) => {
            let response = (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": {"code": 401, "message": err.to_string()}})),
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
                    status_code: StatusCode::UNAUTHORIZED.as_u16(),
                    duration_ms: started_at.elapsed().as_millis() as i64,
                    ..Default::default()
                },
            )
            .await;
        }
    };

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
    let target_path = format!("/v1beta/models/{rest}");

    match forward_gemini_request(state.clone(), resolved, target_path, request).await {
        Ok((response, mut admin_entry)) => {
            admin_entry.created_at = created_at;
            admin_entry.method = request_method;
            admin_entry.path = request_path;
            admin_entry.query = request_query;
            admin_entry.remote_addr = remote_addr;
            admin_entry.is_stream = false;
            admin_entry.duration_ms = started_at.elapsed().as_millis() as i64;
            finalize_admin_response(&state, response, admin_entry).await
        }
        Err(err) => {
            let response = (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": {"code": 502, "message": err.to_string()}})),
            )
                .into_response();
            finalize_admin_response(
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
                    ..Default::default()
                },
            )
            .await
        }
    }
}

async fn forward_gemini_request(
    state: AppState,
    resolved: ResolvedUpstream,
    target_path: String,
    request: Request,
) -> Result<(Response, AdminLogEntry)> {
    let content_type_header = request.headers().get(CONTENT_TYPE).cloned();
    let accept_header = request.headers().get(ACCEPT).cloned();
    let request_query = request.uri().query().map(ToOwned::to_owned);
    let request_body = to_bytes(request.into_body(), MAX_REQUEST_BODY_BYTES)
        .await
        .map_err(|err| anyhow!("failed to read request body: {err}"))?;
    let request_raw = admin::sanitize_json_for_log(&request_body);

    let mut body: Value =
        serde_json::from_slice(&request_body).map_err(|err| anyhow!("invalid json body: {err}"))?;
    let output_mode = get_output_mode(request_query.as_deref(), &body);
    strip_output_from_value(&mut body);

    let admin_stats = state.admin.as_ref().map(|admin| admin.stats());
    let cache_observer = admin_stats.map(|stats| {
        Arc::new(move |_raw_url: &str, from_cache: bool| {
            if from_cache {
                // Relaxed: 独立统计计数器，读端可接受最终一致。
                stats
                    .cache_hits
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }) as Arc<dyn Fn(&str, bool) + Send + Sync>
    });

    body = rewrite_request_inline_data(
        body,
        &RewriteServices {
            image_client: state.image_client.clone(),
            max_image_bytes: crate::image_io::DEFAULT_MAX_IMAGE_BYTES,
            allow_private_networks: false,
            fetch_service: state.inline_data_fetch_service.clone(),
            cache_observer,
        },
    )
    .await?;
    let request_upstream = admin::sanitize_json_for_log(&serde_json::to_vec(&body)?);

    let upstream_body = serde_json::to_vec(&body)?;
    let upstream_url =
        build_upstream_url(&resolved.base_url, &target_path, request_query.as_deref())?;

    let mut upstream_request = state.upstream_client.post(upstream_url).body(upstream_body);
    if let Some(value) = content_type_header {
        upstream_request = upstream_request.header(CONTENT_TYPE, value);
    }
    if let Some(value) = accept_header {
        upstream_request = upstream_request.header(ACCEPT, value);
    }
    upstream_request = upstream_request.header("x-goog-api-key", resolved.api_key.clone());
    upstream_request =
        upstream_request.header(AUTHORIZATION, format!("Bearer {}", resolved.api_key));

    let upstream_response = upstream_request.send().await?;

    let mut admin_entry = AdminLogEntry {
        output_mode: match output_mode {
            OutputMode::Base64 => "base64".to_string(),
            OutputMode::Url => "url".to_string(),
        },
        request_raw: request_raw.pretty,
        request_raw_images: request_raw.image_urls,
        request_upstream: request_upstream.pretty,
        request_upstream_images: request_upstream.image_urls,
        ..Default::default()
    };

    let response = handle_non_stream_response(
        upstream_response,
        output_mode,
        &state.image_client,
        state.inline_data_fetch_service.as_ref(),
        state.uploader.as_ref(),
        state.config.as_ref(),
    )
    .await?;
    admin_entry.status_code = response.status().as_u16();
    Ok((response, admin_entry))
}

async fn handle_non_stream_response(
    upstream_response: reqwest::Response,
    output_mode: OutputMode,
    image_client: &reqwest::Client,
    fetch_service: Option<&Arc<InlineDataUrlFetchService>>,
    uploader: &Uploader,
    config: &Config,
) -> Result<Response> {
    let status = upstream_response.status();
    let content_type = upstream_response
        .headers()
        .get(CONTENT_TYPE)
        .cloned()
        .unwrap_or_else(|| HeaderValue::from_static("application/json"));
    let body_bytes = upstream_response.bytes().await?;

    if !status.is_success() {
        let mut response = Response::new(Body::from(body_bytes));
        *response.status_mut() = StatusCode::from_u16(status.as_u16())?;
        response.headers_mut().insert(CONTENT_TYPE, content_type);
        return Ok(response);
    }

    let json_body: Value = match serde_json::from_slice(&body_bytes) {
        Ok(body) => body,
        Err(_) => {
            let mut response = Response::new(Body::from(body_bytes));
            *response.status_mut() = StatusCode::from_u16(status.as_u16())?;
            response.headers_mut().insert(CONTENT_TYPE, content_type);
            return Ok(response);
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
    if output_mode == OutputMode::Url {
        final_json = rewrite_inline_data_base64_to_urls(
            final_json,
            uploader,
            &config.public_base_url,
            config.proxy_standard_output_urls,
        )
        .await;
    }
    let final_body = serde_json::to_vec(&final_json)?;
    let mut response = Response::new(Body::from(final_body));
    *response.status_mut() = StatusCode::from_u16(status.as_u16())?;
    response.headers_mut().insert(CONTENT_TYPE, content_type);
    Ok(response)
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
        Some(admin) => admin::admin_stats_response(admin),
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
        entry.response_downstream = sanitized.pretty;
        entry.response_images = sanitized.image_urls;
        if let Ok(value) = serde_json::from_slice::<Value>(&body_bytes) {
            entry.finish_reason = admin::extract_finish_reason(&value).unwrap_or_default();
        }
    }

    entry.status_code = parts.status.as_u16();

    let stats = admin_state.stats();
    // Relaxed: 独立统计计数器，不与其他原子操作构成同步链。
    // 读端（admin stats API）可接受最终一致。
    stats
        .total_requests
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if entry.status_code >= 400 {
        // Relaxed: 独立统计计数器，不与其他原子操作构成同步链。
        // 读端（admin stats API）可接受最终一致。
        stats
            .error_requests
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    // Relaxed: 独立统计计数器，不与其他原子操作构成同步链。
    // 读端（admin stats API）可接受最终一致。
    stats
        .total_duration_ms
        .fetch_add(entry.duration_ms, std::sync::atomic::Ordering::Relaxed);
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

fn strip_output_from_value(body: &mut Value) {
    if let Some(map) = body.as_object_mut() {
        map.remove("output");
    }

    if let Some(image_config) = body.pointer_mut("/generationConfig/imageConfig") {
        if let Some(map) = image_config.as_object_mut() {
            map.remove("output");
        }
    }

    if let Some(image_config) = body.pointer_mut("/generation_config/image_config") {
        if let Some(map) = image_config.as_object_mut() {
            map.remove("output");
        }
    }
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
