use serde_json::json;

#[test]
fn api888_seedream_maps_documented_ratios_and_preserves_native_sizes() {
    for (ratio, size) in [
        ("1:1", "2048x2048"),
        ("3:4", "1728x2304"),
        ("16:9", "2560x1440"),
        ("4:3", "2304x1728"),
        ("9:16", "1440x2560"),
        ("2:3", "1664x2496"),
        ("3:2", "2496x1664"),
        ("21:9", "3024x1296"),
    ] {
        let legacy = json!({"size": "2k", "aspect_ratio": ratio, "output_resolution": "2K"});
        let native = json!({"size": size});
        for mut request in [legacy, native] {
            request["model"] = json!("seedream-5.5");
            request["prompt"] = json!("geometric pattern");
            let normalized = rust_sync_proxy::openai_image::normalize_request_body_for_upstream(
                request,
                false,
                "https://888API.XIN/v1/",
            )
            .unwrap();
            assert_eq!(normalized["size"], size, "{ratio}");
            assert!(normalized.get("aspect_ratio").is_none());
            assert!(normalized.get("output_resolution").is_none());
        }
    }
}

#[test]
fn api888_seedream_rejects_unsupported_sizes_and_fields() {
    for (field, value) in [
        ("aspect_ratio", json!("4:5")),
        ("size", json!("4K")),
        ("size", json!("2048x2048")),
        ("quality", json!("high")),
        ("image", json!(["http://img.example/ref.png"])),
    ] {
        let mut request = json!({
            "model": "seedream-5.5", "prompt": "geometric pattern", "aspect_ratio": "3:2"
        });
        request[field] = value;
        assert!(
            rust_sync_proxy::openai_image::normalize_request_body_for_upstream(
                request,
                false,
                "https://888api.xin",
            )
            .is_err(),
            "unexpectedly accepted {field}"
        );
    }
}

#[test]
fn api888_seedream_preserves_other_hosts_and_models() {
    for (base_url, model) in [
        ("https://888api.xin", "other-model"),
        ("https://888api.xin.example.com", "seedream-5.5"),
        ("https://www.888api.xin", "seedream-5.5"),
    ] {
        let request = json!({
            "model": model,
            "prompt": "geometric pattern",
            "size": "2K",
            "aspect_ratio": "3:2",
            "image": ["https://img.example/reference.png"],
            "watermark": false,
            "response_format": "url"
        });
        let normalized = rust_sync_proxy::openai_image::normalize_request_body_for_upstream(
            request.clone(),
            false,
            base_url,
        )
        .unwrap();
        assert_eq!(normalized, request);
    }
}

#[test]
fn ai666_seedream_55_uses_legacy_resolution_without_replacing_native_size() {
    let body = rust_sync_proxy::openai_image::normalize_request_body_for_upstream(
        json!({
            "model": "seedream-5.5",
            "prompt": "geometric pattern",
            "size": "1536x1024",
            "output_resolution": "1k",
            "image": [],
            "n": 1,
        }),
        false,
        "https://US.AI666.LIVE/v1/",
    )
    .unwrap();
    assert_eq!(body["size"], "1536x1024");
    assert_eq!(body["resolution"], "1K");
    assert!(body.get("output_resolution").is_none());
    assert!(body.get("images").is_none());
}

#[test]
fn ai666_seedream_55_rejects_unsupported_resolution_and_count() {
    for (field, value) in [
        ("size", json!("4k")),
        ("resolution", json!(2)),
        ("n", json!(2)),
    ] {
        let mut request = json!({"model": "seedream-5.5", "prompt": "geometric pattern"});
        request[field] = value;
        assert!(
            rust_sync_proxy::openai_image::normalize_request_body_for_upstream(
                request,
                false,
                "https://us.ai666.live",
            )
            .is_err(),
            "unexpectedly accepted {field}"
        );
    }
}

#[test]
fn ai666_seedream_55_preserves_other_models_and_hosts() {
    for (base_url, model) in [
        ("https://us.ai666.live", "other-model"),
        ("https://us.ai666.live.example.com", "seedream-5.5"),
        ("https://ai666.live", "seedream-5.5"),
    ] {
        let request = json!({
            "model": model,
            "prompt": "geometric pattern",
            "size": "2K",
            "image": ["https://img.example/reference.png"],
            "response_format": "url",
        });
        let normalized = rust_sync_proxy::openai_image::normalize_request_body_for_upstream(
            request.clone(),
            false,
            base_url,
        )
        .unwrap();
        assert_eq!(normalized, request);
    }
}

#[test]
fn tianyue_request_rewrites_model_and_merges_image_aliases() {
    for size in ["1K", "1k", "2K", "2k"] {
        let body = rust_sync_proxy::openai_image::normalize_request_body_for_upstream(
            json!({
                "model": "seedream-5-pro",
                "size": size,
                "prompt": "draw cat",
                "duration": 1,
                "aspect_ratio": "16:9",
                "output_resolution": size.to_ascii_uppercase(),
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
            format!("seedream-5-pro-{}", size.to_ascii_uppercase())
        );
        assert_eq!(body["size"], size);
        assert_eq!(body["prompt"], "draw cat");
        assert_eq!(body["duration"], 1);
        assert_eq!(body["aspect_ratio"], "16:9");
        assert_eq!(body["output_resolution"], size.to_ascii_uppercase());
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
fn tianyue_request_maps_seedream_55_with_or_without_size() {
    let with_size = rust_sync_proxy::openai_image::normalize_request_body_for_upstream(
        json!({
            "model": "seedream-5.5",
            "size": "4K",
            "prompt": "draw cat",
            "duration": 1,
            "aspect_ratio": "16:9",
            "image": ["https://img.example/a.png"],
            "images": ["https://img.example/b.png"],
            "reference_images": ["https://img.example/c.png"],
        }),
        false,
        "https://api.tianyue.xyz",
    )
    .unwrap();
    assert_eq!(with_size["model"], "GZ-seedream-5.5-2K");
    assert_eq!(with_size["size"], "4K");
    assert_eq!(with_size["prompt"], "draw cat");
    assert_eq!(with_size["duration"], 1);
    assert_eq!(with_size["aspect_ratio"], "16:9");
    assert_eq!(
        with_size["images"],
        json!([
            "https://img.example/c.png",
            "https://img.example/b.png",
            "https://img.example/a.png",
        ])
    );
    assert!(with_size.get("image").is_none());
    assert!(with_size.get("reference_images").is_none());

    let without_size = rust_sync_proxy::openai_image::normalize_request_body_for_upstream(
        json!({
            "model": "seedream-5.5",
            "prompt": "draw dog",
            "reference_images": ["https://img.example/d.png"],
        }),
        false,
        "https://api.tianyue.xyz",
    )
    .unwrap();
    assert_eq!(without_size["model"], "GZ-seedream-5.5-2K");
    assert!(without_size.get("size").is_none());
    assert_eq!(without_size["images"], json!(["https://img.example/d.png"]));
    assert!(without_size.get("reference_images").is_none());
}

#[test]
fn tianyue_request_keeps_other_models_sizes_and_channels_unchanged() {
    for body in [
        json!({"model": "seedream-5-pro"}),
        json!({"model": "seedream-5-pro", "size": "4K"}),
        json!({"model": "seedream-5-pro", "size": 2}),
        json!({"model": "other-model", "size": "2K"}),
        json!({"model": "seedream-5-pro-1K", "size": "2K"}),
        json!({"model": "seedream-5-pro-2K", "size": "1K"}),
        json!({"model": "seedream-5.5-2K", "size": "2K"}),
        json!({"model": "GZ-seedream-5.5-2K", "size": "1K"}),
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

    let body =
        json!({"model": "seedream-5.5", "size": "4K", "image": ["https://img.example/a.png"]});
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
        assert_eq!(normalized["size"], body["size"]);
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
