use anyhow::{Result, anyhow};

use rust_sync_proxy::upload::{
    BoxUploadFuture, ImageHostMode, UploadResult, upload_image_with_mode, wrap_external_proxy_url,
};

#[tokio::test]
async fn r2_then_legacy_falls_back_to_legacy_on_r2_failure() {
    let result = upload_image_with_mode(
        ImageHostMode::R2ThenLegacy,
        b"png-bytes",
        "image/png",
        &failing_r2(),
        &working_legacy(),
    )
    .await
    .unwrap();

    assert_eq!(result.provider, "legacy");
}

fn failing_r2() -> impl Fn(Vec<u8>, String) -> BoxUploadFuture + Sync {
    |_data, _mime_type| Box::pin(async { Err(anyhow!("r2 failed")) })
}

fn working_legacy() -> impl Fn(Vec<u8>, String) -> BoxUploadFuture + Sync {
    |data, mime_type| {
        Box::pin(async move {
            ok_upload_result(
                "legacy",
                format!(
                    "https://legacy.example/upload?size={}&mime={}",
                    data.len(),
                    mime_type
                ),
            )
        })
    }
}

fn ok_upload_result(provider: &str, url: String) -> Result<UploadResult> {
    Ok(UploadResult {
        url,
        provider: provider.to_string(),
    })
}

#[test]
fn external_proxy_prefix_wraps_uploaded_url() {
    let wrapped = wrap_external_proxy_url(
        "https://proxy.example.com/fetch?url=",
        "https://img.example.com/a.png",
    );
    assert_eq!(
        wrapped,
        "https://proxy.example.com/fetch?url=https%3A%2F%2Fimg.example.com%2Fa.png"
    );
}
