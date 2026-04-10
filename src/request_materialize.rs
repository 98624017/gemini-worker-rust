use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Result, anyhow};
use serde_json::Value;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::blob_runtime::{BlobHandle, BlobRuntime};
use crate::cache::InlineDataUrlFetchService;
use crate::image_io::{FetchedBlob, REQUEST_MAX_IMAGE_BYTES, fetch_image_into_blob};
use crate::request_scan::scan_request_image_urls;

const MAX_CONCURRENT_REQUEST_IMAGE_FETCHES: usize = 4;

#[derive(Clone, Debug)]
pub struct RequestReplacement {
    pub json_pointer: String,
    pub mime_type: String,
    pub blob: BlobHandle,
}

#[derive(Clone, Debug)]
pub struct MaterializedRequestImages {
    pub request: Value,
    pub replacements: Vec<RequestReplacement>,
}

#[derive(Clone)]
pub struct RequestMaterializeServices {
    pub image_client: reqwest::Client,
    pub max_image_bytes: usize,
    pub allow_private_networks: bool,
    pub fetch_service: Option<Arc<InlineDataUrlFetchService>>,
    pub cache_observer: Option<Arc<dyn Fn(&str, bool) + Send + Sync>>,
}

pub async fn materialize_request_images(
    request: Value,
    runtime: &BlobRuntime,
    client: &reqwest::Client,
) -> Result<MaterializedRequestImages> {
    let services = RequestMaterializeServices {
        image_client: client.clone(),
        max_image_bytes: REQUEST_MAX_IMAGE_BYTES,
        allow_private_networks: true,
        fetch_service: None,
        cache_observer: None,
    };
    materialize_request_images_with_services(request, runtime, &services).await
}

pub async fn materialize_request_images_with_services(
    request: Value,
    runtime: &BlobRuntime,
    services: &RequestMaterializeServices,
) -> Result<MaterializedRequestImages> {
    let refs = scan_request_image_urls(&request)?;
    let mut unique_urls = Vec::new();
    let mut seen_urls = HashSet::new();
    for image_ref in &refs {
        if seen_urls.insert(image_ref.url.clone()) {
            unique_urls.push(image_ref.url.clone());
        }
    }

    let runtime = runtime.clone();
    let services = services.clone();
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_REQUEST_IMAGE_FETCHES.max(1)));
    let mut fetches = JoinSet::new();

    for url in unique_urls {
        let runtime = runtime.clone();
        let services = services.clone();
        let semaphore = Arc::clone(&semaphore);
        fetches.spawn(async move {
            let _permit = semaphore
                .acquire_owned()
                .await
                .map_err(|err| anyhow!("request materialize semaphore closed: {err}"))?;
            fetch_request_image(url, &runtime, &services).await
        });
    }

    let mut fetched_by_url = HashMap::new();
    while let Some(result) = fetches.join_next().await {
        let (url, fetched) =
            result.map_err(|err| anyhow!("request image fetch task failed: {err}"))??;
        fetched_by_url.insert(url, fetched);
    }

    let mut replacements = Vec::with_capacity(refs.len());
    for image_ref in refs {
        let fetched = fetched_by_url
            .get(&image_ref.url)
            .ok_or_else(|| anyhow!("missing fetched blob for {}", image_ref.url))?;
        replacements.push(RequestReplacement {
            json_pointer: image_ref.json_pointer,
            mime_type: fetched.mime_type.clone(),
            blob: fetched.blob.clone(),
        });
    }

    Ok(MaterializedRequestImages {
        request,
        replacements,
    })
}

async fn fetch_request_image(
    url: String,
    runtime: &BlobRuntime,
    services: &RequestMaterializeServices,
) -> Result<(String, FetchedBlob)> {
    let fetched = if let Some(fetch_service) = &services.fetch_service {
        let fetched = fetch_service.fetch(&url).await?;
        if let Some(observer) = &services.cache_observer {
            observer(&url, fetched.from_cache);
        }
        FetchedBlob {
            mime_type: fetched.mime_type.clone(),
            blob: runtime
                .store_bytes(fetched.bytes.to_vec(), fetched.mime_type)
                .await?,
        }
    } else {
        fetch_image_into_blob(
            &services.image_client,
            runtime,
            &url,
            services.max_image_bytes,
            services.allow_private_networks,
        )
        .await?
    };

    Ok((url, fetched))
}
