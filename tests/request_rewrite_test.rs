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

    let rewritten = rust_sync_proxy::request_rewrite::rewrite_aiapidev_request_body(body);
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

    let rewritten = rust_sync_proxy::request_rewrite::rewrite_aiapidev_request_body(body);

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
