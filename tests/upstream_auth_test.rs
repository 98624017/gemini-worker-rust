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
