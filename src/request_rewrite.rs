use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use serde_json::{Map, Value};

use crate::blob_runtime::{BlobRuntime, BlobRuntimeConfig};
use crate::cache::InlineDataUrlFetchService;
use crate::image_io::REQUEST_MAX_IMAGE_BYTES;
use crate::request_encode::encode_request_body;
use crate::request_materialize::{
    RequestMaterializeServices, materialize_request_images_with_services,
};
use crate::request_scan::scan_request_image_urls;

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
            max_image_bytes: REQUEST_MAX_IMAGE_BYTES,
            allow_private_networks: false,
            fetch_service: None,
            cache_observer: None,
        }
    }
}

pub fn scan_inline_data_urls(body: &Value) -> Result<InlineDataScan> {
    let refs = scan_request_image_urls(body)?;
    if refs.len() > MAX_INLINE_DATA_URLS {
        return Err(anyhow!("too many inlineData URLs"));
    }

    let mut unique_urls = HashSet::new();
    for image_ref in &refs {
        unique_urls.insert(image_ref.url.clone());
    }

    Ok(InlineDataScan {
        unique_urls: unique_urls.into_iter().collect(),
        total_refs: refs.len(),
    })
}

pub async fn rewrite_request_inline_data(body: Value, services: &RewriteServices) -> Result<Value> {
    let scan = scan_inline_data_urls(&body)?;
    if scan.total_refs == 0 {
        return Ok(body);
    }

    let runtime = compat_blob_runtime();
    let materialized = materialize_request_images_with_services(
        body,
        &runtime,
        &RequestMaterializeServices {
            image_client: services.image_client.clone(),
            max_image_bytes: services.max_image_bytes,
            allow_private_networks: services.allow_private_networks,
            fetch_service: services.fetch_service.clone(),
            cache_observer: services.cache_observer.clone(),
        },
    )
    .await?;
    let encoded = encode_request_body(
        materialized.request,
        materialized.replacements.clone(),
        &runtime,
    )
    .await?;
    let bytes = runtime.read_bytes(&encoded.body_blob).await?;

    for replacement in &materialized.replacements {
        runtime.remove(&replacement.blob).await?;
    }
    runtime.remove(&encoded.body_blob).await?;

    Ok(serde_json::from_slice(&bytes)?)
}

pub fn rewrite_aiapidev_request_body(mut body: Value) -> Value {
    strip_output_fields(&mut body);
    rewrite_aiapidev_value(body, "")
}

fn rewrite_aiapidev_value(value: Value, path: &str) -> Value {
    match value {
        Value::Object(map) => rewrite_aiapidev_object(map, path),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .enumerate()
                .map(|(index, item)| rewrite_aiapidev_value(item, &format!("{path}/{index}")))
                .collect(),
        ),
        primitive => primitive,
    }
}

fn rewrite_aiapidev_object(map: Map<String, Value>, path: &str) -> Value {
    if let Some((key, payload)) = rewrite_inline_data_payload(&map) {
        let mut out = Map::new();
        out.insert(key.to_string(), payload);
        return Value::Object(out);
    }

    let mut out = Map::new();

    if path.starts_with("/contents/") {
        if let Some(role) = map.get("role").cloned() {
            out.insert(
                "role".to_string(),
                rewrite_aiapidev_value(role, &format!("{path}/role")),
            );
        }
        if let Some(parts) = map.get("parts").cloned() {
            out.insert(
                "parts".to_string(),
                rewrite_aiapidev_value(parts, &format!("{path}/parts")),
            );
        }
    }

    for (key, child) in map {
        if path.starts_with("/contents/") && matches!(key.as_str(), "role" | "parts") {
            continue;
        }
        let rewritten_key = rewrite_aiapidev_key(&key);
        let child_path = format!("{path}/{}", escape_json_pointer_token(&rewritten_key));
        out.insert(rewritten_key, rewrite_aiapidev_value(child, &child_path));
    }

    Value::Object(out)
}

fn rewrite_inline_data_payload(map: &Map<String, Value>) -> Option<(&'static str, Value)> {
    let inline_data = map
        .get("inlineData")
        .or_else(|| map.get("inline_data"))?
        .as_object()?;
    let data = inline_data.get("data")?.as_str()?;
    let mime_type = inline_data
        .get("mimeType")
        .or_else(|| inline_data.get("mime_type"))
        .and_then(Value::as_str)
        .unwrap_or("image/png");

    if is_http_url(data) {
        return Some((
            "file_data",
            serde_json::json!({
                "file_uri": data,
                "mime_type": mime_type,
            }),
        ));
    }

    Some((
        "inline_data",
        serde_json::json!({
            "data": data,
            "mime_type": mime_type,
        }),
    ))
}

fn rewrite_aiapidev_key(key: &str) -> String {
    match key {
        "generationConfig" => "generation_config".to_string(),
        "imageConfig" => "image_config".to_string(),
        "responseModalities" => "response_modalities".to_string(),
        "aspectRatio" => "aspect_ratio".to_string(),
        "imageSize" => "image_size".to_string(),
        _ => key.to_string(),
    }
}

fn strip_output_fields(body: &mut Value) {
    if let Some(map) = body.as_object_mut() {
        map.remove("output");
    }

    for pointer in [
        "/generationConfig/imageConfig",
        "/generation_config/image_config",
    ] {
        if let Some(image_config) = body.pointer_mut(pointer) {
            if let Some(map) = image_config.as_object_mut() {
                map.remove("output");
            }
        }
    }
}

fn escape_json_pointer_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

fn is_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn compat_blob_runtime() -> BlobRuntime {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);

    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    BlobRuntime::new(BlobRuntimeConfig {
        inline_max_bytes: 8 * 1024 * 1024,
        request_hot_budget_bytes: 24 * 1024 * 1024,
        global_hot_budget_bytes: 384 * 1024 * 1024,
        spill_dir: std::env::temp_dir()
            .join(format!("rust-sync-proxy-request-rewrite-{unix_ms}-{id}")),
    })
}
