#[test]
fn bearer_token_can_override_base_url_and_api_key() {
    let headers = [("authorization", "Bearer https://demo.example|secret")];
    let resolved =
        rust_sync_proxy::upstream::resolve_upstream(headers, "https://magic666.top", "env-key")
            .unwrap();
    assert_eq!(resolved.base_url, "https://demo.example");
    assert_eq!(resolved.api_key, "secret");
}
