use serde_json::json;

#[test]
fn normalize_openai_image_request_supports_all_aliases() {
    let cases = [
        (
            json!({
                "model": "gpt-image-2",
                "prompt": "draw cat",
                "image": ["https://img.example/a.png"],
            }),
            json!(["https://img.example/a.png"]),
        ),
        (
            json!({
                "model": "gpt-image-2",
                "prompt": "draw cat",
                "images": ["https://img.example/b.png"],
            }),
            json!(["https://img.example/b.png"]),
        ),
        (
            json!({
                "model": "gpt-image-2",
                "prompt": "draw cat",
                "reference_images": ["https://img.example/c.png"],
            }),
            json!(["https://img.example/c.png"]),
        ),
    ];

    for (body, want_images) in cases {
        let normalized = rust_sync_proxy::openai_image::normalize_request_body(body).unwrap();
        assert_eq!(normalized["reference_images"], want_images);
        assert_eq!(normalized["response_format"], "b64_json");
        assert!(normalized.get("image").is_none());
        assert!(normalized.get("images").is_none());
    }
}

#[test]
fn build_openai_image_response_falls_back_created_timestamp() {
    let body = json!({
        "data": [{"b64_json": "iVBORw0KGgo="}]
    });

    let response = rust_sync_proxy::openai_image::build_response_payload(
        body,
        &[rust_sync_proxy::openai_image::UploadedImage {
            url: "https://img.example/final.png".to_string(),
        }],
        1_776_663_103,
    )
    .unwrap();

    assert_eq!(response["created"], 1_776_663_103);
    assert_eq!(response["data"][0]["url"], "https://img.example/final.png");
    assert_eq!(response["usage"]["total_tokens"], 2048);
}

#[test]
fn build_openai_image_response_preserves_upstream_created_timestamp() {
    let body = json!({
        "created": 1_776_663_555,
        "data": [{"b64_json": "iVBORw0KGgo="}]
    });

    let response = rust_sync_proxy::openai_image::build_response_payload(
        body,
        &[rust_sync_proxy::openai_image::UploadedImage {
            url: "https://img.example/final.png".to_string(),
        }],
        1_776_663_103,
    )
    .unwrap();

    assert_eq!(response["created"], 1_776_663_555);
}

#[test]
fn sniff_image_mime_type_detects_known_formats() {
    let cases = [
        (&[137, 80, 78, 71, 13, 10, 26, 10][..], Some("image/png")),
        (&[0xFF, 0xD8, 0xFF, 0xE0][..], Some("image/jpeg")),
        (
            &[b'R', b'I', b'F', b'F', 1, 2, 3, 4, b'W', b'E', b'B', b'P'][..],
            Some("image/webp"),
        ),
        (&b"GIF89a"[..], Some("image/gif")),
        (&[1, 2, 3, 4][..], None),
    ];

    for (bytes, want) in cases {
        let got = rust_sync_proxy::image_io::sniff_image_mime_type(bytes);
        assert_eq!(got, want);
    }
}
