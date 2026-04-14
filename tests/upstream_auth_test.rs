use serde_json::json;

#[test]
fn bearer_token_can_override_base_url_and_api_key() {
    let headers = [("authorization", "Bearer https://demo.example|secret")];
    let resolved =
        rust_sync_proxy::upstream::resolve_upstream(headers, "https://magic666.top", "env-key")
            .unwrap();
    assert_eq!(resolved.base_url, "https://demo.example");
    assert_eq!(resolved.api_key, "secret");
}

#[test]
fn dual_upstream_token_uses_second_group_for_4k_requests() {
    let headers = [(
        "x-goog-api-key",
        "https://first.example|first-key,https://second.example|second-key",
    )];
    let request_body = json!({
        "generationConfig": {
            "imageConfig": {
                "imageSize": "4k"
            }
        }
    });
    let resolved = rust_sync_proxy::upstream::resolve_upstream_for_request(
        headers,
        &request_body,
        "https://magic666.top",
        "env-key",
    )
    .unwrap();

    assert_eq!(resolved.base_url, "https://second.example");
    assert_eq!(resolved.api_key, "second-key");
}

#[test]
fn dual_upstream_token_uses_first_group_for_non_4k_requests() {
    let headers = [(
        "x-goog-api-key",
        "https://first.example|first-key,https://second.example|second-key",
    )];
    let request_body = json!({
        "generationConfig": {
            "imageConfig": {
                "imageSize": "2k"
            }
        }
    });
    let resolved = rust_sync_proxy::upstream::resolve_upstream_for_request(
        headers,
        &request_body,
        "https://magic666.top",
        "env-key",
    )
    .unwrap();

    assert_eq!(resolved.base_url, "https://first.example");
    assert_eq!(resolved.api_key, "first-key");
}

#[test]
fn malformed_dual_upstream_token_returns_error() {
    let headers = [(
        "x-goog-api-key",
        "https://first.example|first-key,second-key-only",
    )];
    let request_body = json!({
        "generationConfig": {
            "imageConfig": {
                "imageSize": "4K"
            }
        }
    });
    let err = rust_sync_proxy::upstream::resolve_upstream_for_request(
        headers,
        &request_body,
        "https://magic666.top",
        "env-key",
    )
    .unwrap_err();

    assert!(
        err.to_string().contains("dual upstream"),
        "unexpected error: {err}"
    );
}

#[test]
fn aiapidev_base_url_is_detected() {
    assert!(rust_sync_proxy::upstream::is_aiapidev_base_url(
        "https://www.aiapidev.com"
    ));
    assert!(rust_sync_proxy::upstream::is_aiapidev_base_url(
        "https://aiapidev.com"
    ));
    assert!(!rust_sync_proxy::upstream::is_aiapidev_base_url(
        "https://example.com"
    ));
}

#[test]
fn aiapidev_model_path_is_mapped() {
    assert_eq!(
        rust_sync_proxy::upstream::rewrite_aiapidev_model_path(
            "/v1beta/models/gemini-3-pro-image-preview:generateContent"
        ),
        "/v1beta/models/nanobananapro:generateContent"
    );
    assert_eq!(
        rust_sync_proxy::upstream::rewrite_aiapidev_model_path(
            "/v1beta/models/gemini-3.1-flash-image-preview:generateContent"
        ),
        "/v1beta/models/nanobanana2:generateContent"
    );
    assert_eq!(
        rust_sync_proxy::upstream::rewrite_aiapidev_model_path(
            "/v1beta/models/other-model:generateContent"
        ),
        "/v1beta/models/other-model:generateContent"
    );
}
