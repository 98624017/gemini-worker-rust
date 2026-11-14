use super::*;
use crate::mai_token::TaskState;

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const MAX_CONSECUTIVE_FAILURES: usize = 5;
const MAX_TASK_RESPONSE_BYTES: usize = 64 * 1024;

struct ExchangeFailure {
    response: Response,
    retryable: bool,
    retry_after: Duration,
}

pub(super) async fn handle(
    state: &AppState,
    resolved: &ResolvedUpstream,
    body: Value,
    mut entry: AdminLogEntry,
    cache_key: Option<&UpstreamBlockCacheKey>,
    deadline: tokio::time::Instant,
) -> (Response, AdminLogEntry) {
    entry.upstream_mode = "mai-token".into();
    entry.upstream_stage = "mai_token_validate_request".into();
    let payload = match crate::mai_token::build_request_body(&body) {
        Ok(payload) => payload,
        Err(error) => {
            let response = error_response(
                &entry,
                StatusCode::BAD_REQUEST,
                "proxy",
                "invalid_request",
                &error.to_string(),
            );
            entry.status_code = response.status().as_u16();
            return (response, entry);
        }
    };
    if state.admin.is_some() {
        let logged = admin::sanitize_json_value_for_log(&payload);
        entry.request_upstream = redact(&logged.pretty, resolved, &state.config);
        entry.request_upstream_images = logged.image_urls;
    }
    let started = Instant::now();
    let result = tokio::time::timeout_at(
        deadline,
        execute(state, resolved, payload, &mut entry, cache_key),
    )
    .await;
    entry.upstream_build_ms = started.elapsed().as_millis() as i64;
    let response = match result {
        Ok(Ok((urls, created))) => {
            entry.upstream_stage = "mai_token_complete".into();
            entry.error_detail.clear();
            let images: Vec<_> = urls
                .into_iter()
                .map(|url| crate::openai_image::UploadedImage {
                    url: build_openai_image_output_url(&state.config, "direct", &url),
                })
                .collect();
            Json(crate::openai_image::build_response_payload_from_uploaded(
                &images, created,
            ))
            .into_response()
        }
        Ok(Err(response)) => response,
        Err(_) => error_response(
            &entry,
            StatusCode::GATEWAY_TIMEOUT,
            "proxy",
            "upstream_timeout",
            if entry.upstream_task_id.is_empty() {
                "上游任务提交结果尚未确认，请勿直接重复提交，请联系上游核对"
            } else {
                "等待上游图片任务超时，请查询已有任务，不要重复创建"
            },
        ),
    };
    entry.status_code = response.status().as_u16();
    (response, entry)
}

async fn execute(
    state: &AppState,
    resolved: &ResolvedUpstream,
    payload: Value,
    entry: &mut AdminLogEntry,
    cache_key: Option<&UpstreamBlockCacheKey>,
) -> std::result::Result<(Vec<String>, i64), Response> {
    entry.upstream_stage = "mai_token_submit".into();
    let submit_url = api_url(&resolved.base_url, None).map_err(|error| {
        error_response(
            entry,
            StatusCode::BAD_GATEWAY,
            "proxy",
            "invalid_upstream_url",
            &error.to_string(),
        )
    })?;
    let encoded = serde_json::to_vec(&payload).map_err(|_| {
        error_response(
            entry,
            StatusCode::INTERNAL_SERVER_ERROR,
            "proxy",
            "request_encode_failed",
            "编码上游图片请求失败",
        )
    })?;
    let content_length = encoded.len().to_string();
    // 不可重放的 body 阻止 HTTP 客户端自动重新提交计费任务。
    let body = reqwest::Body::wrap_stream(ReaderStream::new(std::io::Cursor::new(encoded)));
    let request = state
        .upstream_client
        .post(submit_url)
        .bearer_auth(&resolved.api_key)
        .header(CONTENT_TYPE, "application/json")
        .header("content-length", content_length)
        .body(body);
    let submitted = exchange(state, resolved, request, entry, cache_key)
        .await
        .map_err(|failure| failure.response)?;
    let task_id = crate::mai_token::task_id(&submitted);
    if let Ok(task_id) = &task_id {
        entry.upstream_task_id = redact(task_id, resolved, &state.config);
    }
    if let Some(result) = task_result(state, resolved, &submitted, entry, cache_key).await? {
        return Ok(result);
    }
    let task_id = task_id.map_err(|error| {
        error_response(
            entry,
            StatusCode::BAD_GATEWAY,
            "proxy",
            "invalid_task_response",
            &error.to_string(),
        )
    })?;
    entry.upstream_stage = "mai_token_poll".into();
    let poll_url = api_url(&resolved.base_url, Some(&task_id)).map_err(|error| {
        error_response(
            entry,
            StatusCode::BAD_GATEWAY,
            "proxy",
            "invalid_upstream_url",
            &error.to_string(),
        )
    })?;
    let mut failures = 0;
    tokio::time::sleep(POLL_INTERVAL).await;
    loop {
        let request = state
            .upstream_client
            .get(poll_url.clone())
            .bearer_auth(&resolved.api_key);
        let delay = match exchange(state, resolved, request, entry, cache_key).await {
            Ok(body) => {
                failures = 0;
                if let Some(result) = task_result(state, resolved, &body, entry, cache_key).await? {
                    return Ok(result);
                }
                POLL_INTERVAL
            }
            Err(failure) => {
                failures += 1;
                if !failure.retryable || failures >= MAX_CONSECUTIVE_FAILURES {
                    return Err(failure.response);
                }
                failure.retry_after
            }
        };
        tokio::time::sleep(delay).await;
    }
}

async fn task_result(
    state: &AppState,
    resolved: &ResolvedUpstream,
    body: &Value,
    entry: &AdminLogEntry,
    cache_key: Option<&UpstreamBlockCacheKey>,
) -> std::result::Result<Option<(Vec<String>, i64)>, Response> {
    match crate::mai_token::parse_task_state(body).map_err(|error| {
        error_response(
            entry,
            StatusCode::BAD_GATEWAY,
            "proxy",
            "invalid_task_response",
            &redact(&error.to_string(), resolved, &state.config),
        )
    })? {
        TaskState::Pending => Ok(None),
        TaskState::Completed(urls) => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            Ok(Some((
                urls,
                crate::openai_image::extract_created_timestamp(body, now),
            )))
        }
        TaskState::Failed(message) => {
            let message = redact(
                &format!("上游图片任务失败：{message}"),
                resolved,
                &state.config,
            );
            Err(upstream_block_cache_json_error_response(
                StatusCode::BAD_GATEWAY,
                error_body(
                    entry,
                    StatusCode::BAD_GATEWAY,
                    "upstream",
                    "upstream_task_failed",
                    &message,
                ),
                state.upstream_block_cache.as_ref(),
                cache_key,
            )
            .await)
        }
    }
}

async fn exchange(
    state: &AppState,
    resolved: &ResolvedUpstream,
    request: reqwest::RequestBuilder,
    entry: &mut AdminLogEntry,
    cache_key: Option<&UpstreamBlockCacheKey>,
) -> std::result::Result<Value, ExchangeFailure> {
    let transport_error = |error: reqwest::Error, entry: &mut AdminLogEntry| {
        let class = classify_reqwest_proxy_error(&error);
        let retryable =
            !error.is_builder() && !error.is_redirect() && class.kind != "upstream_tls_failed";
        let status = if error.is_timeout() {
            StatusCode::GATEWAY_TIMEOUT
        } else {
            StatusCode::BAD_GATEWAY
        };
        entry.error_detail = redact(
            &reqwest_error_detail(&error.without_url()),
            resolved,
            &state.config,
        );
        ExchangeFailure {
            response: error_response(entry, status, "proxy", class.kind, class.message),
            retryable,
            retry_after: POLL_INTERVAL,
        }
    };
    let mut response = request
        .send()
        .await
        .map_err(|error| transport_error(error, entry))?;
    let status = response.status();
    entry.upstream_status_code = status.as_u16();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .cloned()
        .unwrap_or_else(|| HeaderValue::from_static("application/json"));
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(POLL_INTERVAL)
        .max(POLL_INTERVAL);
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| transport_error(error, entry))?
    {
        if bytes.len() + chunk.len() > MAX_TASK_RESPONSE_BYTES {
            return Err(ExchangeFailure {
                response: error_response(
                    entry,
                    StatusCode::BAD_GATEWAY,
                    "proxy",
                    "upstream_response_too_large",
                    "上游图片任务响应过大，无法处理",
                ),
                retryable: false,
                retry_after,
            });
        }
        bytes.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        let bytes = redact(&String::from_utf8_lossy(&bytes), resolved, &state.config).into_bytes();
        let blocked = classify_blockable_upstream_error(status, &bytes).is_some();
        maybe_store_upstream_block_error(
            state.upstream_block_cache.as_ref(),
            cache_key,
            status,
            content_type.clone(),
            &bytes,
        )
        .await;
        let mut response = raw_response_with_upstream_status(
            block_error_response_status(status, &bytes),
            status,
            content_type,
            bytes,
        );
        if !entry.upstream_task_id.is_empty() {
            if let Ok(value) = HeaderValue::from_str(&entry.upstream_task_id) {
                response.headers_mut().insert("x-upstream-task-id", value);
            }
        }
        return Err(ExchangeFailure {
            response,
            retryable: !blocked && is_aiapidev_retryable_poll_status(status),
            retry_after,
        });
    }
    serde_json::from_slice(&bytes).map_err(|_| ExchangeFailure {
        response: error_response(
            entry,
            StatusCode::BAD_GATEWAY,
            "proxy",
            "invalid_task_response",
            "上游图片任务响应不是有效 JSON",
        ),
        retryable: false,
        retry_after,
    })
}

fn api_url(base_url: &str, task_id: Option<&str>) -> Result<Url> {
    let mut url = Url::parse(base_url).map_err(|_| anyhow!(MSG_UPSTREAM_URL_INVALID))?;
    url.set_query(None);
    url.set_fragment(None);
    url.set_path("/v1/videos");
    if let Some(task_id) = task_id {
        url.path_segments_mut()
            .map_err(|_| anyhow!(MSG_UPSTREAM_URL_INVALID))?
            .push(task_id);
    }
    Ok(url)
}

fn error_body(
    entry: &AdminLogEntry,
    status: StatusCode,
    source: &str,
    kind: &str,
    message: &str,
) -> Value {
    let mut body = proxy_error_json(
        status.as_u16(),
        message,
        source,
        &entry.upstream_stage,
        kind,
    );
    if !entry.upstream_task_id.is_empty() {
        body["error"]["upstream_task_id"] = json!(entry.upstream_task_id);
    }
    body
}

fn error_response(
    entry: &AdminLogEntry,
    status: StatusCode,
    source: &str,
    kind: &str,
    message: &str,
) -> Response {
    (
        status,
        Json(error_body(entry, status, source, kind, message)),
    )
        .into_response()
}

fn redact(value: &str, resolved: &ResolvedUpstream, config: &Config) -> String {
    [&resolved.api_key, &config.upstream_api_key]
        .into_iter()
        .filter(|key| !key.is_empty())
        .fold(value.to_string(), |text, key| {
            text.replace(key, "[redacted]")
        })
}
