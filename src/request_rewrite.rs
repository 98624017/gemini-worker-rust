use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use serde_json::Value;

use crate::blob_runtime::{BlobRuntime, BlobRuntimeConfig};
use crate::cache::InlineDataUrlFetchService;
use crate::image_io::DEFAULT_MAX_IMAGE_BYTES;
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
            max_image_bytes: DEFAULT_MAX_IMAGE_BYTES,
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

pub async fn rewrite_request_inline_data(
    body: Value,
    services: &RewriteServices,
) -> Result<Value> {
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
        spill_dir: std::env::temp_dir().join(format!("rust-sync-proxy-request-rewrite-{unix_ms}-{id}")),
    })
}
