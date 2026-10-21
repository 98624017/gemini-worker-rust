use super::*;
use crate::midjourney::{CompletedTask, Error, Mode, PreparedRequest};
use crate::request_encode::encode_data_uri_request_body;
use crate::request_materialize::RequestReplacement;

mod reference;

const POLL_INTERVAL: Duration = Duration::from_secs(10);
const MAX_CONSECUTIVE_FAILURES: usize = 5;
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

struct PendingLog {
    admin: Option<Arc<AdminState>>,
    entry: Option<AdminLogEntry>,
    started_at: Instant,
}

impl Drop for PendingLog {
    fn drop(&mut self) {
        let (Some(admin), Some(mut entry)) = (self.admin.take(), self.entry.take()) else {
            return;
        };
        entry.status_code = 499;
        entry.duration_ms = self.started_at.elapsed().as_millis() as i64;
        entry.error_source = "proxy".into();
        entry.error_stage = entry.upstream_stage.clone();
        entry.error_kind = "downstream_cancelled".into();
        entry.error_message = "调用方已断开，停止本次任务查询".into();
        // 仅异步保存取消记录；生成和查询始终随 HTTP 请求一起释放。
        tokio::spawn(async move {
            admin::apply_admin_stats(admin.stats().as_ref(), &entry);
            admin.record(entry).await;
        });
    }
}

pub(super) async fn handle(
    state: &AppState,
    resolved: &ResolvedUpstream,
    body: Value,
    mut entry: AdminLogEntry,
    started_at: Instant,
    deadline: tokio::time::Instant,
) -> (Response, AdminLogEntry) {
    entry.output_mode = "url".into();
    entry.upstream_stage = "magic_validate_request".into();
    let mut pending = PendingLog {
        admin: state.admin.clone(),
        entry: Some(entry),
        started_at,
    };
    let entry = pending.entry.as_mut().unwrap();
    let cache_key = state
        .upstream_block_cache
        .as_ref()
        .map(|_| UpstreamBlockCacheKey::new("/v1/images/generations", &resolved.base_url, &body));
    let result = match crate::midjourney::prepare_request(body, MAX_OPENAI_REFERENCE_IMAGES) {
        Err(err) => Err(err),
        Ok(request) => {
            entry.upstream_mode = request.mode.as_str().into();
            if let (Some(cache), Some(key)) = (&state.upstream_block_cache, &cache_key)
                && let Some(hit) = cache.get(key).await
            {
                entry.error_source = "proxy".into();
                entry.error_stage = "upstream_block_cache".into();
                entry.error_kind = "cache_hit".into();
                let response = response_from_block_cache_hit(hit);
                return (response, pending.entry.take().unwrap());
            }
            entry.upstream_stage = if !request.reference_urls.is_empty() {
                "magic_prepare_image"
            } else {
                "magic_encode_request"
            }
            .into();
            if tokio::time::Instant::now() >= deadline {
                Err(timeout_error(timeout_stage(entry)))
            } else {
                let upstream_started = Instant::now();
                let result = tokio::time::timeout_at(
                    deadline,
                    execute(state, resolved, &request, entry, deadline),
                )
                .await;
                entry.upstream_build_ms = upstream_started.elapsed().as_millis() as i64;
                match result {
                    Ok(result) => result,
                    Err(_) => Err(timeout_error(timeout_stage(entry))),
                }
            }
        }
    };
    let response = match result {
        Ok(task) => {
            entry.upstream_stage = "magic_complete".into();
            let images: Vec<_> = task
                .urls
                .into_iter()
                .map(|url| crate::openai_image::UploadedImage {
                    url: build_openai_image_output_url(&state.config, "direct", &url),
                })
                .collect();
            let created = task.created.unwrap_or_else(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64
            });
            Json(crate::openai_image::build_response_payload_from_uploaded(
                &images, created,
            ))
            .into_response()
        }
        Err(err) => error_response(state, resolved, err, entry, cache_key.as_ref()).await,
    };
    (response, pending.entry.take().unwrap())
}

async fn execute(
    state: &AppState,
    resolved: &ResolvedUpstream,
    prepared: &PreparedRequest,
    entry: &mut AdminLogEntry,
    deadline: tokio::time::Instant,
) -> std::result::Result<CompletedTask, Error> {
    let mut resources = reference::RequestBlobs::new(&state.blob_runtime);
    let mut payload = prepared.payload.clone();
    let mut logged = payload.clone();
    let mut replacements = Vec::new();
    let mut images = Vec::new();
    let mut summaries = Vec::new();
    let prepare_started = Instant::now();
    // 逐张准备以限制解码峰值；占用既有全局槽，顺序和重复项均保留。
    for (index, url) in prepared.reference_urls.iter().enumerate() {
        entry.upstream_stage = "magic_prepare_image".into();
        let result = tokio::time::timeout_at(
            deadline,
            reference::prepare(
                &state.image_client,
                &state.blob_runtime,
                &mut resources,
                url,
            ),
        )
        .await
        .unwrap_or_else(|_| Err(timeout_error("magic_prepare_image")));
        entry.request_image_prepare_ms = prepare_started.elapsed().as_millis() as i64;
        let image = result.map_err(|mut error| {
            error.message = format!("参考图片 {}：{}", index + 1, error.message);
            error
        })?;
        summaries.push(format!(
            "[base64 omitted mime={} bytes={}]",
            image.meta().mime_type,
            image.meta().size_bytes
        ));
        images.push(image.clone());
        replacements.push(RequestReplacement {
            json_pointer: format!("/base64Array/{index}"),
            mime_type: image.meta().mime_type.clone(),
            blob: image,
        });
    }
    if !images.is_empty() {
        payload["base64Array"] = json!(vec![Value::Null; images.len()]);
        logged["base64Array"] = json!(summaries);
    }
    if state.admin.is_some() {
        entry.request_upstream = admin::truncate_for_admin_log(
            &redact(
                &serde_json::to_string_pretty(&logged).unwrap(),
                resolved,
                &state.config,
            ),
            admin::ADMIN_MAX_BODY_BYTES_PER_ENTRY,
        );
    }
    entry.upstream_stage = "magic_encode_request".into();
    let started = Instant::now();
    let result = encode_data_uri_request_body(payload, replacements, &state.blob_runtime).await;
    entry.request_encode_ms = started.elapsed().as_millis() as i64;
    let encoded = result.map_err(encode_error)?;
    resources.track(encoded.body_blob.clone());
    for image in images {
        resources.release(&image).await.map_err(encode_error)?;
    }
    let reader = state
        .blob_runtime
        .open_reader(&encoded.body_blob)
        .await
        .map_err(encode_error)?;
    entry.upstream_stage = "magic_submit".into();
    let submit_url = task_url(&resolved.base_url, prepared.mode, None)?;
    // 不可重放的请求体阻止 HTTP 客户端自动重新提交计费任务。
    let body = reqwest::Body::wrap_stream(ReaderStream::new(reader));
    let request = state
        .upstream_client
        .post(submit_url)
        .bearer_auth(&resolved.api_key)
        .header(CONTENT_TYPE, "application/json")
        .header(axum::http::header::CONTENT_LENGTH, encoded.content_length)
        .body(body);
    let submission = exchange(request, "magic_submit", entry)
        .await
        .map_err(|failure| failure.error)?;
    let task_id = crate::midjourney::submitted_task(&submission)?;
    entry.upstream_task_id = redact(&task_id, resolved, &state.config);
    entry.upstream_stage = "magic_poll".into();
    resources.cleanup().await;
    drop(encoded);
    let poll_url = task_url(&resolved.base_url, prepared.mode, Some(&task_id))?;
    let mut failures = 0;
    loop {
        let request = state
            .upstream_client
            .get(poll_url.clone())
            .bearer_auth(&resolved.api_key);
        match exchange(request, "magic_poll", entry).await {
            Ok(body) => {
                failures = 0;
                if let Some(task) = crate::midjourney::task_result(&body)? {
                    return Ok(task);
                }
            }
            Err(failure) => {
                failures += 1;
                if !failure.retryable || failures >= MAX_CONSECUTIVE_FAILURES {
                    return Err(failure.error);
                }
            }
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

fn task_url(base_url: &str, mode: Mode, task_id: Option<&str>) -> std::result::Result<Url, Error> {
    let mut url = Url::parse(base_url)
        .map_err(|_| Error::protocol("magic_submit", MSG_UPSTREAM_URL_INVALID))?;
    url.set_query(None);
    url.set_fragment(None);
    url.set_path(if task_id.is_some() {
        "/mj/task/"
    } else {
        mode.submit_path()
    });
    if let Some(task_id) = task_id {
        url.path_segments_mut()
            .map_err(|_| Error::protocol("magic_poll", MSG_UPSTREAM_URL_INVALID))?
            .pop_if_empty()
            .push(task_id)
            .push("fetch");
    }
    Ok(url)
}

struct ExchangeFailure {
    error: Error,
    retryable: bool,
}

async fn exchange(
    request: reqwest::RequestBuilder,
    stage: &'static str,
    entry: &mut AdminLogEntry,
) -> std::result::Result<Value, ExchangeFailure> {
    let mut response = request
        .send()
        .await
        .map_err(|err| transport_error(err, stage))?;
    let status = response.status();
    entry.upstream_status_code = status.as_u16();
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| transport_error(err, stage))?
    {
        if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(ExchangeFailure {
                error: Error::protocol(stage, "Magic 任务响应过大，无法处理"),
                retryable: false,
            });
        }
        bytes.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        let body = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"description": String::from_utf8_lossy(&bytes)}));
        let error = Error::upstream(status, stage, &body);
        let retryable = is_aiapidev_retryable_poll_status(status)
            && error.status != StatusCode::BAD_REQUEST
            && classify_blockable_upstream_error(status, &bytes).is_none();
        return Err(ExchangeFailure { error, retryable });
    }
    serde_json::from_slice(&bytes).map_err(|_| ExchangeFailure {
        error: Error::protocol(stage, "Magic 任务响应不是有效 JSON"),
        retryable: false,
    })
}

fn transport_error(err: reqwest::Error, stage: &'static str) -> ExchangeFailure {
    let class = classify_reqwest_proxy_error(&err);
    let retryable = !err.is_builder() && !err.is_redirect() && class.kind != "upstream_tls_failed";
    let mut error = Error::new(
        if err.is_timeout() {
            StatusCode::GATEWAY_TIMEOUT
        } else {
            StatusCode::BAD_GATEWAY
        },
        "proxy",
        stage,
        class.kind,
        class.message,
    );
    error.detail = Some(reqwest_error_detail(&err.without_url()));
    ExchangeFailure { error, retryable }
}

fn timeout_error(stage: &'static str) -> Error {
    Error::new(
        StatusCode::GATEWAY_TIMEOUT,
        "proxy",
        stage,
        "timeout",
        if matches!(stage, "magic_prepare_image" | "magic_encode_request") {
            "准备 Magic 参考图片或请求超时，请检查图片源并重试"
        } else {
            "等待 Magic 任务完成超时，请稍后查询已有任务"
        },
    )
}

fn timeout_stage(entry: &AdminLogEntry) -> &'static str {
    match entry.upstream_stage.as_str() {
        "magic_prepare_image" => "magic_prepare_image",
        "magic_encode_request" => "magic_encode_request",
        "magic_poll" => "magic_poll",
        _ => "magic_submit",
    }
}

fn encode_error(error: anyhow::Error) -> Error {
    let mut result = Error::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "proxy",
        "magic_encode_request",
        "request_encode_failed",
        "Magic 提交请求编码失败，请稍后重试",
    );
    result.detail = Some(error.to_string());
    result
}

fn redact(value: &str, resolved: &ResolvedUpstream, config: &Config) -> String {
    [&resolved.api_key, &config.upstream_api_key]
        .into_iter()
        .filter(|key| !key.is_empty())
        .fold(value.to_string(), |text, key| {
            text.replace(key, "[redacted]")
        })
}

async fn error_response(
    state: &AppState,
    resolved: &ResolvedUpstream,
    err: Error,
    entry: &mut AdminLogEntry,
    cache_key: Option<&UpstreamBlockCacheKey>,
) -> Response {
    entry.upstream_stage = err.stage.into();
    let mut error = json!({
        "code": err.status.as_u16(),
        "message": redact(&err.message, resolved, &state.config),
        "source": err.source, "stage": err.stage, "kind": err.kind,
    });
    if let Some(code) = err.business_code {
        error["business_code"] = json!(code);
    }
    if let Some(reason) = err.fail_reason {
        error["failReason"] = json!(redact(&reason, resolved, &state.config));
    }
    if !entry.upstream_task_id.is_empty() {
        error["task_id"] = json!(entry.upstream_task_id);
    }
    entry.error_detail = match err.detail {
        Some(detail) => format!("{error}; {}", redact(&detail, resolved, &state.config)),
        None => error.to_string(),
    };
    let mut response = upstream_block_cache_json_error_response(
        err.status,
        json!({"error": error}),
        state.upstream_block_cache.as_ref(),
        cache_key,
    )
    .await;
    if let Ok(status) = StatusCode::from_u16(entry.upstream_status_code) {
        response
            .extensions_mut()
            .insert(OriginalUpstreamStatus(status));
    }
    response
}
