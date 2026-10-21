use base64::Engine;
use serde_json::json;
use tokio::io::AsyncReadExt;

fn replacement(
    json_pointer: &str,
    blob: rust_sync_proxy::BlobHandle,
) -> rust_sync_proxy::request_materialize::RequestReplacement {
    rust_sync_proxy::request_materialize::RequestReplacement {
        json_pointer: json_pointer.to_string(),
        mime_type: "image/png".to_string(),
        blob,
    }
}

async fn read_blob_to_string(
    runtime: &rust_sync_proxy::BlobRuntime,
    handle: &rust_sync_proxy::BlobHandle,
) -> String {
    let mut reader = runtime.open_reader(handle).await.unwrap();
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).await.unwrap();
    String::from_utf8(buf).unwrap()
}

#[tokio::test]
async fn request_encoder_writes_inline_data_base64_from_blob_handle() {
    let runtime = rust_sync_proxy::test_blob_runtime(8 * 1024 * 1024);
    let blob = runtime
        .store_bytes(vec![1, 2, 3], "image/png".into())
        .await
        .unwrap();
    let request = json!({
        "output": "url",
        "contents": [{"parts": [{"inlineData": {"data": "https://example.com/a.png"}}]}]
    });
    let encoded = rust_sync_proxy::request_encode::encode_request_body(
        request,
        vec![replacement("/contents/0/parts/0/inlineData", blob)],
        &runtime,
    )
    .await
    .unwrap();

    let text = read_blob_to_string(&runtime, &encoded.body_blob).await;
    assert!(text.contains("\"mimeType\":\"image/png\""));
    assert!(text.contains("\"data\":\"AQID\""));
    assert!(!text.contains("\"output\""));
}

#[tokio::test]
async fn request_encoder_reads_large_external_shared_blob_without_spill() {
    let runtime = rust_sync_proxy::test_blob_runtime(1024);
    let source = bytes::Bytes::from(vec![7_u8; 4096]);
    let blob = runtime
        .store_external_shared_bytes(source.clone(), "image/png".into())
        .await
        .unwrap();
    let request = json!({
        "contents": [{"parts": [{"inlineData": {"data": "https://example.com/a.png"}}]}]
    });

    let encoded = rust_sync_proxy::request_encode::encode_request_body(
        request,
        vec![replacement("/contents/0/parts/0/inlineData", blob)],
        &runtime,
    )
    .await
    .unwrap();

    let text = read_blob_to_string(&runtime, &encoded.body_blob).await;
    let expected = base64::engine::general_purpose::STANDARD.encode(source.as_ref());
    assert!(text.contains(&expected));
}

#[tokio::test]
async fn request_encoder_writes_spilled_blob_as_base64() {
    let runtime = rust_sync_proxy::test_blob_runtime(1024);
    let source = vec![11_u8; 4096];
    let blob = runtime
        .store_bytes(source.clone(), "image/png".into())
        .await
        .unwrap();
    let request = json!({
        "contents": [{"parts": [{"inlineData": {"data": "https://example.com/a.png"}}]}]
    });

    let encoded = rust_sync_proxy::request_encode::encode_request_body(
        request,
        vec![replacement("/contents/0/parts/0/inlineData", blob)],
        &runtime,
    )
    .await
    .unwrap();

    let text = read_blob_to_string(&runtime, &encoded.body_blob).await;
    let expected = base64::engine::general_purpose::STANDARD.encode(source);
    assert!(text.contains(&expected));
}

#[tokio::test]
async fn request_encoder_cleans_partial_output_when_source_blob_is_unavailable() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = rust_sync_proxy::BlobRuntime::new(rust_sync_proxy::BlobRuntimeConfig {
        inline_max_bytes: 8,
        request_hot_budget_bytes: 8,
        global_hot_budget_bytes: 8,
        spill_dir: directory.path().to_path_buf(),
    });
    let blob = runtime
        .store_bytes(vec![11_u8; 4096], "image/png".into())
        .await
        .unwrap();
    runtime.remove(&blob).await.unwrap();
    let request = json!({
        "contents": [{"parts": [{"inlineData": {"data": "https://example.com/a.png"}}]}]
    });

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        rust_sync_proxy::request_encode::encode_request_body(
            request,
            vec![replacement("/contents/0/parts/0/inlineData", blob)],
            &runtime,
        ),
    )
    .await
    .expect("encoder must stop when its source blob is unavailable");

    assert!(result.is_err());
    assert!(
        std::fs::read_dir(directory.path())
            .unwrap()
            .next()
            .is_none()
    );
}
