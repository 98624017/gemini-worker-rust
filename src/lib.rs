pub mod admin;
pub mod allocator;
pub mod blob_runtime;
pub mod cache;
pub mod config;
pub mod http;
pub mod image_io;
pub mod openai_image;
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

pub fn test_config() -> Config {
    let mut config = Config::from_env_map(&std::collections::HashMap::new())
        .expect("empty config map should produce default test config");
    config.upstream_api_key = "test-upstream-key".to_string();
    config
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
