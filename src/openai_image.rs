use anyhow::{Result, anyhow};
use serde::Serialize;
use serde_json::{Map, Value, json};
use url::Url;

const IMAGE_FIELD_ALIASES: [&str; 3] = ["reference_images", "images", "image"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UploadedImage {
    pub url: String,
}

pub fn normalize_request_body(body: Value) -> Result<Value> {
    let mut object = match body {
        Value::Object(map) => map,
        _ => return Err(anyhow!("request body must be a json object")),
    };

    let response_format = object
        .get("response_format")
        .and_then(Value::as_str)
        .unwrap_or("url");
    if !response_format.eq_ignore_ascii_case("url") {
        return Err(anyhow!("response_format must be url when provided"));
    }

    let reference_images = collect_reference_images(&mut object)?;
    object.insert(
        "reference_images".to_string(),
        Value::Array(reference_images.into_iter().map(Value::String).collect()),
    );
    object.insert(
        "response_format".to_string(),
        Value::String("b64_json".to_string()),
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
    let created = upstream_body
        .get("created")
        .and_then(Value::as_i64)
        .unwrap_or(fallback_created);

    Ok(json!({
        "created": created,
        "data": uploaded,
        "usage": build_fixed_usage(),
    }))
}

fn collect_reference_images(object: &mut Map<String, Value>) -> Result<Vec<String>> {
    let mut images = Vec::new();

    for alias in IMAGE_FIELD_ALIASES {
        let Some(value) = object.remove(alias) else {
            continue;
        };

        let array = value
            .as_array()
            .ok_or_else(|| anyhow!("{alias} must be an array"))?;
        for item in array {
            let raw = item
                .as_str()
                .ok_or_else(|| anyhow!("{alias} items must be strings"))?;
            validate_reference_image_url(raw)?;
            images.push(raw.to_string());
        }
    }

    Ok(images)
}

fn validate_reference_image_url(raw: &str) -> Result<()> {
    let parsed = Url::parse(raw).map_err(|_| anyhow!("reference image must be an absolute url"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(anyhow!("reference image must use http or https"));
    }
    if parsed.host_str().unwrap_or_default().trim().is_empty() {
        return Err(anyhow!("reference image host is required"));
    }
    Ok(())
}
