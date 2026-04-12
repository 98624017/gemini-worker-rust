pub mod admin;
pub mod allocator;
pub mod blob_runtime;
pub mod cache;
pub mod config;
pub mod http;
pub mod image_io;
pub mod request_encode;
pub mod request_materialize;
pub mod request_rewrite;
pub mod request_scan;
pub mod response_materialize;
pub mod response_rewrite;
pub mod upload;
pub mod upstream;

pub use blob_runtime::{BlobHandle, BlobRuntime, BlobRuntimeConfig, BlobStorage};
pub use http::build_router;

use config::Config;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub fn test_config() -> Config {
    Config {
        port: 8787,
        upstream_base_url: "https://magic666.top".to_string(),
        upstream_api_key: "test-upstream-key".to_string(),
        upstream_timeout: Duration::from_millis(600_000),
        upstream_connect_timeout: Duration::from_millis(10_000),
        upstream_tcp_keepalive: Duration::from_millis(30_000),
        upstream_pool_idle_timeout: Duration::from_millis(15_000),
        image_host_mode: "legacy".to_string(),
        allowed_proxy_domains: vec![
            "ai.kefan.cn".to_string(),
            "uguu.se".to_string(),
            ".uguu.se".to_string(),
            ".aitohumanize.com".to_string(),
        ],
        public_base_url: String::new(),
        external_image_proxy_prefix: String::new(),
        slow_log_threshold: Duration::from_millis(100_000),
        proxy_standard_output_urls: true,
        proxy_special_upstream_urls: true,
        enable_image_compression: false,
        image_compression_jpeg_quality: 97,
        admin_password: String::new(),
        image_fetch_timeout: Duration::from_millis(20_000),
        image_tls_handshake_timeout: Duration::from_millis(15_000),
        image_fetch_insecure_skip_verify: false,
        image_fetch_external_proxy_domains: Vec::new(),
        inline_data_url_cache_dir: String::new(),
        inline_data_url_cache_ttl: Duration::from_millis(3_600_000),
        inline_data_url_cache_max_bytes: 1 << 30,
        inline_data_url_memory_cache_max_bytes: 100 * 1024 * 1024,
        inline_data_url_background_fetch_wait_timeout: Duration::from_millis(20_000),
        inline_data_url_background_fetch_total_timeout: Duration::from_millis(90_000),
        inline_data_url_background_fetch_max_inflight: 128,
        blob_inline_max_bytes: 8 * 1024 * 1024,
        blob_request_hot_budget_bytes: 24 * 1024 * 1024,
        blob_global_hot_budget_bytes: 384 * 1024 * 1024,
        blob_spill_dir: "/tmp/rust-sync-proxy-blobs".to_string(),
        upload_timeout: Duration::from_millis(10_000),
        upload_tls_handshake_timeout: Duration::from_millis(10_000),
        upload_insecure_skip_verify: false,
        legacy_uguu_upload_url: "https://uguu.se/upload".to_string(),
        legacy_kefan_upload_url: "https://ai.kefan.cn/api/upload/local".to_string(),
        r2_endpoint: String::new(),
        r2_bucket: String::new(),
        r2_access_key_id: String::new(),
        r2_secret_access_key: String::new(),
        r2_public_base_url: String::new(),
        r2_object_prefix: "images".to_string(),
    }
}

pub fn test_blob_runtime(inline_max_bytes: u64) -> BlobRuntime {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);

    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let spill_dir = std::env::temp_dir().join(format!("rust-sync-proxy-test-blobs-{id}"));
    BlobRuntime::new(BlobRuntimeConfig {
        inline_max_bytes,
        request_hot_budget_bytes: 24 * 1024 * 1024,
        global_hot_budget_bytes: 384 * 1024 * 1024,
        spill_dir,
    })
}
