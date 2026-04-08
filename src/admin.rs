use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use axum::Json;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::http::header::WWW_AUTHENTICATE;
use axum::response::{Html, IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Serialize;
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::sync::Mutex;

pub const ADMIN_MAX_BODY_BYTES_PER_ENTRY: usize = 64 * 1024;
const ADMIN_LOG_CAPACITY: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SanitizedAdminLog {
    pub pretty: String,
    pub image_urls: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AdminLogEntry {
    pub id: i64,
    pub created_at: String,
    pub method: String,
    pub path: String,
    pub query: String,
    pub remote_addr: String,
    pub is_stream: bool,
    pub output_mode: String,
    pub status_code: u16,
    pub duration_ms: i64,
    pub finish_reason: String,
    pub request_raw: String,
    pub request_raw_images: Vec<String>,
    pub request_raw_image_cache_hits: Vec<String>,
    pub request_upstream: String,
    pub request_upstream_images: Vec<String>,
    pub response_downstream: String,
    pub response_images: Vec<String>,
}

#[derive(Default)]
pub struct AdminStats {
    pub total_requests: AtomicI64,
    pub error_requests: AtomicI64,
    pub total_duration_ms: AtomicI64,
    pub cache_hits: AtomicI64,
}

#[derive(Default)]
struct AdminLogBuffer {
    next_id: AtomicI64,
    entries: Mutex<VecDeque<AdminLogEntry>>,
}

#[derive(Clone)]
pub struct AdminState {
    password: String,
    logs: Arc<AdminLogBuffer>,
    stats: Arc<AdminStats>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AdminStatsPayload {
    total_requests: i64,
    error_requests: i64,
    total_duration_ms: i64,
    cache_hits: i64,
}

impl AdminState {
    pub fn new(password: String) -> Self {
        Self {
            password,
            logs: Arc::new(AdminLogBuffer::default()),
            stats: Arc::new(AdminStats::default()),
        }
    }

    pub fn enabled(&self) -> bool {
        !self.password.trim().is_empty()
    }

    pub async fn record(&self, mut entry: AdminLogEntry) {
        let id = self.logs.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        entry.id = id;

        let mut guard = self.logs.entries.lock().await;
        guard.push_front(entry);
        while guard.len() > ADMIN_LOG_CAPACITY {
            guard.pop_back();
        }
    }

    pub fn stats(&self) -> Arc<AdminStats> {
        Arc::clone(&self.stats)
    }

    pub async fn snapshot_logs(&self) -> Vec<AdminLogEntry> {
        self.logs.entries.lock().await.iter().cloned().collect()
    }

    pub fn unauthorized_response(&self) -> Response {
        let mut response = (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": {"code": 401, "message": "Unauthorized"}})),
        )
            .into_response();
        response.headers_mut().insert(
            WWW_AUTHENTICATE,
            r#"Basic realm="banana-proxy-admin""#.parse().unwrap(),
        );
        response
    }

    pub fn check_basic_auth(&self, headers: &HeaderMap) -> bool {
        if !self.enabled() {
            return false;
        }
        let Some(raw) = headers.get("authorization") else {
            return false;
        };
        let Ok(raw) = raw.to_str() else {
            return false;
        };
        let Some(encoded) = raw.strip_prefix("Basic ") else {
            return false;
        };
        let Ok(decoded) = STANDARD.decode(encoded.trim()) else {
            return false;
        };
        let Ok(decoded) = String::from_utf8(decoded) else {
            return false;
        };
        let Some((_, password)) = decoded.split_once(':') else {
            return false;
        };
        password == self.password
    }
}

pub fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

pub fn admin_logs_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html lang="zh-CN">
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>banana-proxy 管理后台</title></head>
<body><h1>banana-proxy 管理后台</h1><p>请使用 <code>/admin/api/logs</code> 与 <code>/admin/api/stats</code>。</p></body>
</html>"#,
    )
}

pub async fn admin_logs_response(state: &AdminState) -> Response {
    Json(json!({"items": state.snapshot_logs().await})).into_response()
}

pub fn admin_stats_response(state: &AdminState) -> Response {
    let stats = state.stats();
    Json(AdminStatsPayload {
        total_requests: stats.total_requests.load(Ordering::Relaxed),
        error_requests: stats.error_requests.load(Ordering::Relaxed),
        total_duration_ms: stats.total_duration_ms.load(Ordering::Relaxed),
        cache_hits: stats.cache_hits.load(Ordering::Relaxed),
    })
    .into_response()
}

pub fn sanitize_json_for_log(raw: &[u8]) -> SanitizedAdminLog {
    if raw.is_empty() {
        return SanitizedAdminLog {
            pretty: String::new(),
            image_urls: Vec::new(),
        };
    }

    let mut root = match serde_json::from_slice::<Value>(raw) {
        Ok(root) => root,
        Err(_) => {
            return SanitizedAdminLog {
                pretty: truncate_for_admin_log(
                    &String::from_utf8_lossy(raw),
                    ADMIN_MAX_BODY_BYTES_PER_ENTRY,
                ),
                image_urls: Vec::new(),
            };
        }
    };

    let image_urls = redact_inline_data_and_collect_image_urls(&mut root);
    let pretty = serde_json::to_string_pretty(&root)
        .map(|text| truncate_for_admin_log(&text, ADMIN_MAX_BODY_BYTES_PER_ENTRY))
        .unwrap_or_else(|_| {
            truncate_for_admin_log(
                &String::from_utf8_lossy(raw),
                ADMIN_MAX_BODY_BYTES_PER_ENTRY,
            )
        });

    SanitizedAdminLog { pretty, image_urls }
}

pub fn extract_finish_reason(body: &Value) -> Option<String> {
    body.get("candidates")
        .and_then(Value::as_array)
        .and_then(|candidates| candidates.first())
        .and_then(|candidate| candidate.get("finishReason"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn redact_inline_data_and_collect_image_urls(root: &mut Value) -> Vec<String> {
    let mut urls = BTreeSet::new();

    fn walk(node: &mut Value, urls: &mut BTreeSet<String>) {
        match node {
            Value::Object(map) => {
                for key in ["inlineData", "inline_data"] {
                    if let Some(Value::Object(inline)) = map.get_mut(key) {
                        if let Some(Value::String(data)) = inline.get("data") {
                            if is_image_url(data) {
                                urls.insert(data.trim().to_string());
                            } else if !data.trim().is_empty() {
                                inline.insert(
                                    "data".to_string(),
                                    Value::String(format!("[base64 omitted len={}]", data.len())),
                                );
                            }
                        }
                    }
                }

                for child in map.values_mut() {
                    walk(child, urls);
                }
            }
            Value::Array(items) => {
                for child in items {
                    walk(child, urls);
                }
            }
            _ => {}
        }
    }

    walk(root, &mut urls);
    urls.into_iter().collect()
}

fn is_image_url(value: &str) -> bool {
    value.starts_with("http://")
        || value.starts_with("https://")
        || value.starts_with("/proxy/image")
}

fn truncate_for_admin_log(input: &str, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input.to_string();
    }

    let mut boundary = 0usize;
    for (index, _) in input.char_indices() {
        if index > max_bytes {
            break;
        }
        boundary = index;
    }
    let suffix = "\n...[truncated]";
    format!("{}{}", &input[..boundary], suffix)
}
