use anyhow::{Result, anyhow};
use serde::Serialize;
use serde_json::{Map, Value, json};
use time::{PrimitiveDateTime, format_description::FormatItem, macros::format_description};
use url::Url;

const IMAGE_FIELD_ALIASES: [&str; 3] = ["reference_images", "images", "image"];
const API888_SEEDREAM_SIZES: [(&str, &str); 8] = [
    ("1:1", "2048x2048"),
    ("3:4", "1728x2304"),
    ("16:9", "2560x1440"),
    ("4:3", "2304x1728"),
    ("9:16", "1440x2560"),
    ("2:3", "1664x2496"),
    ("3:2", "2496x1664"),
    ("21:9", "3024x1296"),
];
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

pub fn normalize_request_body_for_upstream(
    body: Value,
    force_b64_json: bool,
    base_url: &str,
) -> Result<Value> {
    let mut body = normalize_request_body(body, force_b64_json)?;
    let is_tianyue = crate::provider::is_tianyue_base_url(base_url);
    let is_ai666_seedream = is_ai666_seedream_55_request(base_url, &body);
    let is_api888_seedream = is_api888_seedream_55_request(base_url, &body);
    if (is_tianyue || is_ai666_seedream || is_api888_seedream)
        && let Some(object) = body.as_object_mut()
    {
        if is_tianyue && object.get("model").and_then(Value::as_str) == Some("seedream-5.5") {
            object.insert(
                "model".to_string(),
                Value::String("GZ-seedream-5.5-2K".to_string()),
            );
        }
        if is_tianyue
            && object.get("model").and_then(Value::as_str) == Some("seedream-5-pro")
            && let Some(size) = object.get("size").and_then(Value::as_str)
            && (size.eq_ignore_ascii_case("1K") || size.eq_ignore_ascii_case("2K"))
        {
            let model = format!("seedream-5-pro-{}", size.to_ascii_uppercase());
            object.insert("model".to_string(), Value::String(model));
        }
        for alias in ["image", "reference_images"] {
            if let Some(images) = object.remove(alias) {
                object.insert("images".to_string(), images);
            }
        }
        if is_ai666_seedream {
            normalize_ai666_seedream_55_request(object)?;
        }
        if is_api888_seedream {
            normalize_api888_seedream_55_request(object)?;
        }
    }
    Ok(body)
}

pub(crate) fn is_ai666_seedream_55_request(base_url: &str, body: &Value) -> bool {
    body.get("model").and_then(Value::as_str) == Some("seedream-5.5")
        && crate::provider::is_ai666_base_url(base_url)
}

pub(crate) fn is_api888_seedream_55_request(base_url: &str, body: &Value) -> bool {
    body.get("model").and_then(Value::as_str) == Some("seedream-5.5")
        && crate::provider::is_api888_base_url(base_url)
}

fn normalize_api888_seedream_55_request(object: &mut Map<String, Value>) -> Result<()> {
    if object
        .get("prompt")
        .and_then(Value::as_str)
        .is_none_or(|prompt| prompt.trim().is_empty())
    {
        return Err(anyhow!("prompt 必须是非空字符串"));
    }
    if object.get("n").is_some_and(|n| n.as_u64() != Some(1)) {
        return Err(anyhow!("888API Seedream 5.5 的 n 必须为整数 1"));
    }
    if let Some(images) = object.get("images").and_then(Value::as_array) {
        if images.is_empty() {
            object.remove("images");
        } else if images.iter().any(|image| {
            !image
                .as_str()
                .and_then(|url| Url::parse(url).ok())
                .is_some_and(|url| url.scheme() == "https")
        }) {
            return Err(anyhow!("888API 参考图片 URL 必须使用 HTTPS"));
        }
    }

    let size = api888_seedream_size(object)?;
    object.remove("aspect_ratio");
    for field in ["resolution", "output_resolution"] {
        if let Some(value) = object.remove(field)
            && !value
                .as_str()
                .map(str::trim)
                .is_some_and(is_legacy_seedream_size)
        {
            return Err(anyhow!("888API Seedream 5.5 的 {field} 只接受 1K 或 2K"));
        }
    }
    if let Some(watermark) = object.remove("watermark")
        && watermark.as_bool() != Some(false)
    {
        return Err(anyhow!("888API Seedream 5.5 不支持设置水印"));
    }
    if object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "model"
                | "prompt"
                | "size"
                | "n"
                | "images"
                | "response_format"
                | "background"
                | "output_format"
        )
    }) {
        return Err(anyhow!("请求包含 888API Seedream 5.5 不支持的字段"));
    }
    object.insert("size".to_string(), Value::String(size.to_string()));
    object.entry("n".to_string()).or_insert(Value::from(1));
    Ok(())
}

fn api888_seedream_size(object: &Map<String, Value>) -> Result<&'static str> {
    let pixels = match object.get("size") {
        None => None,
        Some(value) => {
            let size = value
                .as_str()
                .ok_or_else(|| anyhow!("size 必须是字符串"))?
                .trim();
            (!is_legacy_seedream_size(size)).then_some(size)
        }
    };
    let ratio = match object.get("aspect_ratio") {
        Some(value) => Some(
            value
                .as_str()
                .ok_or_else(|| anyhow!("aspect_ratio 必须是字符串"))?
                .trim(),
        ),
        None if pixels.is_none() => Some("1:1"),
        None => None,
    };
    API888_SEEDREAM_SIZES
        .iter()
        .find_map(|&(supported_ratio, supported_size)| {
            (ratio.is_none_or(|ratio| ratio == supported_ratio)
                && pixels.is_none_or(|size| size.eq_ignore_ascii_case(supported_size)))
            .then_some(supported_size)
        })
        .ok_or_else(|| anyhow!("size 和 aspect_ratio 必须对应 888API 支持的 2K 尺寸"))
}

fn is_legacy_seedream_size(size: &str) -> bool {
    size.eq_ignore_ascii_case("1K") || size.eq_ignore_ascii_case("2K")
}

fn normalize_ai666_seedream_55_request(object: &mut Map<String, Value>) -> Result<()> {
    if object
        .get("prompt")
        .and_then(Value::as_str)
        .is_none_or(|prompt| prompt.trim().is_empty())
    {
        return Err(anyhow!("prompt 必须是非空字符串"));
    }
    if object.get("n").is_some_and(|n| n.as_u64() != Some(1)) {
        return Err(anyhow!("AI666 Seedream 5.5 每次只生成一张图片，n 必须为 1"));
    }
    if object
        .get("images")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
    {
        object.remove("images");
    }

    // 上游将清晰度和尺寸分开，保留像素尺寸或比例形式的 size。
    let size_resolution = match object.get("size") {
        Some(Value::String(size)) if size.trim_end().ends_with(['K', 'k']) => object.remove("size"),
        Some(Value::String(_)) | None => None,
        Some(_) => return Err(anyhow!("size 必须是字符串")),
    };
    let output_resolution = object.remove("output_resolution");
    if let Some(resolution) = object
        .get("resolution")
        .or(size_resolution.as_ref())
        .or(output_resolution.as_ref())
    {
        let resolution = match resolution.as_str().map(str::trim) {
            Some(value) if value.eq_ignore_ascii_case("1K") => "1K",
            Some(value) if value.eq_ignore_ascii_case("2K") => "2K",
            _ => return Err(anyhow!("AI666 Seedream 5.5 分辨率只支持 1K 或 2K")),
        };
        object.insert(
            "resolution".to_string(),
            Value::String(resolution.to_string()),
        );
    }
    Ok(())
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
