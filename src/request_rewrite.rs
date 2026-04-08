use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use anyhow::{Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;

use crate::cache::InlineDataUrlFetchService;
use crate::image_io::{DEFAULT_MAX_IMAGE_BYTES, fetch_image_as_inline_data_with_options};

const MAX_INLINE_DATA_URLS: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineDataScan {
    pub unique_urls: Vec<String>,
    pub total_refs: usize,
}

#[derive(Clone)]
pub struct RewriteServices {
    pub image_client: reqwest::Client,
    pub max_image_bytes: usize,
    pub allow_private_networks: bool,
    pub fetch_service: Option<Arc<InlineDataUrlFetchService>>,
    pub cache_observer: Option<Arc<dyn Fn(&str, bool) + Send + Sync>>,
}

impl Default for RewriteServices {
    fn default() -> Self {
        Self {
            image_client: reqwest::Client::new(),
            max_image_bytes: DEFAULT_MAX_IMAGE_BYTES,
            allow_private_networks: false,
            fetch_service: None,
            cache_observer: None,
        }
    }
}

pub fn scan_inline_data_urls(body: &Value) -> Result<InlineDataScan> {
    let mut unique_urls = BTreeSet::new();
    let mut total_refs = 0usize;

    fn walk(
        node: &Value,
        unique_urls: &mut BTreeSet<String>,
        total_refs: &mut usize,
    ) -> Result<()> {
        match node {
            Value::Object(map) => {
                if let Some(Value::Object(inline_data)) = map.get("inlineData") {
                    if let Some(Value::String(data)) = inline_data.get("data") {
                        if is_http_url(data) {
                            *total_refs += 1;
                            if *total_refs > MAX_INLINE_DATA_URLS {
                                return Err(anyhow!("too many inlineData URLs"));
                            }
                            unique_urls.insert(data.clone());
                        }
                    }
                }

                for child in map.values() {
                    walk(child, unique_urls, total_refs)?;
                }
            }
            Value::Array(items) => {
                for child in items {
                    walk(child, unique_urls, total_refs)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    walk(body, &mut unique_urls, &mut total_refs)?;

    Ok(InlineDataScan {
        unique_urls: unique_urls.into_iter().collect(),
        total_refs,
    })
}

pub async fn rewrite_request_inline_data(
    mut body: Value,
    services: &RewriteServices,
) -> Result<Value> {
    let scan = scan_inline_data_urls(&body)?;
    if scan.total_refs == 0 {
        return Ok(body);
    }

    let mut replacements = HashMap::new();
    for raw_url in &scan.unique_urls {
        let fetched = if let Some(fetch_service) = &services.fetch_service {
            let fetched = fetch_service.fetch(raw_url).await?;
            if let Some(observer) = &services.cache_observer {
                observer(raw_url, fetched.from_cache);
            }
            crate::image_io::FetchedInlineData {
                mime_type: fetched.mime_type,
                bytes: fetched.bytes,
            }
        } else {
            fetch_image_as_inline_data_with_options(
                &services.image_client,
                raw_url,
                services.max_image_bytes,
                services.allow_private_networks,
            )
            .await?
        };
        replacements.insert(
            raw_url.clone(),
            (fetched.mime_type, STANDARD.encode(fetched.bytes.as_ref())),
        );
    }

    // 二次遍历只做补丁回填，避免把可变引用跨 await 传播。
    patch_inline_data_urls(&mut body, &replacements);
    Ok(body)
}

fn patch_inline_data_urls(node: &mut Value, replacements: &HashMap<String, (String, String)>) {
    match node {
        Value::Object(map) => {
            if let Some(Value::Object(inline_data)) = map.get_mut("inlineData") {
                if let Some(Value::String(data)) = inline_data.get("data") {
                    if let Some((mime_type, base64_data)) = replacements.get(data) {
                        inline_data.insert("data".to_string(), Value::String(base64_data.clone()));
                        inline_data
                            .insert("mimeType".to_string(), Value::String(mime_type.clone()));
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

fn is_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}
