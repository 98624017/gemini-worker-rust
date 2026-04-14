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
