use anyhow::{Result, anyhow};
use serde::Serialize;
use serde_json::{Map, Value, json};
use time::{PrimitiveDateTime, format_description::FormatItem, macros::format_description};
use url::Url;

const IMAGE_FIELD_ALIASES: [&str; 3] = ["reference_images", "images", "image"];
const AIAPIDEV_TIMESTAMP_FORMAT: &[FormatItem<'static>] =
    format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UploadedImage {
    pub url: String,
}

pub fn normalize_request_body(body: Value, force_b64_json: bool) -> Result<Value> {
    let mut object = match body {
        Value::Object(map) => map,
        _ => return Err(anyhow!("请求内容必须是 JSON 对象，请检查后再试")),
    };

    let response_format = object
        .get("response_format")
        .and_then(Value::as_str)
        .unwrap_or("url");
    if !response_format.eq_ignore_ascii_case("url") {
        return Err(anyhow!("response_format 只支持 url，请调整后再试"));
    }

    let image_field = collect_reference_images(&mut object)?;
    if let Some((field_name, images)) = image_field {
        object.insert(
            field_name.to_string(),
            Value::Array(images.into_iter().map(Value::String).collect()),
        );
    }
    object.insert(
        "response_format".to_string(),
        Value::String(if force_b64_json {
            "b64_json".to_string()
        } else {
            "url".to_string()
        }),
    );

    Ok(Value::Object(object))
}

pub fn build_fixed_usage() -> Value {
    json!({
        "input_tokens": 1024,
        "input_tokens_details": {
            "image_tokens": 1000,
            "text_tokens": 24
        },
        "output_tokens": 1024,
        "total_tokens": 2048,
        "output_tokens_details": {
            "image_tokens": 1024,
            "text_tokens": 0
        }
    })
}

pub fn build_response_payload(
    upstream_body: Value,
    uploaded: &[UploadedImage],
    fallback_created: i64,
) -> Result<Value> {
    let created = extract_created_timestamp(&upstream_body, fallback_created);

    Ok(json!({
        "created": created,
        "data": uploaded,
        "usage": build_fixed_usage(),
    }))
}

pub fn build_response_payload_from_uploaded(uploaded: &[UploadedImage], created: i64) -> Value {
    json!({
        "created": created,
        "data": uploaded,
        "usage": build_fixed_usage(),
    })
}

pub fn extract_created_timestamp(upstream_body: &Value, fallback_created: i64) -> i64 {
    if let Some(created) = upstream_body.get("created").and_then(Value::as_i64) {
        return created;
    }

    for key in ["finishedTime", "submittedTime", "createTime"] {
        let Some(raw) = upstream_body.get(key).and_then(Value::as_str) else {
            continue;
        };
        if let Some(created) = parse_aiapidev_timestamp(raw) {
            return created;
        }
    }

    fallback_created
}

fn collect_reference_images(
    object: &mut Map<String, Value>,
) -> Result<Option<(&'static str, Vec<String>)>> {
    let target_alias = IMAGE_FIELD_ALIASES
        .iter()
        .rev()
        .find(|alias| object.contains_key(**alias))
        .copied();
    let mut images = Vec::new();

    for alias in IMAGE_FIELD_ALIASES {
        let Some(value) = object.remove(alias) else {
            continue;
        };

        let array = value
            .as_array()
            .ok_or_else(|| anyhow!("{alias} 必须是数组，请检查后再试"))?;
        for item in array {
            let raw = item
                .as_str()
                .ok_or_else(|| anyhow!("{alias} 中的图片地址必须是字符串"))?;
            validate_reference_image_url(raw)?;
            images.push(raw.to_string());
        }
    }

    Ok(target_alias.map(|alias| (alias, images)))
}

fn validate_reference_image_url(raw: &str) -> Result<()> {
    let parsed = Url::parse(raw).map_err(|_| anyhow!("参考图片必须是完整 URL"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(anyhow!("参考图片 URL 必须使用 http 或 https"));
    }
    if parsed.host_str().unwrap_or_default().trim().is_empty() {
        return Err(anyhow!("参考图片 URL 缺少主机名，请检查后再试"));
    }
    Ok(())
}

fn parse_aiapidev_timestamp(raw: &str) -> Option<i64> {
    let parsed = PrimitiveDateTime::parse(raw.trim(), AIAPIDEV_TIMESTAMP_FORMAT).ok()?;
    Some(parsed.assume_utc().unix_timestamp())
}
