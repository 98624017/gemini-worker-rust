use serde_json::json;

#[test]
fn collects_unique_inline_data_urls_and_enforces_limit() {
    let body = json!({
        "contents": [{
            "parts": [
                {"inlineData": {"data": "https://img.example/a.png"}},
                {"inlineData": {"data": "https://img.example/a.png"}}
            ]
        }]
    });

    let scan = rust_sync_proxy::request_rewrite::scan_inline_data_urls(&body).unwrap();
    assert_eq!(scan.unique_urls.len(), 1);
    assert_eq!(scan.total_refs, 2);
}

#[test]
fn scan_inline_data_urls_allows_seven_url_refs() {
    let parts: Vec<_> = (0..7)
        .map(|index| {
            json!({
                "inlineData": {
                    "data": format!("https://img.example/{index}.png")
                }
            })
        })
        .collect();
    let body = json!({
        "contents": [{
            "parts": parts
        }]
    });

    let scan = rust_sync_proxy::request_rewrite::scan_inline_data_urls(&body).unwrap();
    assert_eq!(scan.total_refs, 7);
    assert_eq!(scan.unique_urls.len(), 7);
}

#[test]
fn scan_inline_data_urls_rejects_eight_url_refs() {
    let parts: Vec<_> = (0..8)
        .map(|index| {
            json!({
                "inlineData": {
                    "data": format!("https://img.example/{index}.png")
                }
            })
        })
        .collect();
    let body = json!({
        "contents": [{
            "parts": parts
        }]
    });

    let err = rust_sync_proxy::request_rewrite::scan_inline_data_urls(&body).unwrap_err();
    assert_eq!(err.to_string(), "too many inlineData URLs");
}

#[test]
fn rewrites_aiapidev_request_body_to_file_data_and_snake_case() {
    let body = json!({
        "contents": [{
            "parts": [
                {"text": "两张图片合并"},
                {
                    "inlineData": {
                        "data": "https://n.uguu.se/bCEfckeJ.png",
                        "mimeType": "image/png"
                    }
                }
            ],
            "role": "user"
        }],
        "generationConfig": {
            "imageConfig": {
                "aspectRatio": "3:4",
                "imageSize": "1K",
                "output": "url"
            },
            "responseModalities": ["IMAGE"]
        }
    });

    let rewritten = rust_sync_proxy::request_rewrite::rewrite_aiapidev_request_body(body, "", &[]);
    let serialized = serde_json::to_string(&rewritten).unwrap();

    assert!(serialized.contains("\"role\":\"user\",\"parts\""));
    assert!(rewritten.get("generationConfig").is_none());
    assert!(
        rewritten["generation_config"]["image_config"]
            .get("output")
            .is_none()
    );
    assert_eq!(
        rewritten["contents"][0]["parts"][1]["file_data"],
        json!({
            "file_uri": "https://n.uguu.se/bCEfckeJ.png",
            "mime_type": "image/png"
        })
    );
    assert_eq!(
        rewritten["generation_config"],
        json!({
            "image_config": {
                "aspect_ratio": "3:4",
                "image_size": "1K"
            },
            "response_modalities": ["IMAGE"]
        })
    );
}

#[test]
fn rewrites_aiapidev_request_body_base64_inline_data_to_snake_case_inline_data() {
    let body = json!({
        "contents": [{
            "parts": [{
                "inlineData": {
                    "data": "iVBORw0KGgoAAAANSUhEUgAAAAUA",
                    "mimeType": "image/png"
                }
            }],
            "role": "user"
        }]
    });

    let rewritten = rust_sync_proxy::request_rewrite::rewrite_aiapidev_request_body(body, "", &[]);

    assert_eq!(
        rewritten["contents"][0]["parts"][0]["inline_data"],
        json!({
            "data": "iVBORw0KGgoAAAANSUhEUgAAAAUA",
            "mime_type": "image/png"
        })
    );
    assert!(
        rewritten["contents"][0]["parts"][0]
            .get("inlineData")
            .is_none()
    );
}

#[test]
fn rewrites_aiapidev_request_body_url_inline_data_to_external_proxy_when_domain_matches() {
    let raw_url = "https://miratoon.oss-cn-hangzhou.aliyuncs.com/SHOT_VALUE_IMAGE/demo.jpg";
    let external_proxy_prefix = "https://proxy.example.com/fetch?url=";
    let encoded_raw_url: String =
        url::form_urlencoded::byte_serialize(raw_url.as_bytes()).collect();
    let body = json!({
        "contents": [{
            "parts": [{
                "inlineData": {
                    "data": raw_url,
                    "mimeType": "image/jpeg"
                }
            }],
            "role": "user"
        }]
    });

    let rewritten = rust_sync_proxy::request_rewrite::rewrite_aiapidev_request_body(
        body,
        external_proxy_prefix,
        &[".oss-cn-hangzhou.aliyuncs.com".to_string()],
    );

    assert_eq!(
        rewritten["contents"][0]["parts"][0]["file_data"],
        json!({
            "file_uri": format!(
                "{}{}",
                external_proxy_prefix,
                encoded_raw_url
            ),
            "mime_type": "image/jpeg"
        })
    );
}

#[test]
fn rewrites_aiapidev_request_body_uses_public_base_url_compatible_proxy_prefix() {
    let raw_url = "https://miratoon.oss-cn-hangzhou.aliyuncs.com/SHOT_VALUE_IMAGE/demo.jpg";
    let encoded_raw_url: String =
        url::form_urlencoded::byte_serialize(raw_url.as_bytes()).collect();
    let mut config = rust_sync_proxy::test_config();
    config.public_base_url = "https://proxy.example.com/base".to_string();
    let external_proxy_prefix = config.resolved_external_image_proxy_prefix();
    let body = json!({
        "contents": [{
            "parts": [{
                "inlineData": {
                    "data": raw_url,
                    "mimeType": "image/jpeg"
                }
            }],
            "role": "user"
        }]
    });

    let rewritten = rust_sync_proxy::request_rewrite::rewrite_aiapidev_request_body(
        body,
        &external_proxy_prefix,
        &[".oss-cn-hangzhou.aliyuncs.com".to_string()],
    );

    assert_eq!(
        rewritten["contents"][0]["parts"][0]["file_data"],
        json!({
            "file_uri": format!(
                "{}{}",
                external_proxy_prefix,
                encoded_raw_url
            ),
            "mime_type": "image/jpeg"
        })
    );
}

#[test]
fn rewrites_aiapidev_request_body_strips_query_before_external_proxy_when_domain_matches() {
    let raw_url = "https://miratoon.oss-cn-hangzhou.aliyuncs.com/SHOT_VALUE_IMAGE/demo.jpg?x-oss-date=20260415T120247Z&x-oss-signature=demo#fragment";
    let stripped_url = "https://miratoon.oss-cn-hangzhou.aliyuncs.com/SHOT_VALUE_IMAGE/demo.jpg";
    let external_proxy_prefix = "https://proxy.example.com/base/proxy/image?url=";
    let encoded_stripped_url: String =
        url::form_urlencoded::byte_serialize(stripped_url.as_bytes()).collect();
    let body = json!({
        "contents": [{
            "parts": [{
                "inlineData": {
                    "data": raw_url,
                    "mimeType": "image/jpeg"
                }
            }],
            "role": "user"
        }]
    });

    let rewritten = rust_sync_proxy::request_rewrite::rewrite_aiapidev_request_body(
        body,
        external_proxy_prefix,
        &[".oss-cn-hangzhou.aliyuncs.com".to_string()],
    );

    assert_eq!(
        rewritten["contents"][0]["parts"][0]["file_data"],
        json!({
            "file_uri": format!(
                "{}{}",
                external_proxy_prefix,
                encoded_stripped_url
            ),
            "mime_type": "image/jpeg"
        })
    );
}

#[test]
fn rewrites_aiapidev_request_body_keeps_query_for_non_matching_domain() {
    let raw_url = "https://img.example.com/demo.jpg?token=demo#fragment";
    let body = json!({
        "contents": [{
            "parts": [{
                "inlineData": {
                    "data": raw_url,
                    "mimeType": "image/jpeg"
                }
            }],
            "role": "user"
        }]
    });

    let rewritten = rust_sync_proxy::request_rewrite::rewrite_aiapidev_request_body(
        body,
        "",
        &[".oss-cn-hangzhou.aliyuncs.com".to_string()],
    );

    assert_eq!(
        rewritten["contents"][0]["parts"][0]["file_data"],
        json!({
            "file_uri": raw_url,
            "mime_type": "image/jpeg"
        })
    );
}

#[test]
fn rewrites_aiapidev_request_body_keeps_already_proxied_url_without_double_wrapping() {
    let external_proxy_prefix = "https://proxy.example.com/base/proxy/image?url=";
    let proxied_url = "https://proxy.example.com/base/proxy/image?url=https%3A%2F%2Fmiratoon.oss-cn-hangzhou.aliyuncs.com%2FSHOT_VALUE_IMAGE%2Fdemo.jpg";
    let body = json!({
        "contents": [{
            "parts": [{
                "inlineData": {
                    "data": proxied_url,
                    "mimeType": "image/jpeg"
                }
            }],
            "role": "user"
        }]
    });

    let rewritten = rust_sync_proxy::request_rewrite::rewrite_aiapidev_request_body(
        body,
        external_proxy_prefix,
        &[".oss-cn-hangzhou.aliyuncs.com".to_string()],
    );

    assert_eq!(
        rewritten["contents"][0]["parts"][0]["file_data"],
        json!({
            "file_uri": proxied_url,
            "mime_type": "image/jpeg"
        })
    );
}
