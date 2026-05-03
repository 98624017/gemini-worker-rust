use axum::http::StatusCode;
use serde_json::{Map, Value, json};
use url::form_urlencoded;

pub const IMAGE_GENERATION_PATH: &str = "/v1/draw/nano-banana";
pub const DEFAULT_MODEL: &str = "nano-banana-fast";
const MSG_PARSE_UPSTREAM_FAILED: &str = "解析上游服务响应失败，请稍后再试";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrsaiSource {
    Gemini,
    OpenAi,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrsaiImageParams {
    pub model: String,
    pub prompt: String,
    pub urls: Vec<String>,
    pub aspect_ratio: String,
    pub image_size: String,
    pub output: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GrsaiResult {
    pub status: String,
    pub image_urls: Vec<String>,
    pub failure_reason: Option<String>,
    pub error_detail: Option<String>,
    pub start_time: Option<i64>,
    pub end_time: Option<i64>,
    pub raw_data: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GrsaiError {
    pub http_status: StatusCode,
    pub message: String,
    pub upstream_code: Option<i64>,
    pub failure_reason: Option<String>,
    pub body_text: String,
    pub raw_json: Option<Value>,
}

pub fn normalize_model(raw_model: &str, source: GrsaiSource) -> String {
    let model = raw_model.trim();
    if model.is_empty() {
        return DEFAULT_MODEL.to_string();
    }
    match source {
        GrsaiSource::Gemini => match model {
            "gemini-3-pro-image-preview" => "nano-banana-pro".to_string(),
            "gemini-2.5-flash-image" => DEFAULT_MODEL.to_string(),
            "gemini-3.1-flash-image-preview" => "nano-banana-2".to_string(),
            _ => model.to_string(),
        },
        GrsaiSource::OpenAi => model.to_string(),
    }
}

pub fn extract_gemini_params(
    body: &Value,
    raw_model: &str,
    query: Option<&str>,
) -> Result<GrsaiImageParams, GrsaiError> {
    let object = body
        .as_object()
        .ok_or_else(|| invalid_request("请求内容必须是 JSON 对象"))?;
    let content = pick_gemini_content(object)
        .ok_or_else(|| invalid_request("Gemini 请求缺少 contents 字段或内容为空"))?;

    let parts = content
        .get("parts")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_request("Gemini 请求缺少 parts 数组"))?;

    let prompt = parts
        .iter()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let urls = parts
        .iter()
        .filter_map(extract_part_url)
        .collect::<Vec<String>>();

    let image_config = body
        .pointer("/generationConfig/imageConfig")
        .or_else(|| body.pointer("/generation_config/image_config"));
    let query_map = parse_query_map(query);

    let aspect_ratio = pick_string_override(
        &query_map,
        &["aspectRatio", "aspect_ratio"],
        image_config,
        &["aspectRatio", "aspect_ratio"],
        "auto",
    );
    let image_size = pick_string_override(
        &query_map,
        &["imageSize", "image_size"],
        image_config,
        &["imageSize", "image_size"],
        "1K",
    );
    let output = pick_string_override(&query_map, &["output"], image_config, &["output"], "");

    Ok(GrsaiImageParams {
        model: normalize_model(raw_model, GrsaiSource::Gemini),
        prompt,
        urls,
        aspect_ratio,
        image_size,
        output,
    })
}

pub fn extract_openai_params(body: &Value) -> Result<GrsaiImageParams, GrsaiError> {
    let object = body
        .as_object()
        .ok_or_else(|| invalid_request("请求内容必须是 JSON 对象"))?;

    let model = object
        .get("model")
        .and_then(Value::as_str)
        .or_else(|| object.get("model_name").and_then(Value::as_str))
        .unwrap_or_default();
    let prompt = object
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let urls = object
        .get("urls")
        .or_else(|| object.get("images"))
        .map(extract_string_array)
        .transpose()?
        .unwrap_or_default();

    Ok(GrsaiImageParams {
        model: normalize_model(model, GrsaiSource::OpenAi),
        prompt,
        urls,
        aspect_ratio: pick_object_string(object, &["aspect_ratio", "aspectRatio"], "auto"),
        image_size: pick_object_string(object, &["image_size", "imageSize"], "1K"),
        output: pick_object_string(object, &["output"], "url"),
    })
}

pub fn build_grsai_request_body(params: &GrsaiImageParams) -> Value {
    json!({
        "model": params.model,
        "prompt": params.prompt,
        "urls": params.urls,
        "aspectRatio": params.aspect_ratio,
        "imageSize": params.image_size,
        "shutProgress": true,
    })
}

pub fn parse_grsai_response(status: StatusCode, body: &[u8]) -> Result<GrsaiResult, GrsaiError> {
    let body_text = String::from_utf8_lossy(body).into_owned();
    let payload_text = extract_payload_text(body, &body_text)?;
    let raw_json: Value = serde_json::from_str(&payload_text).map_err(|err| GrsaiError {
        http_status: StatusCode::BAD_GATEWAY,
        message: format!("{MSG_PARSE_UPSTREAM_FAILED}: {err}"),
        upstream_code: None,
        failure_reason: None,
        body_text: body_text.clone(),
        raw_json: None,
    })?;

    if !status.is_success() {
        return Err(build_http_error(status, &body_text, raw_json));
    }

    let (payload, code, _message) = unwrap_payload(raw_json, &body_text)?;
    let Some(object) = payload.as_object() else {
        return Err(GrsaiError {
            http_status: StatusCode::BAD_GATEWAY,
            message: format!("{MSG_PARSE_UPSTREAM_FAILED}: 响应 data 不是对象"),
            upstream_code: code,
            failure_reason: None,
            body_text: body_text.clone(),
            raw_json: Some(payload),
        });
    };

    let status_text = object
        .get("status")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| GrsaiError {
            http_status: StatusCode::BAD_GATEWAY,
            message: format!("{MSG_PARSE_UPSTREAM_FAILED}: 缺少 status 字段"),
            upstream_code: code,
            failure_reason: object_string(object, &["failure_reason"]),
            body_text: body_text.clone(),
            raw_json: Some(payload.clone()),
        })?;

    let image_urls = object
        .get("results")
        .and_then(Value::as_array)
        .map(|results| {
            results
                .iter()
                .filter_map(|item| item.get("url").and_then(Value::as_str))
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(GrsaiResult {
        status: status_text.to_string(),
        image_urls,
        failure_reason: object_string(object, &["failure_reason"]),
        error_detail: object_string(object, &["error_detail", "errorDetail", "detail"]),
        start_time: object.get("start_time").and_then(Value::as_i64),
        end_time: object.get("end_time").and_then(Value::as_i64),
        raw_data: payload,
    })
}

fn pick_gemini_content<'a>(object: &'a Map<String, Value>) -> Option<&'a Value> {
    let contents = object.get("contents")?.as_array()?;
    contents
        .iter()
        .find(|content| content.get("role").and_then(Value::as_str) == Some("user"))
        .or_else(|| contents.first())
}

fn extract_part_url(part: &Value) -> Option<String> {
    part.pointer("/inlineData/data")
        .or_else(|| part.pointer("/inline_data/data"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn parse_query_map(query: Option<&str>) -> Vec<(String, String)> {
    query
        .map(|value| {
            form_urlencoded::parse(value.as_bytes())
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect()
        })
        .unwrap_or_default()
}

fn pick_string_override(
    query_map: &[(String, String)],
    query_keys: &[&str],
    object: Option<&Value>,
    object_keys: &[&str],
    default: &str,
) -> String {
    for key in query_keys {
        if let Some((_, value)) = query_map
            .iter()
            .find(|(name, value)| name == key && !value.trim().is_empty())
        {
            return value.trim().to_string();
        }
    }
    object
        .and_then(|value| {
            object_keys
                .iter()
                .find_map(|key| value.get(*key).and_then(Value::as_str))
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default)
        .to_string()
}

fn pick_object_string(object: &Map<String, Value>, keys: &[&str], default: &str) -> String {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default)
        .to_string()
}

fn extract_string_array(value: &Value) -> Result<Vec<String>, GrsaiError> {
    let array = value
        .as_array()
        .ok_or_else(|| invalid_request("图片 URL 字段必须是字符串数组"))?;
    array
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .ok_or_else(|| invalid_request("图片 URL 字段必须是字符串数组"))
        })
        .collect()
}

fn extract_payload_text(body: &[u8], body_text: &str) -> Result<String, GrsaiError> {
    if body.is_empty() || body_text.trim().is_empty() {
        return Err(parse_error("响应体为空", body_text, None));
    }
    let trimmed = body_text.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return Ok(trimmed.to_string());
    }

    body_text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .filter(|line| !line.is_empty() && *line != "[DONE]")
        .next_back()
        .map(ToString::to_string)
        .ok_or_else(|| parse_error("未找到可解析的数据段", body_text, None))
}

fn unwrap_payload(
    raw_json: Value,
    body_text: &str,
) -> Result<(Value, Option<i64>, String), GrsaiError> {
    let Some(object) = raw_json.as_object() else {
        return Ok((raw_json, None, String::new()));
    };

    let code = object.get("code").and_then(Value::as_i64);
    let message = object
        .get("msg")
        .or_else(|| object.get("message"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    if let Some(code) = code {
        if code != 0 {
            let raw = Value::Object(object.clone());
            return Err(GrsaiError {
                http_status: guess_business_status(code, &message),
                message: if message.trim().is_empty() {
                    "上游服务返回业务错误".to_string()
                } else {
                    message
                },
                upstream_code: Some(code),
                failure_reason: object
                    .get("data")
                    .and_then(Value::as_object)
                    .and_then(|data| object_string(data, &["failure_reason"])),
                body_text: body_text.to_string(),
                raw_json: Some(raw),
            });
        }
        let data = object.get("data").cloned().ok_or_else(|| {
            parse_error(
                "缺少 data 字段",
                body_text,
                Some(Value::Object(object.clone())),
            )
        })?;
        return Ok((data, Some(code), message));
    }

    Ok((Value::Object(object.clone()), None, message))
}

fn build_http_error(status: StatusCode, body_text: &str, raw_json: Value) -> GrsaiError {
    let object = raw_json.as_object();
    let message = object
        .and_then(|map| map.get("msg").or_else(|| map.get("message")))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(status.canonical_reason().unwrap_or("upstream error"))
        .to_string();
    let failure_reason = object
        .and_then(|map| map.get("data"))
        .and_then(Value::as_object)
        .and_then(|data| object_string(data, &["failure_reason"]));
    let upstream_code = object.and_then(|map| map.get("code").and_then(Value::as_i64));

    GrsaiError {
        http_status: status,
        message,
        upstream_code,
        failure_reason,
        body_text: body_text.to_string(),
        raw_json: Some(raw_json),
    }
}

fn invalid_request(message: &str) -> GrsaiError {
    GrsaiError {
        http_status: StatusCode::BAD_REQUEST,
        message: message.to_string(),
        upstream_code: None,
        failure_reason: None,
        body_text: String::new(),
        raw_json: None,
    }
}

fn parse_error(message: &str, body_text: &str, raw_json: Option<Value>) -> GrsaiError {
    GrsaiError {
        http_status: StatusCode::BAD_GATEWAY,
        message: format!("{MSG_PARSE_UPSTREAM_FAILED}: {message}"),
        upstream_code: None,
        failure_reason: None,
        body_text: body_text.to_string(),
        raw_json,
    }
}

fn object_string(object: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn guess_business_status(code: i64, message: &str) -> StatusCode {
    match code {
        400 => StatusCode::BAD_REQUEST,
        401 => StatusCode::UNAUTHORIZED,
        403 => StatusCode::FORBIDDEN,
        404 => StatusCode::NOT_FOUND,
        409 => StatusCode::CONFLICT,
        422 => StatusCode::UNPROCESSABLE_ENTITY,
        429 => StatusCode::TOO_MANY_REQUESTS,
        _ => {
            let lower = message.to_ascii_lowercase();
            if lower.contains("api key")
                || lower.contains("unauthorized")
                || lower.contains("auth")
                || lower.contains("token")
                || lower.contains("invalid key")
            {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::BAD_GATEWAY
            }
        }
    }
}
