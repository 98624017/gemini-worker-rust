use std::collections::HashMap;
use std::env;
use std::time::Duration;

use anyhow::{Result, anyhow};
use url::Url;

const DEFAULT_UPSTREAM_BASE_URL: &str = "https://magic666.top";
const DEFAULT_PORT: u16 = 8787;
const DEFAULT_UPSTREAM_TIMEOUT_MS: u64 = 600_000;
const DEFAULT_UPSTREAM_CONNECT_TIMEOUT_MS: u64 = 10_000;
const DEFAULT_UPSTREAM_TCP_KEEPALIVE_MS: u64 = 30_000;
const DEFAULT_UPSTREAM_POOL_IDLE_TIMEOUT_MS: u64 = 15_000;
const DEFAULT_SLOW_LOG_THRESHOLD_MS: u64 = 100_000;
const DEFAULT_IMAGE_FETCH_TIMEOUT_MS: u64 = 20_000;
const DEFAULT_UPLOAD_TIMEOUT_MS: u64 = 20_000;
const DEFAULT_IMAGE_TLS_HANDSHAKE_TIMEOUT_MS: u64 = 15_000;
const DEFAULT_UPLOAD_TLS_HANDSHAKE_TIMEOUT_MS: u64 = 10_000;
const DEFAULT_INSTANCE_MEMORY_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const DEFAULT_INLINE_DATA_URL_CACHE_TTL_MS: u64 = 3_600_000;
const DEFAULT_INLINE_DATA_URL_CACHE_MAX_BYTES: u64 = 1 << 30;
const DEFAULT_INLINE_DATA_URL_MEMORY_CACHE_MAX_BYTES: u64 = 100 * 1024 * 1024;
const DEFAULT_INLINE_DATA_URL_BACKGROUND_FETCH_TOTAL_TIMEOUT_MS: u64 = 90_000;
const DEFAULT_INLINE_DATA_URL_BACKGROUND_FETCH_MAX_INFLIGHT: usize = 128;
const DEFAULT_BLOB_INLINE_MAX_BYTES: u64 = 8 * 1024 * 1024;
const DEFAULT_BLOB_REQUEST_HOT_BUDGET_BYTES: u64 = 24 * 1024 * 1024;
const DEFAULT_BLOB_GLOBAL_HOT_BUDGET_BYTES: u64 = 384 * 1024 * 1024;
const DEFAULT_BLOB_SPILL_DIR: &str = "/tmp/rust-sync-proxy-blobs";
const DEFAULT_LEGACY_UGUU_UPLOAD_URL: &str = "https://uguu.se/upload";
const DEFAULT_LEGACY_KEFAN_UPLOAD_URL: &str = "https://ai.kefan.cn/api/upload/local";
const DEFAULT_IMAGE_COMPRESSION_JPEG_QUALITY: u8 = 97;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlobBudgetDefaults {
    pub inline_max_bytes: u64,
    pub request_hot_budget_bytes: u64,
    pub global_hot_budget_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub port: u16,
    pub upstream_base_url: String,
    pub upstream_api_key: String,
    pub upstream_timeout: Duration,
    pub upstream_connect_timeout: Duration,
    pub upstream_tcp_keepalive: Duration,
    pub upstream_pool_idle_timeout: Duration,
    pub image_host_mode: String,
    pub allowed_proxy_domains: Vec<String>,
    pub public_base_url: String,
    pub external_image_proxy_prefix: String,
    pub slow_log_threshold: Duration,
    pub proxy_standard_output_urls: bool,
    pub proxy_special_upstream_urls: bool,
    pub enable_image_compression: bool,
    pub enable_request_image_webp_optimization: bool,
    pub image_compression_jpeg_quality: u8,
    pub admin_password: String,
    pub image_fetch_timeout: Duration,
    pub image_tls_handshake_timeout: Duration,
    pub image_fetch_insecure_skip_verify: bool,
    pub image_fetch_external_proxy_domains: Vec<String>,
    pub inline_data_url_cache_dir: String,
    pub inline_data_url_cache_ttl: Duration,
    pub inline_data_url_cache_max_bytes: u64,
    pub inline_data_url_memory_cache_max_bytes: u64,
    pub inline_data_url_background_fetch_wait_timeout: Duration,
    pub inline_data_url_background_fetch_total_timeout: Duration,
    pub inline_data_url_background_fetch_max_inflight: usize,
    pub blob_inline_max_bytes: u64,
    pub blob_request_hot_budget_bytes: u64,
    pub blob_global_hot_budget_bytes: u64,
    pub blob_spill_dir: String,
    pub upload_timeout: Duration,
    pub upload_tls_handshake_timeout: Duration,
    pub upload_insecure_skip_verify: bool,
    pub legacy_uguu_upload_url: String,
    pub legacy_kefan_upload_url: String,
    pub r2_endpoint: String,
    pub r2_bucket: String,
    pub r2_access_key_id: String,
    pub r2_secret_access_key: String,
    pub r2_public_base_url: String,
    pub r2_object_prefix: String,
}

impl Config {
    pub fn from_process_env() -> Result<Self> {
        let env_map = env::vars().collect::<HashMap<_, _>>();
        Self::from_env_map(&env_map)
    }

    pub fn from_env_map(env_map: &HashMap<String, String>) -> Result<Self> {
        let blob_budget_defaults =
            blob_budget_defaults_for_memory(parse_non_negative_u64_with_default(
                env_map.get("INSTANCE_MEMORY_BYTES"),
                DEFAULT_INSTANCE_MEMORY_BYTES,
            ));
        let port = env_map
            .get("PORT")
            .map(String::as_str)
            .and_then(parse_port)
            .unwrap_or(DEFAULT_PORT);

        let upstream_base_url = env_map
            .get("UPSTREAM_BASE_URL")
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
            .unwrap_or(DEFAULT_UPSTREAM_BASE_URL)
            .to_string();
        let upstream_timeout_ms_raw = parse_positive_u64_with_default(
            env_map.get("UPSTREAM_TIMEOUT_MS"),
            DEFAULT_UPSTREAM_TIMEOUT_MS,
        );

        let image_host_mode = env_map
            .get("IMAGE_HOST_MODE")
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
            .unwrap_or("legacy")
            .to_ascii_lowercase();

        let allowed_proxy_domains = env_map
            .get("ALLOWED_PROXY_DOMAINS")
            .map(String::as_str)
            .map(parse_csv)
            .filter(|domains| !domains.is_empty())
            .unwrap_or_else(default_allowed_proxy_domains);
        let image_fetch_timeout_ms_raw = parse_positive_u64_with_default(
            env_map.get("IMAGE_FETCH_TIMEOUT_MS"),
            DEFAULT_IMAGE_FETCH_TIMEOUT_MS,
        );

        let config = Self {
            port,
            upstream_base_url,
            upstream_api_key: env_map.get("UPSTREAM_API_KEY").cloned().unwrap_or_default(),
            upstream_timeout: Duration::from_millis(upstream_timeout_ms_raw),
            upstream_connect_timeout: Duration::from_millis(parse_positive_u64_with_default(
                env_map.get("UPSTREAM_CONNECT_TIMEOUT_MS"),
                DEFAULT_UPSTREAM_CONNECT_TIMEOUT_MS,
            )),
            upstream_tcp_keepalive: Duration::from_millis(parse_positive_u64_with_default(
                env_map.get("UPSTREAM_TCP_KEEPALIVE_MS"),
                DEFAULT_UPSTREAM_TCP_KEEPALIVE_MS,
            )),
            upstream_pool_idle_timeout: Duration::from_millis(parse_positive_u64_with_default(
                env_map.get("UPSTREAM_POOL_IDLE_TIMEOUT_MS"),
                DEFAULT_UPSTREAM_POOL_IDLE_TIMEOUT_MS,
            )),
            image_host_mode,
            allowed_proxy_domains,
            public_base_url: env_map
                .get("PUBLIC_BASE_URL")
                .map(String::as_str)
                .map(normalize_optional_http_base_url)
                .unwrap_or_default(),
            external_image_proxy_prefix: env_map
                .get("EXTERNAL_IMAGE_PROXY_PREFIX")
                .map(String::as_str)
                .map(parse_optional_string_with_disabled)
                .unwrap_or_default(),
            slow_log_threshold: Duration::from_millis(parse_non_negative_u64_with_default(
                env_map.get("SLOW_LOG_THRESHOLD_MS"),
                DEFAULT_SLOW_LOG_THRESHOLD_MS,
            )),
            proxy_standard_output_urls: parse_bool(env_map.get("PROXY_STANDARD_OUTPUT_URLS"), true),
            proxy_special_upstream_urls: parse_bool(
                env_map.get("PROXY_SPECIAL_UPSTREAM_URLS"),
                true,
            ),
            enable_image_compression: parse_bool(env_map.get("ENABLE_IMAGE_COMPRESSION"), false),
            enable_request_image_webp_optimization: parse_bool(
                env_map.get("ENABLE_REQUEST_IMAGE_WEBP_OPTIMIZATION"),
                false,
            ),
            image_compression_jpeg_quality: parse_jpeg_quality(
                env_map.get("IMAGE_COMPRESSION_JPEG_QUALITY"),
                DEFAULT_IMAGE_COMPRESSION_JPEG_QUALITY,
            ),
            admin_password: env_map
                .get("ADMIN_PASSWORD")
                .map(String::as_str)
                .map(parse_optional_string_with_disabled)
                .unwrap_or_default(),
            image_fetch_timeout: Duration::from_millis(image_fetch_timeout_ms_raw),
            image_tls_handshake_timeout: Duration::from_millis(parse_positive_u64_with_default(
                env_map.get("IMAGE_TLS_HANDSHAKE_TIMEOUT_MS"),
                DEFAULT_IMAGE_TLS_HANDSHAKE_TIMEOUT_MS,
            )),
            image_fetch_insecure_skip_verify: parse_bool(
                env_map.get("IMAGE_FETCH_INSECURE_SKIP_VERIFY"),
                false,
            ),
            image_fetch_external_proxy_domains: env_map
                .get("IMAGE_FETCH_EXTERNAL_PROXY_DOMAINS")
                .map(String::as_str)
                .map(parse_csv)
                .unwrap_or_default(),
            inline_data_url_cache_dir: env_map
                .get("INLINE_DATA_URL_CACHE_DIR")
                .map(String::as_str)
                .map(parse_optional_string_with_disabled)
                .unwrap_or_default(),
            inline_data_url_cache_ttl: Duration::from_millis(parse_non_negative_u64_with_default(
                env_map.get("INLINE_DATA_URL_CACHE_TTL_MS"),
                DEFAULT_INLINE_DATA_URL_CACHE_TTL_MS,
            )),
            inline_data_url_cache_max_bytes: parse_non_negative_u64_with_default(
                env_map.get("INLINE_DATA_URL_CACHE_MAX_BYTES"),
                DEFAULT_INLINE_DATA_URL_CACHE_MAX_BYTES,
            ),
            inline_data_url_memory_cache_max_bytes: parse_cache_bytes(
                env_map.get("INLINE_DATA_URL_MEMORY_CACHE_MAX_BYTES"),
                DEFAULT_INLINE_DATA_URL_MEMORY_CACHE_MAX_BYTES,
            ),
            inline_data_url_background_fetch_wait_timeout: Duration::from_millis(
                parse_wait_timeout_ms(
                    env_map.get("INLINE_DATA_URL_BACKGROUND_FETCH_WAIT_TIMEOUT_MS"),
                    image_fetch_timeout_ms_raw,
                ),
            ),
            inline_data_url_background_fetch_total_timeout: Duration::from_millis(
                parse_non_negative_u64_with_default(
                    env_map.get("INLINE_DATA_URL_BACKGROUND_FETCH_TOTAL_TIMEOUT_MS"),
                    DEFAULT_INLINE_DATA_URL_BACKGROUND_FETCH_TOTAL_TIMEOUT_MS,
                ),
            ),
            inline_data_url_background_fetch_max_inflight: parse_non_negative_usize_with_default(
                env_map.get("INLINE_DATA_URL_BACKGROUND_FETCH_MAX_INFLIGHT"),
                DEFAULT_INLINE_DATA_URL_BACKGROUND_FETCH_MAX_INFLIGHT,
            ),
            blob_inline_max_bytes: parse_non_negative_u64_with_default(
                env_map.get("BLOB_INLINE_MAX_BYTES"),
                blob_budget_defaults.inline_max_bytes,
            ),
            blob_request_hot_budget_bytes: parse_non_negative_u64_with_default(
                env_map.get("BLOB_REQUEST_HOT_BUDGET_BYTES"),
                blob_budget_defaults.request_hot_budget_bytes,
            ),
            blob_global_hot_budget_bytes: parse_non_negative_u64_with_default(
                env_map.get("BLOB_GLOBAL_HOT_BUDGET_BYTES"),
                blob_budget_defaults.global_hot_budget_bytes,
            ),
            blob_spill_dir: env_map
                .get("BLOB_SPILL_DIR")
                .map(String::as_str)
                .map(parse_optional_string_with_disabled)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| DEFAULT_BLOB_SPILL_DIR.to_string()),
            upload_timeout: Duration::from_millis(parse_positive_u64_with_default(
                env_map.get("UPLOAD_TIMEOUT_MS"),
                DEFAULT_UPLOAD_TIMEOUT_MS,
            )),
            upload_tls_handshake_timeout: Duration::from_millis(parse_positive_u64_with_default(
                env_map.get("UPLOAD_TLS_HANDSHAKE_TIMEOUT_MS"),
                DEFAULT_UPLOAD_TLS_HANDSHAKE_TIMEOUT_MS,
            )),
            upload_insecure_skip_verify: parse_bool(
                env_map.get("UPLOAD_INSECURE_SKIP_VERIFY"),
                false,
            ),
            legacy_uguu_upload_url: DEFAULT_LEGACY_UGUU_UPLOAD_URL.to_string(),
            legacy_kefan_upload_url: DEFAULT_LEGACY_KEFAN_UPLOAD_URL.to_string(),
            r2_endpoint: env_map
                .get("R2_ENDPOINT")
                .map(|v| v.trim().to_string())
                .unwrap_or_default(),
            r2_bucket: env_map
                .get("R2_BUCKET")
                .map(|v| v.trim().to_string())
                .unwrap_or_default(),
            r2_access_key_id: env_map
                .get("R2_ACCESS_KEY_ID")
                .map(|v| v.trim().to_string())
                .unwrap_or_default(),
            r2_secret_access_key: env_map
                .get("R2_SECRET_ACCESS_KEY")
                .map(|v| v.trim().to_string())
                .unwrap_or_default(),
            r2_public_base_url: env_map
                .get("R2_PUBLIC_BASE_URL")
                .map(|v| v.trim().to_string())
                .unwrap_or_default(),
            r2_object_prefix: env_map
                .get("R2_OBJECT_PREFIX")
                .map(|v| v.trim().trim_matches('/').to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "images".to_string()),
        };

        validate(&config)?;
        Ok(config)
    }

    pub fn resolved_external_image_proxy_prefix(&self) -> String {
        let external_prefix = self.external_image_proxy_prefix.trim();
        if !external_prefix.is_empty() {
            return external_prefix.to_string();
        }

        let public_base_url = self.public_base_url.trim().trim_end_matches('/');
        if public_base_url.is_empty() {
            return String::new();
        }

        // 兼容旧版容器配置，避免只保留 PUBLIC_BASE_URL 时忘记同步改环境变量名。
        format!("{public_base_url}/proxy/image?url=")
    }
}

pub fn blob_budget_defaults_for_memory(memory_bytes: u64) -> BlobBudgetDefaults {
    const GIB: u64 = 1024 * 1024 * 1024;

    if memory_bytes >= 8 * GIB {
        BlobBudgetDefaults {
            inline_max_bytes: 16 * 1024 * 1024,
            request_hot_budget_bytes: 64 * 1024 * 1024,
            global_hot_budget_bytes: 1536 * 1024 * 1024,
        }
    } else if memory_bytes >= 4 * GIB {
        BlobBudgetDefaults {
            inline_max_bytes: 12 * 1024 * 1024,
            request_hot_budget_bytes: 40 * 1024 * 1024,
            global_hot_budget_bytes: 768 * 1024 * 1024,
        }
    } else {
        BlobBudgetDefaults {
            inline_max_bytes: DEFAULT_BLOB_INLINE_MAX_BYTES,
            request_hot_budget_bytes: DEFAULT_BLOB_REQUEST_HOT_BUDGET_BYTES,
            global_hot_budget_bytes: DEFAULT_BLOB_GLOBAL_HOT_BUDGET_BYTES,
        }
    }
}

fn validate(config: &Config) -> Result<()> {
    match config.image_host_mode.as_str() {
        "" | "legacy" | "r2" | "r2_then_legacy" => {}
        other => {
            return Err(anyhow!(
                "IMAGE_HOST_MODE must be one of legacy, r2, r2_then_legacy, got {other}"
            ));
        }
    }

    if matches!(config.image_host_mode.as_str(), "r2" | "r2_then_legacy") {
        for (name, value) in [
            ("R2_ENDPOINT", config.r2_endpoint.as_str()),
            ("R2_BUCKET", config.r2_bucket.as_str()),
            ("R2_ACCESS_KEY_ID", config.r2_access_key_id.as_str()),
            ("R2_SECRET_ACCESS_KEY", config.r2_secret_access_key.as_str()),
            ("R2_PUBLIC_BASE_URL", config.r2_public_base_url.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(anyhow!(
                    "{name} is required when IMAGE_HOST_MODE is r2 or r2_then_legacy"
                ));
            }
        }

        parse_http_base_url(&config.r2_endpoint, "R2_ENDPOINT")
            .map_err(|err| anyhow!("R2_ENDPOINT is invalid: {err}"))?;
        parse_http_base_url(&config.r2_public_base_url, "R2_PUBLIC_BASE_URL")?;
    }

    Ok(())
}

fn parse_port(raw: &str) -> Option<u16> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse::<u16>().ok()
}

fn parse_positive_u64_with_default(value: Option<&String>, default_value: u64) -> u64 {
    match value.and_then(|raw| raw.trim().parse::<u64>().ok()) {
        Some(parsed) if parsed > 0 => parsed,
        _ => default_value,
    }
}

fn parse_non_negative_u64_with_default(value: Option<&String>, default_value: u64) -> u64 {
    match value.and_then(|raw| raw.trim().parse::<u64>().ok()) {
        Some(parsed) => parsed,
        None => default_value,
    }
}

fn parse_non_negative_usize_with_default(value: Option<&String>, default_value: usize) -> usize {
    match value.and_then(|raw| raw.trim().parse::<usize>().ok()) {
        Some(parsed) => parsed,
        None => default_value,
    }
}

fn parse_jpeg_quality(value: Option<&String>, default_value: u8) -> u8 {
    match value.and_then(|raw| raw.trim().parse::<u8>().ok()) {
        Some(parsed) if (1..=100).contains(&parsed) => parsed,
        _ => default_value,
    }
}

fn parse_cache_bytes(value: Option<&String>, default_value: u64) -> u64 {
    match value.map(|raw| raw.trim()) {
        Some(raw) if is_disabled_value(raw) => 0,
        Some(raw) => raw.parse::<u64>().unwrap_or(default_value),
        None => default_value,
    }
}

fn parse_wait_timeout_ms(value: Option<&String>, default_value: u64) -> u64 {
    match value.and_then(|raw| raw.trim().parse::<u64>().ok()) {
        Some(parsed) => parsed,
        None => default_value,
    }
}

fn parse_csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn is_disabled_value(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "" | "0" | "false" | "no" | "n" | "off" | "disable" | "disabled" | "none"
    )
}

fn parse_optional_string_with_disabled(raw: &str) -> String {
    if is_disabled_value(raw) {
        String::new()
    } else {
        raw.trim().to_string()
    }
}

fn normalize_optional_http_base_url(raw: &str) -> String {
    if is_disabled_value(raw) {
        return String::new();
    }
    parse_http_base_url(raw, "PUBLIC_BASE_URL").unwrap_or_default()
}

fn parse_http_base_url(raw: &str, env_name: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("{env_name} is empty"));
    }

    let mut parsed = Url::parse(trimmed).map_err(|err| anyhow!("parse {env_name}: {err}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(anyhow!("{env_name} must use http or https"));
    }
    if parsed.host_str().unwrap_or_default().trim().is_empty() {
        return Err(anyhow!("{env_name} host is empty"));
    }
    parsed.set_fragment(None);
    let normalized = parsed.to_string().trim_end_matches('/').to_string();
    Ok(normalized)
}

fn parse_bool(value: Option<&String>, default_value: bool) -> bool {
    match value.map(|v| v.trim().to_ascii_lowercase()) {
        Some(v)
            if matches!(
                v.as_str(),
                "1" | "true" | "yes" | "y" | "on" | "enable" | "enabled"
            ) =>
        {
            true
        }
        Some(v)
            if matches!(
                v.as_str(),
                "0" | "false" | "no" | "n" | "off" | "disable" | "disabled" | "none"
            ) =>
        {
            false
        }
        Some(_) => default_value,
        None => default_value,
    }
}

fn default_allowed_proxy_domains() -> Vec<String> {
    vec![
        "ai.kefan.cn".to_string(),
        "uguu.se".to_string(),
        ".uguu.se".to_string(),
        ".aitohumanize.com".to_string(),
    ]
}
