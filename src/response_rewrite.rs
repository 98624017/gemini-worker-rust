use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;

use crate::cache::InlineDataUrlFetchService;
use crate::config::Config;
use crate::image_io::{DEFAULT_MAX_IMAGE_BYTES, fetch_image_as_inline_data};
use crate::upload::Uploader;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputMode {
    Base64,
    Url,
}

pub fn normalize_gemini_response(mut body: Value) -> Value {
    remove_thought_signatures(&mut body);
    keep_largest_inline_image(body)
}

pub async fn normalize_special_markdown_image_response(
    mut body: Value,
    output_mode: OutputMode,
    image_client: &reqwest::Client,
    fetch_service: Option<&Arc<InlineDataUrlFetchService>>,
    config: &Config,
) -> anyhow::Result<Value> {
    if contains_inline_data(&body) {
        return Ok(body);
    }

    let Some(parts) = body
        .pointer("/candidates/0/content/parts")
        .and_then(Value::as_array)
        .cloned()
    else {
        return Ok(body);
    };

    let Some(image_url) = extract_first_markdown_image_url_from_parts(&parts) else {
        return Ok(body);
    };

    let inline_data = match output_mode {
        OutputMode::Url => {
            let data = if config.proxy_special_upstream_urls
                && !config.public_base_url.trim().is_empty()
            {
                wrap_proxy_url_encoded(&config.public_base_url, &image_url)
            } else {
                image_url.clone()
            };
            serde_json::json!({
                "mimeType": guess_image_mime_type_from_url(&image_url),
                "data": data,
            })
        }
        OutputMode::Base64 => {
            let fetched = if let Some(fetch_service) = fetch_service {
                let fetched = fetch_service.fetch(&image_url).await?;
                crate::image_io::FetchedInlineData {
                    mime_type: fetched.mime_type,
                    bytes: fetched.bytes,
                }
            } else {
                fetch_image_as_inline_data(image_client, &image_url, DEFAULT_MAX_IMAGE_BYTES)
                    .await?
            };
            serde_json::json!({
                "mimeType": fetched.mime_type,
                "data": STANDARD.encode(fetched.bytes),
            })
        }
    };

    if let Some(target) = body.pointer_mut("/candidates/0/content/parts") {
        *target = Value::Array(vec![serde_json::json!({"inlineData": inline_data})]);
    }
    Ok(body)
}

pub async fn rewrite_inline_data_base64_to_urls(
    mut body: Value,
    uploader: &Uploader,
    public_base_url: &str,
    wrap_legacy_urls: bool,
) -> Value {
    let entries = scan_inline_data_base64_entries(&body);
    let mut replacements = HashMap::new();

    for entry in entries {
        if let Ok(image_bytes) = STANDARD.decode(entry.data.as_bytes()) {
            if let Ok(upload_result) = uploader.upload_image(&image_bytes, &entry.mime_type).await {
                let final_url = if upload_result.provider == "legacy"
                    && wrap_legacy_urls
                    && !public_base_url.trim().is_empty()
                {
                    crate::upload::wrap_proxy_url(public_base_url, &upload_result.url)
                } else {
                    upload_result.url
                };
                replacements.insert(entry, final_url);
            }
        }
    }

    patch_inline_data_urls(&mut body, &replacements);
    body
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct InlineDataEntry {
    mime_type: String,
    data: String,
}

pub fn remove_thought_signatures(node: &mut Value) {
    match node {
        Value::Object(map) => {
            map.remove("thoughtSignature");
            for child in map.values_mut() {
                remove_thought_signatures(child);
            }
        }
        Value::Array(items) => {
            for child in items {
                remove_thought_signatures(child);
            }
        }
        _ => {}
    }
}

fn is_url_like(value: &str) -> bool {
    value.starts_with("http://")
        || value.starts_with("https://")
        || value.starts_with("/proxy/image")
}

fn scan_inline_data_base64_entries(node: &Value) -> Vec<InlineDataEntry> {
    let mut entries = Vec::new();

    fn walk(node: &Value, entries: &mut Vec<InlineDataEntry>) {
        match node {
            Value::Object(map) => {
                if let Some(Value::Object(inline_data)) = map.get("inlineData") {
                    if let (Some(Value::String(data)), Some(Value::String(mime_type))) =
                        (inline_data.get("data"), inline_data.get("mimeType"))
                    {
                        if !is_url_like(data) {
                            let entry = InlineDataEntry {
                                mime_type: mime_type.clone(),
                                data: data.clone(),
                            };
                            if !entries.contains(&entry) {
                                entries.push(entry);
                            }
                        }
                    }
                }

                for child in map.values() {
                    walk(child, entries);
                }
            }
            Value::Array(items) => {
                for child in items {
                    walk(child, entries);
                }
            }
            _ => {}
        }
    }

    walk(node, &mut entries);
    entries
}

fn patch_inline_data_urls(node: &mut Value, replacements: &HashMap<InlineDataEntry, String>) {
    match node {
        Value::Object(map) => {
            if let Some(Value::Object(inline_data)) = map.get_mut("inlineData") {
                if let (Some(Value::String(data)), Some(Value::String(mime_type))) =
                    (inline_data.get("data"), inline_data.get("mimeType"))
                {
                    let entry = InlineDataEntry {
                        mime_type: mime_type.clone(),
                        data: data.clone(),
                    };
                    if let Some(url) = replacements.get(&entry) {
                        inline_data.insert("data".to_string(), Value::String(url.clone()));
                    }
                }
            }

            for child in map.values_mut() {
                patch_inline_data_urls(child, replacements);
            }
        }
        Value::Array(items) => {
            for child in items {
                patch_inline_data_urls(child, replacements);
            }
        }
        _ => {}
    }
}

fn contains_inline_data(node: &Value) -> bool {
    match node {
        Value::Object(map) => {
            map.contains_key("inlineData") || map.values().any(contains_inline_data)
        }
        Value::Array(items) => items.iter().any(contains_inline_data),
        _ => false,
    }
}

fn extract_first_markdown_image_url_from_parts(parts: &[Value]) -> Option<String> {
    for part in parts {
        let Some(text) = part.get("text").and_then(Value::as_str) else {
            continue;
        };
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        if let Some(url) = extract_markdown_image_url(text) {
            return Some(url);
        }
    }
    None
}

fn extract_markdown_image_url(text: &str) -> Option<String> {
    let start = text.find("](")?;
    let rest = &text[start + 2..];
    let end = rest.find(')')?;
    let candidate = rest[..end].trim();
    if candidate.starts_with("http://") || candidate.starts_with("https://") {
        return Some(candidate.to_string());
    }
    None
}

fn wrap_proxy_url_encoded(public_base_url: &str, target_url: &str) -> String {
    let base = public_base_url.trim().trim_end_matches('/');
    let encoded = URL_SAFE_NO_PAD.encode(target_url.as_bytes());
    format!("{base}/proxy/image?u={encoded}")
}

fn guess_image_mime_type_from_url(raw_url: &str) -> &'static str {
    let lower = raw_url.to_ascii_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        return "image/jpeg";
    }
    if lower.ends_with(".webp") {
        return "image/webp";
    }
    if lower.ends_with(".gif") {
        return "image/gif";
    }
    "image/png"
}

pub fn keep_largest_inline_image(mut body: Value) -> Value {
    let Some(candidates) = body.get_mut("candidates").and_then(Value::as_array_mut) else {
        return body;
    };

    for candidate in candidates {
        let Some(parts) = candidate
            .get_mut("content")
            .and_then(Value::as_object_mut)
            .and_then(|content| content.get_mut("parts"))
            .and_then(Value::as_array_mut)
        else {
            continue;
        };

        let mut best_index = None;
        let mut best_size = 0usize;

        for (index, part) in parts.iter().enumerate() {
            let Some(inline_data) = part.get("inlineData").and_then(Value::as_object) else {
                continue;
            };
            let Some(data) = inline_data.get("data").and_then(Value::as_str) else {
                continue;
            };
            if data.starts_with("http://")
                || data.starts_with("https://")
                || data.starts_with("/proxy/image")
            {
                continue;
            }
            if data.len() > best_size {
                best_size = data.len();
                best_index = Some(index);
            }
        }

        if let Some(best_index) = best_index {
            let mut retained = Vec::with_capacity(parts.len());
            for (index, part) in parts.iter().enumerate() {
                let is_inline_image = part
                    .get("inlineData")
                    .and_then(Value::as_object)
                    .and_then(|inline_data| inline_data.get("data"))
                    .and_then(Value::as_str)
                    .is_some();

                if !is_inline_image || index == best_index {
                    retained.push(part.clone());
                }
            }
            *parts = retained;
        }
    }

    body
}
