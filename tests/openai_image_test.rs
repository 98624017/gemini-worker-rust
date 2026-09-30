use serde_json::json;

#[test]
fn tianyue_request_rewrites_model_and_merges_image_aliases() {
    for size in ["1K", "1k", "2K", "2k"] {
        let body = rust_sync_proxy::openai_image::normalize_request_body_for_upstream(
            json!({
                "model": "seedream-5-pro",
                "size": size,
                "prompt": "draw cat",
                "image": ["https://img.example/a.png"],
                "images": ["https://img.example/b.png"],
                "reference_images": ["https://img.example/c.png"],
            }),
            false,
            "https://API.TIANYUE.XYZ/v1",
        )
        .unwrap();
        assert_eq!(
            body["model"],
            format!("GZ-seedream-5-pro-{}", size.to_ascii_uppercase())
        );
        assert_eq!(body["size"], size);
        assert_eq!(body["prompt"], "draw cat");
        assert_eq!(body["response_format"], "url");
        assert_eq!(
            body["images"],
            json!([
                "https://img.example/c.png",
                "https://img.example/b.png",
                "https://img.example/a.png",
            ])
        );
        assert!(body.get("image").is_none());
        assert!(body.get("reference_images").is_none());
    }
}

#[test]
fn tianyue_request_keeps_other_models_sizes_and_channels_unchanged() {
    for body in [
        json!({"model": "seedream-5-pro"}),
        json!({"model": "seedream-5-pro", "size": "4K"}),
        json!({"model": "seedream-5-pro", "size": 2}),
        json!({"model": "other-model", "size": "2K"}),
        json!({"model": "GZ-seedream-5-pro-1K", "size": "2K"}),
    ] {
        let normalized = rust_sync_proxy::openai_image::normalize_request_body_for_upstream(
            body.clone(),
            false,
            "https://api.tianyue.xyz",
        )
        .unwrap();
        assert_eq!(normalized["model"], body["model"]);
        assert!(normalized.get("images").is_none());
    }

    let body =
        json!({"model": "seedream-5-pro", "size": "2K", "image": ["https://img.example/a.png"]});
    for base_url in [
        "https://api.example.com",
        "https://api.tianyue.xyz.evil.com",
        "https://other.tianyue.xyz",
        "invalid",
    ] {
        let normalized = rust_sync_proxy::openai_image::normalize_request_body_for_upstream(
            body.clone(),
            false,
            base_url,
        )
        .unwrap();
        assert_eq!(normalized["model"], body["model"]);
        assert_eq!(normalized["image"], body["image"]);
        assert!(normalized.get("images").is_none());
    }

    assert!(
        rust_sync_proxy::openai_image::normalize_request_body_for_upstream(
            json!({"image": ["file:///tmp/a.png"]}),
            false,
            "https://api.tianyue.xyz",
        )
        .is_err()
    );
}

#[test]
fn normalize_openai_image_request_supports_all_aliases_without_forcing_b64_json() {
    let cases = [
        (
            json!({
                "model": "gpt-image-2",
                "prompt": "draw cat",
                "image": ["https://img.example/a.png"],
            }),
            "image",
        ),
        (
            json!({
                "model": "gpt-image-2",
                "prompt": "draw cat",
                "images": ["https://img.example/b.png"],
            }),
            "images",
        ),
        (
            json!({
                "model": "gpt-image-2",
                "prompt": "draw cat",
                "reference_images": ["https://img.example/c.png"],
            }),
            "reference_images",
        ),
    ];

    for (body, want_field) in cases {
        let want_images = body[want_field].clone();
        let normalized =
            rust_sync_proxy::openai_image::normalize_request_body(body, false).unwrap();
        assert_eq!(normalized[want_field], want_images);
        assert_eq!(normalized["response_format"], "url");
        for alias in ["image", "images", "reference_images"] {
            if alias == want_field {
                continue;
            }
            assert!(normalized.get(alias).is_none());
        }
    }
}

#[test]
fn normalize_openai_image_request_forces_b64_json_when_requested() {
    let normalized = rust_sync_proxy::openai_image::normalize_request_body(
        json!({
            "model": "gpt-image-2",
            "prompt": "draw cat",
            "image": ["https://img.example/a.png"],
            "response_format": "url"
        }),
        true,
    )
    .unwrap();

    assert_eq!(normalized["image"], json!(["https://img.example/a.png"]));
    assert_eq!(normalized["response_format"], "b64_json");
    assert!(normalized.get("images").is_none());
    assert!(normalized.get("reference_images").is_none());
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
fn build_openai_image_response_uses_aiapidev_finished_time_when_created_missing() {
    let body = json!({
        "finishedTime": "2026-04-23 15:52:58",
        "result": {
            "items": [{
                "url": "https://pub.example.com/result.png",
                "type": "image"
            }]
        }
    });

    let response = rust_sync_proxy::openai_image::build_response_payload(
        body,
        &[rust_sync_proxy::openai_image::UploadedImage {
            url: "https://img.example/final.png".to_string(),
        }],
        1_776_663_103,
    )
    .unwrap();

    assert_eq!(response["created"], 1_776_959_578_i64);
}

#[test]
fn build_aiapidev_openai_image_response_uses_fixed_usage() {
    let response = rust_sync_proxy::openai_image::build_response_payload_from_uploaded(
        &[rust_sync_proxy::openai_image::UploadedImage {
            url: "https://img.example/final.png".to_string(),
        }],
        1_776_663_103,
    );

    assert_eq!(response["created"], 1_776_663_103);
    assert_eq!(response["data"][0]["url"], "https://img.example/final.png");
    assert_eq!(response["usage"]["input_tokens"], 1024);
    assert_eq!(response["usage"]["output_tokens"], 1024);
    assert_eq!(response["usage"]["total_tokens"], 2048);
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
