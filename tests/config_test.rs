use std::collections::HashMap;
use std::time::Duration;

#[test]
fn defaults_match_go_proxy_expectations() {
    let cfg = rust_sync_proxy::config::Config::from_env_map(&HashMap::new()).unwrap();
    assert_eq!(cfg.port, 8787);
    assert_eq!(cfg.upstream_base_url, "https://magic666.top");
    assert_eq!(cfg.image_host_mode.as_str(), "legacy");
    assert_eq!(cfg.slow_log_threshold, Duration::from_millis(100_000));
    assert_eq!(cfg.image_fetch_timeout, Duration::from_millis(20_000));
    assert_eq!(cfg.upload_timeout, Duration::from_millis(10_000));
    assert_eq!(
        cfg.image_tls_handshake_timeout,
        Duration::from_millis(15_000)
    );
    assert_eq!(cfg.upload_tls_handshake_timeout, Duration::from_millis(10_000));
    assert!(!cfg.image_fetch_insecure_skip_verify);
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
        ("UPLOAD_INSECURE_SKIP_VERIFY".to_string(), "1".to_string()),
    ]);

    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();
    assert!(cfg.public_base_url.is_empty());
    assert!(cfg.admin_password.is_empty());
    assert!(cfg.inline_data_url_cache_dir.is_empty());
    assert_eq!(cfg.inline_data_url_memory_cache_max_bytes, 0);
    assert!(cfg.image_fetch_insecure_skip_verify);
    assert!(cfg.upload_insecure_skip_verify);
}

#[test]
fn invalid_port_falls_back_to_default_like_go() {
    let env = HashMap::from([("PORT".to_string(), "bad-port".to_string())]);
    let cfg = rust_sync_proxy::config::Config::from_env_map(&env).unwrap();
    assert_eq!(cfg.port, 8787);
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
