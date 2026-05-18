use std::collections::HashMap;
use std::time::Duration;

#[test]
fn defaults_match_runtime_expectations() {
    let cfg = rust_sync_proxy::config::Config::from_env_map(&HashMap::new()).unwrap();
    assert_eq!(cfg.port, 8787);
    assert_eq!(cfg.upstream_base_url, "https://magic666.top");
    assert_eq!(cfg.upstream_timeout, Duration::from_millis(600_000));
    assert_eq!(cfg.upstream_connect_timeout, Duration::from_millis(30_000));
    assert_eq!(cfg.upstream_tcp_keepalive, Duration::from_millis(30_000));
    assert_eq!(
        cfg.upstream_pool_idle_timeout,
        Duration::from_millis(15_000)
    );
    assert_eq!(cfg.upstream_block_cache_ttl, Duration::from_millis(300_000));
    assert_eq!(cfg.upstream_block_cache_max_entries, 1024);
    assert_eq!(cfg.image_host_mode.as_str(), "legacy");
    assert_eq!(cfg.slow_log_threshold, Duration::from_millis(100_000));
    assert_eq!(cfg.image_fetch_timeout, Duration::from_millis(20_000));
    assert_eq!(cfg.upload_timeout, Duration::from_millis(20_000));
    assert!(!cfg.enable_image_compression);
    assert!(!cfg.enable_request_image_webp_optimization);
    assert_eq!(
        cfg.image_tls_handshake_timeout,
        Duration::from_millis(15_000)
    );
    assert_eq!(
        cfg.upload_tls_handshake_timeout,
        Duration::from_millis(10_000)
    );
    assert!(!cfg.image_fetch_insecure_skip_verify);
    assert!(!cfg.upstream_insecure_skip_verify);
    assert!(!cfg.upload_insecure_skip_verify);
    assert_eq!(
        cfg.inline_data_url_memory_cache_max_bytes,
        100 * 1024 * 1024
    );
    assert_eq!(
        cfg.inline_data_url_background_fetch_wait_timeout,
        cfg.image_fetch_timeout
    );
    assert_eq!(cfg.legacy_uguu_upload_url, "https://uguu.se/upload");
    assert_eq!(
        cfg.legacy_kefan_upload_url,
        "https://ai.kefan.cn/api/upload/local"
    );
    assert!(cfg.openai_image_b64_json_upstream_domains.is_empty());
    assert_eq!(
        cfg.openai_image_edits_upstream_domains,
        vec!["happyapi.org".to_string(), "www.happyapi.org".to_string()]
    );
    assert!(cfg.openai_image_upstream_url_proxy_prefix.is_empty());
    assert!(!cfg.proxy_r2_output_urls);
}

#[test]
fn test_config_uses_runtime_defaults_except_test_api_key() {
    let mut expected = rust_sync_proxy::config::Config::from_env_map(&HashMap::new()).unwrap();
    expected.upstream_api_key = "test-upstream-key".to_string();
    let test = rust_sync_proxy::test_config();

    assert_eq!(test, expected);
}

#[test]
fn disabled_values_follow_go_semantics() {
    let env = HashMap::from([
        ("PUBLIC_BASE_URL".to_string(), "off".to_string()),
        ("ADMIN_PASSWORD".to_string(), "disabled".to_string()),
        ("INLINE_DATA_URL_CACHE_DIR".to_string(), "none".to_string()),
        (
            "INLINE_DATA_URL_MEMORY_CACHE_MAX_BYTES".to_string(),
            "off".to_string(),
        ),
        (
            "IMAGE_FETCH_INSECURE_SKIP_VERIFY".to_string(),
            "true".to_string(),
        ),
        (
            "UPSTREAM_INSECURE_SKIP_VERIFY".to_string(),
            "true".to_string(),
        ),
        ("UPLOAD_INSECURE_SKIP_VERIFY".to_string(), "1".to_string()),
    ]);

    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();
    assert!(cfg.public_base_url.is_empty());
    assert!(cfg.admin_password.is_empty());
    assert!(cfg.inline_data_url_cache_dir.is_empty());
    assert_eq!(cfg.inline_data_url_memory_cache_max_bytes, 0);
    assert!(cfg.image_fetch_insecure_skip_verify);
    assert!(cfg.upstream_insecure_skip_verify);
    assert!(cfg.upload_insecure_skip_verify);
}

#[test]
fn image_compression_flag_can_be_enabled_from_env() {
    let env = HashMap::from([("ENABLE_IMAGE_COMPRESSION".to_string(), "true".to_string())]);
    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();
    assert!(cfg.enable_image_compression);
}

#[test]
fn request_image_webp_optimization_flag_can_be_enabled_from_env() {
    let env = HashMap::from([(
        "ENABLE_REQUEST_IMAGE_WEBP_OPTIMIZATION".to_string(),
        "true".to_string(),
    )]);
    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();
    assert!(cfg.enable_request_image_webp_optimization);
}

#[test]
fn image_compression_jpeg_quality_can_be_overridden_from_env() {
    let env = HashMap::from([(
        "IMAGE_COMPRESSION_JPEG_QUALITY".to_string(),
        "100".to_string(),
    )]);
    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();
    assert_eq!(cfg.image_compression_jpeg_quality, 100);
}

#[test]
fn invalid_port_falls_back_to_default_like_go() {
    let env = HashMap::from([("PORT".to_string(), "bad-port".to_string())]);
    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();
    assert_eq!(cfg.port, 8787);
}

#[test]
fn upstream_http_timeouts_can_be_overridden_from_env() {
    let env = HashMap::from([
        ("UPSTREAM_TIMEOUT_MS".to_string(), "65432".to_string()),
        (
            "UPSTREAM_CONNECT_TIMEOUT_MS".to_string(),
            "4321".to_string(),
        ),
        ("UPSTREAM_TCP_KEEPALIVE_MS".to_string(), "21000".to_string()),
        (
            "UPSTREAM_POOL_IDLE_TIMEOUT_MS".to_string(),
            "9000".to_string(),
        ),
    ]);

    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();
    assert_eq!(cfg.upstream_timeout, Duration::from_millis(65_432));
    assert_eq!(cfg.upstream_connect_timeout, Duration::from_millis(4_321));
    assert_eq!(cfg.upstream_tcp_keepalive, Duration::from_millis(21_000));
    assert_eq!(cfg.upstream_pool_idle_timeout, Duration::from_millis(9_000));
}

#[test]
fn upstream_block_cache_defaults_to_five_minutes_and_1024_entries() {
    let cfg = rust_sync_proxy::config::Config::from_env_map(&HashMap::new()).unwrap();

    assert_eq!(cfg.upstream_block_cache_ttl, Duration::from_millis(300_000));
    assert_eq!(cfg.upstream_block_cache_max_entries, 1024);
}

#[test]
fn upstream_block_cache_can_be_configured_from_env() {
    let env = HashMap::from([
        (
            "UPSTREAM_BLOCK_CACHE_TTL_MS".to_string(),
            "12345".to_string(),
        ),
        (
            "UPSTREAM_BLOCK_CACHE_MAX_ENTRIES".to_string(),
            "17".to_string(),
        ),
    ]);

    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();

    assert_eq!(cfg.upstream_block_cache_ttl, Duration::from_millis(12_345));
    assert_eq!(cfg.upstream_block_cache_max_entries, 17);
}

#[test]
fn upstream_block_cache_can_be_disabled_from_env() {
    let env = HashMap::from([
        ("UPSTREAM_BLOCK_CACHE_TTL_MS".to_string(), "0".to_string()),
        (
            "UPSTREAM_BLOCK_CACHE_MAX_ENTRIES".to_string(),
            "0".to_string(),
        ),
    ]);

    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();

    assert_eq!(cfg.upstream_block_cache_ttl, Duration::from_millis(0));
    assert_eq!(cfg.upstream_block_cache_max_entries, 0);
}

#[test]
fn background_fetch_wait_timeout_defaults_to_image_fetch_timeout() {
    let env = HashMap::from([("IMAGE_FETCH_TIMEOUT_MS".to_string(), "3456".to_string())]);
    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();
    assert_eq!(cfg.image_fetch_timeout, Duration::from_millis(3456));
    assert_eq!(
        cfg.inline_data_url_background_fetch_wait_timeout,
        Duration::from_millis(3456)
    );
}

#[test]
fn public_base_url_can_fallback_to_legacy_proxy_prefix() {
    let env = HashMap::from([(
        "PUBLIC_BASE_URL".to_string(),
        "https://proxy.example.com/".to_string(),
    )]);
    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();
    assert_eq!(
        cfg.resolved_external_image_proxy_prefix(),
        "https://proxy.example.com/proxy/image?url="
    );
}

#[test]
fn external_image_proxy_prefix_still_overrides_public_base_url() {
    let env = HashMap::from([
        (
            "PUBLIC_BASE_URL".to_string(),
            "https://proxy.example.com".to_string(),
        ),
        (
            "EXTERNAL_IMAGE_PROXY_PREFIX".to_string(),
            "https://external.example.com/fetch?url=".to_string(),
        ),
    ]);
    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();
    assert_eq!(
        cfg.resolved_external_image_proxy_prefix(),
        "https://external.example.com/fetch?url="
    );
}

#[test]
fn openai_image_upstream_url_proxy_prefix_can_be_loaded_from_env() {
    let env = HashMap::from([(
        "OPENAI_IMAGE_UPSTREAM_URL_PROXY_PREFIX".to_string(),
        "https://openai-proxy.example.com/fetch?url=".to_string(),
    )]);

    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();

    assert_eq!(
        cfg.openai_image_upstream_url_proxy_prefix,
        "https://openai-proxy.example.com/fetch?url="
    );
}

#[test]
fn proxy_r2_output_urls_can_be_enabled_from_env() {
    let env = HashMap::from([("PROXY_R2_OUTPUT_URLS".to_string(), "1".to_string())]);

    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();

    assert!(cfg.proxy_r2_output_urls);
}

#[test]
fn proxy_r2_output_urls_can_be_disabled_from_env() {
    let env = HashMap::from([("PROXY_R2_OUTPUT_URLS".to_string(), "0".to_string())]);

    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();

    assert!(!cfg.proxy_r2_output_urls);
}

#[test]
fn proxy_r2_output_urls_defaults_to_enabled_when_public_base_url_exists() {
    let env = HashMap::from([(
        "PUBLIC_BASE_URL".to_string(),
        "https://proxy.example.com".to_string(),
    )]);

    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();

    assert!(cfg.proxy_r2_output_urls);
}

#[test]
fn proxy_r2_output_urls_defaults_to_enabled_when_external_proxy_prefix_exists() {
    let env = HashMap::from([(
        "EXTERNAL_IMAGE_PROXY_PREFIX".to_string(),
        "https://proxy.example.com/fetch?url=".to_string(),
    )]);

    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();

    assert!(cfg.proxy_r2_output_urls);
}

#[test]
fn proxy_r2_output_urls_explicit_zero_overrides_public_base_url_default() {
    let env = HashMap::from([
        (
            "PUBLIC_BASE_URL".to_string(),
            "https://proxy.example.com".to_string(),
        ),
        ("PROXY_R2_OUTPUT_URLS".to_string(), "0".to_string()),
    ]);

    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();

    assert!(!cfg.proxy_r2_output_urls);
}

#[test]
fn proxy_r2_output_urls_defaults_to_disabled_when_standard_proxy_flag_is_disabled() {
    let env = HashMap::from([
        (
            "PUBLIC_BASE_URL".to_string(),
            "https://proxy.example.com".to_string(),
        ),
        ("PROXY_STANDARD_OUTPUT_URLS".to_string(), "0".to_string()),
    ]);

    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();

    assert!(!cfg.proxy_r2_output_urls);
}

#[test]
fn invalid_r2_endpoint_is_rejected_during_config_load() {
    let env = HashMap::from([
        ("IMAGE_HOST_MODE".to_string(), "r2".to_string()),
        ("R2_ENDPOINT".to_string(), "ftp://example.com".to_string()),
        ("R2_BUCKET".to_string(), "bucket".to_string()),
        ("R2_ACCESS_KEY_ID".to_string(), "key".to_string()),
        ("R2_SECRET_ACCESS_KEY".to_string(), "secret".to_string()),
        (
            "R2_PUBLIC_BASE_URL".to_string(),
            "https://img.example.com".to_string(),
        ),
    ]);

    let err = rust_sync_proxy::config::Config::from_env_map(&env).unwrap_err();
    assert!(err.to_string().contains("R2_ENDPOINT"));
}

#[test]
fn legacy_upload_env_vars_do_not_change_runtime_config() {
    let env = HashMap::from([
        (
            "LEGACY_UGUU_UPLOAD_URL".to_string(),
            "https://override.example/uguu".to_string(),
        ),
        (
            "LEGACY_KEFAN_UPLOAD_URL".to_string(),
            "https://override.example/kefan".to_string(),
        ),
    ]);

    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();
    assert_eq!(cfg.legacy_uguu_upload_url, "https://uguu.se/upload");
    assert_eq!(
        cfg.legacy_kefan_upload_url,
        "https://ai.kefan.cn/api/upload/local"
    );
}

#[test]
fn openai_image_b64_json_upstream_domains_can_be_loaded_from_env() {
    let env = HashMap::from([(
        "OPENAI_IMAGE_B64_JSON_UPSTREAM_DOMAINS".to_string(),
        "api.example.com,.example.org".to_string(),
    )]);

    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();

    assert_eq!(
        cfg.openai_image_b64_json_upstream_domains,
        vec!["api.example.com".to_string(), ".example.org".to_string()]
    );
}

#[test]
fn openai_image_b64_json_upstream_domains_match_exact_and_suffix_hosts() {
    let mut cfg = rust_sync_proxy::test_config();
    cfg.openai_image_b64_json_upstream_domains =
        vec!["api.example.com".to_string(), ".example.org".to_string()];

    assert!(cfg.should_force_openai_image_b64_json_for_upstream("https://api.example.com"));
    assert!(!cfg.should_force_openai_image_b64_json_for_upstream("https://foo.api.example.com"));
    assert!(cfg.should_force_openai_image_b64_json_for_upstream("https://img.example.org"));
    assert!(!cfg.should_force_openai_image_b64_json_for_upstream("https://example.org"));
    assert!(!cfg.should_force_openai_image_b64_json_for_upstream("https://example.net"));
}

#[test]
fn openai_image_edits_upstream_domains_can_be_loaded_from_env() {
    let env = HashMap::from([(
        "OPENAI_IMAGE_EDITS_UPSTREAM_DOMAINS".to_string(),
        "api.example.com,.example.org".to_string(),
    )]);

    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();

    assert_eq!(
        cfg.openai_image_edits_upstream_domains,
        vec!["api.example.com".to_string(), ".example.org".to_string()]
    );
}

#[test]
fn openai_image_edits_upstream_domains_match_exact_and_suffix_hosts() {
    let mut cfg = rust_sync_proxy::test_config();
    cfg.openai_image_edits_upstream_domains =
        vec!["api.example.com".to_string(), ".example.org".to_string()];

    assert!(cfg.should_use_openai_image_edits_for_upstream("https://api.example.com"));
    assert!(!cfg.should_use_openai_image_edits_for_upstream("https://foo.api.example.com"));
    assert!(cfg.should_use_openai_image_edits_for_upstream("https://img.example.org"));
    assert!(!cfg.should_use_openai_image_edits_for_upstream("https://example.org"));
    assert!(!cfg.should_use_openai_image_edits_for_upstream("https://example.net"));
}
