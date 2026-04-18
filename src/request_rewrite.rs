use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use serde_json::{Map, Value};
use url::Url;

use crate::blob_runtime::{BlobRuntime, BlobRuntimeConfig};
use crate::cache::InlineDataUrlFetchService;
use crate::image_io::{REQUEST_MAX_IMAGE_BYTES, hostname_matches_domain_patterns};
use crate::request_encode::encode_request_body;
use crate::request_materialize::{
    RequestMaterializeServices, materialize_request_images_with_services,
};
use crate::request_scan::{MAX_INLINE_DATA_URLS, scan_request_image_urls};

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
            enable_webp_optimization: false,
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

pub fn rewrite_aiapidev_request_body(
    mut body: Value,
    external_proxy_prefix: &str,
    external_proxy_domains: &[String],
) -> Value {
    strip_output_fields(&mut body);
    rewrite_aiapidev_value(body, "", external_proxy_prefix, external_proxy_domains)
}

fn rewrite_aiapidev_value(
    value: Value,
    path: &str,
    external_proxy_prefix: &str,
    external_proxy_domains: &[String],
) -> Value {
    match value {
        Value::Object(map) => {
            rewrite_aiapidev_object(map, path, external_proxy_prefix, external_proxy_domains)
        }
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .enumerate()
                .map(|(index, item)| {
                    rewrite_aiapidev_value(
                        item,
                        &format!("{path}/{index}"),
                        external_proxy_prefix,
                        external_proxy_domains,
                    )
                })
                .collect(),
        ),
        primitive => primitive,
    }
}

fn rewrite_aiapidev_object(
    map: Map<String, Value>,
    path: &str,
    external_proxy_prefix: &str,
    external_proxy_domains: &[String],
) -> Value {
    if let Some((key, payload)) =
        rewrite_inline_data_payload(&map, external_proxy_prefix, external_proxy_domains)
    {
        let mut out = Map::new();
        out.insert(key.to_string(), payload);
        return Value::Object(out);
    }

    let mut out = Map::new();

    if path.starts_with("/contents/") {
        if let Some(role) = map.get("role").cloned() {
            out.insert(
                "role".to_string(),
                rewrite_aiapidev_value(
                    role,
                    &format!("{path}/role"),
                    external_proxy_prefix,
                    external_proxy_domains,
                ),
            );
        }
        if let Some(parts) = map.get("parts").cloned() {
            out.insert(
                "parts".to_string(),
                rewrite_aiapidev_value(
                    parts,
                    &format!("{path}/parts"),
                    external_proxy_prefix,
                    external_proxy_domains,
                ),
            );
        }
    }

    for (key, child) in map {
        if path.starts_with("/contents/") && matches!(key.as_str(), "role" | "parts") {
            continue;
        }
        let rewritten_key = rewrite_aiapidev_key(&key);
        let child_path = format!("{path}/{}", escape_json_pointer_token(&rewritten_key));
        out.insert(
            rewritten_key,
            rewrite_aiapidev_value(
                child,
                &child_path,
                external_proxy_prefix,
                external_proxy_domains,
            ),
        );
    }

    Value::Object(out)
}

fn rewrite_inline_data_payload(
    map: &Map<String, Value>,
    external_proxy_prefix: &str,
    external_proxy_domains: &[String],
) -> Option<(&'static str, Value)> {
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
        let file_uri =
            maybe_wrap_aiapidev_file_uri(data, external_proxy_prefix, external_proxy_domains);
        return Some((
            "file_data",
            serde_json::json!({
                "file_uri": file_uri,
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

fn maybe_wrap_aiapidev_file_uri(
    raw_url: &str,
    external_proxy_prefix: &str,
    external_proxy_domains: &[String],
) -> String {
    if external_proxy_prefix.is_empty()
        || external_proxy_domains.is_empty()
        || is_already_wrapped_external_proxy_url(raw_url, external_proxy_prefix)
    {
        return raw_url.to_string();
    }

    let Ok(mut parsed) = Url::parse(raw_url) else {
        return raw_url.to_string();
    };
    let hostname = parsed.host_str().unwrap_or_default();
    if !hostname_matches_domain_patterns(hostname, external_proxy_domains) {
        return raw_url.to_string();
    }
    parsed.set_query(None);
    parsed.set_fragment(None);
    let stripped_url = parsed.to_string();

    format!(
        "{external_proxy_prefix}{}",
        url::form_urlencoded::byte_serialize(stripped_url.as_bytes()).collect::<String>()
    )
}

fn is_already_wrapped_external_proxy_url(raw_url: &str, external_proxy_prefix: &str) -> bool {
    raw_url.starts_with(external_proxy_prefix)
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
