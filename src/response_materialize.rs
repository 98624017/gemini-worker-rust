use std::collections::HashMap;

use anyhow::Result;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;

use crate::blob_runtime::BlobRuntime;
use crate::config::Config;
use crate::upload::{Uploader, wrap_external_proxy_url};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct InlineDataEntry {
    mime_type: String,
    data: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InlineDataReplacement {
    mime_type: String,
    data: String,
}

pub fn optimize_inline_data_images(body: &mut Value, config: &Config) -> Result<()> {
    optimize_inline_data_images_with_options(
        body,
        config.enable_image_compression,
        crate::image_io::PNG_COMPRESSION_THRESHOLD_BYTES,
        config.image_compression_jpeg_quality,
    )
}

pub async fn finalize_output_urls(
    body: &mut Value,
    runtime: &BlobRuntime,
    uploader: &Uploader,
    config: &Config,
) -> Result<()> {
    let entries = scan_inline_data_base64_entries(body);
    let mut replacements = HashMap::new();
    let external_proxy_prefix = config.resolved_external_image_proxy_prefix();

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

        let final_url = if !should_proxy_standard_output_url(config, &upload_result.provider)
            || external_proxy_prefix.trim().is_empty()
        {
            upload_result.url
        } else {
            wrap_external_proxy_url(&external_proxy_prefix, &upload_result.url)
        };
        replacements.insert(entry, final_url);
    }

    patch_inline_data_urls(body, &replacements);
    Ok(())
}

#[cfg(test)]
fn optimize_inline_data_images_with_threshold(
    body: &mut Value,
    enabled: bool,
    threshold_bytes: usize,
) -> Result<()> {
    optimize_inline_data_images_with_options(
        body,
        enabled,
        threshold_bytes,
        crate::image_io::DEFAULT_JPEG_QUALITY,
    )
}

fn optimize_inline_data_images_with_options(
    body: &mut Value,
    enabled: bool,
    threshold_bytes: usize,
    jpeg_quality: u8,
) -> Result<()> {
    if !enabled {
        return Ok(());
    }

    let entries = scan_inline_data_base64_entries(body);
    let mut replacements = HashMap::new();

    for entry in entries {
        let Ok(image_bytes) = STANDARD.decode(entry.data.as_bytes()) else {
            continue;
        };

        let optimized = crate::image_io::maybe_compress_png_bytes_with_options(
            &image_bytes,
            &entry.mime_type,
            enabled,
            threshold_bytes,
            jpeg_quality,
        )?;
        if optimized.mime_type == entry.mime_type
            && optimized.bytes.as_ref() == image_bytes.as_slice()
        {
            continue;
        }

        replacements.insert(
            entry,
            InlineDataReplacement {
                mime_type: optimized.mime_type,
                data: STANDARD.encode(optimized.bytes),
            },
        );
    }

    patch_inline_data_base64(body, &replacements);
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

fn patch_inline_data_base64(
    node: &mut Value,
    replacements: &HashMap<InlineDataEntry, InlineDataReplacement>,
) {
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
                    if let Some(replacement) = replacements.get(&entry) {
                        inline_data.insert(
                            "mimeType".to_string(),
                            Value::String(replacement.mime_type.clone()),
                        );
                        inline_data
                            .insert("data".to_string(), Value::String(replacement.data.clone()));
                    }
                }
            }

            for child in map.values_mut() {
                patch_inline_data_base64(child, replacements);
            }
        }
        Value::Array(items) => {
            for child in items {
                patch_inline_data_base64(child, replacements);
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

fn should_proxy_standard_output_url(config: &Config, provider: &str) -> bool {
    config.proxy_standard_output_urls && !provider.eq_ignore_ascii_case("r2")
}

#[cfg(test)]
mod tests {
    use super::optimize_inline_data_images_with_threshold;
    use base64::Engine;
    use serde_json::json;
    use std::io::Cursor;

    #[test]
    fn optimize_inline_data_images_reencodes_large_png_base64_entries() {
        let image = image::RgbImage::from_fn(128, 128, |x, y| {
            image::Rgb([
                ((x * 31 + y * 17) % 255) as u8,
                ((x * 13 + y * 29) % 255) as u8,
                ((x * 7 + y * 47) % 255) as u8,
            ])
        });
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let png_base64 = base64::engine::general_purpose::STANDARD.encode(&png);
        let mut body = json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "inlineData": {
                            "mimeType": "image/png",
                            "data": png_base64
                        }
                    }]
                }
            }]
        });

        optimize_inline_data_images_with_threshold(&mut body, true, 1).unwrap();

        let inline_data = &body["candidates"][0]["content"]["parts"][0]["inlineData"];
        assert_eq!(inline_data["mimeType"], "image/jpeg");
        assert_ne!(inline_data["data"], png_base64);
    }
}
