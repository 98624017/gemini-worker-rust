use std::collections::HashMap;

use anyhow::Result;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;

use crate::blob_runtime::BlobRuntime;
use crate::upload::{Uploader, wrap_external_proxy_url};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct InlineDataEntry {
    mime_type: String,
    data: String,
}

pub async fn finalize_output_urls(
    body: &mut Value,
    runtime: &BlobRuntime,
    uploader: &Uploader,
    external_proxy_prefix: &str,
) -> Result<()> {
    let entries = scan_inline_data_base64_entries(body);
    let mut replacements = HashMap::new();

    for entry in entries {
        let Ok(image_bytes) = STANDARD.decode(entry.data.as_bytes()) else {
            continue;
        };

        let Ok(blob) = runtime
            .store_bytes(image_bytes, entry.mime_type.clone())
            .await
        else {
            continue;
        };

        let upload_result = uploader.upload_blob(runtime, &blob, &entry.mime_type).await;
        runtime.remove(&blob).await?;

        let Ok(upload_result) = upload_result else {
            continue;
        };

        let final_url = if external_proxy_prefix.trim().is_empty() {
            upload_result.url
        } else {
            wrap_external_proxy_url(external_proxy_prefix, &upload_result.url)
        };
        replacements.insert(entry, final_url);
    }

    patch_inline_data_urls(body, &replacements);
    Ok(())
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

fn is_url_like(value: &str) -> bool {
    value.starts_with("http://")
        || value.starts_with("https://")
        || value.starts_with("/proxy/image")
}
