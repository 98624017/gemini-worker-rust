use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;

use crate::blob_runtime::{BlobHandle, BlobRuntime};
use crate::cache::InlineDataUrlFetchService;
use crate::image_io::{DEFAULT_MAX_IMAGE_BYTES, FetchedBlob, fetch_image_into_blob};
use crate::request_scan::scan_request_image_urls;

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
        max_image_bytes: DEFAULT_MAX_IMAGE_BYTES,
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
    let mut replacements = Vec::with_capacity(refs.len());

    for image_ref in refs {
        let fetched = if let Some(fetch_service) = &services.fetch_service {
            let fetched = fetch_service.fetch(&image_ref.url).await?;
            if let Some(observer) = &services.cache_observer {
                observer(&image_ref.url, fetched.from_cache);
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
                &image_ref.url,
                services.max_image_bytes,
                services.allow_private_networks,
            )
            .await?
        };
        replacements.push(RequestReplacement {
            json_pointer: image_ref.json_pointer,
            mime_type: fetched.mime_type,
            blob: fetched.blob,
        });
    }

    Ok(MaterializedRequestImages {
        request,
        replacements,
    })
}
