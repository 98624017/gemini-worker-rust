use anyhow::Result;
use serde_json::Value;

use crate::blob_runtime::{BlobHandle, BlobRuntime};
use crate::image_io::{DEFAULT_MAX_IMAGE_BYTES, fetch_image_into_blob};
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

pub async fn materialize_request_images(
    request: Value,
    runtime: &BlobRuntime,
    client: &reqwest::Client,
) -> Result<MaterializedRequestImages> {
    let refs = scan_request_image_urls(&request)?;
    let mut replacements = Vec::with_capacity(refs.len());

    for image_ref in refs {
        let fetched =
            fetch_image_into_blob(client, runtime, &image_ref.url, DEFAULT_MAX_IMAGE_BYTES, true)
                .await?;
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
