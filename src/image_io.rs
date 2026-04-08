use std::net::IpAddr;

use anyhow::{Result, anyhow};
use bytes::Bytes;
use reqwest::header::CONTENT_TYPE;
use url::Url;

pub const DEFAULT_MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct FetchedInlineData {
    pub mime_type: String,
    pub bytes: Bytes,
}

pub fn enforce_max_size(actual: usize, limit: usize) -> Result<()> {
    if actual > limit {
        return Err(anyhow!("image too large: {} > {}", actual, limit));
    }
    Ok(())
}

pub async fn fetch_image_as_inline_data(
    client: &reqwest::Client,
    raw_url: &str,
    max_image_bytes: usize,
) -> Result<FetchedInlineData> {
    fetch_image_as_inline_data_with_options(client, raw_url, max_image_bytes, false).await
}

pub async fn fetch_image_as_inline_data_with_options(
    client: &reqwest::Client,
    raw_url: &str,
    max_image_bytes: usize,
    allow_private_networks: bool,
) -> Result<FetchedInlineData> {
    let parsed = Url::parse(raw_url)?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().unwrap_or_default().is_empty()
    {
        return Err(anyhow!("invalid image url"));
    }
    if !allow_private_networks && is_forbidden_fetch_target(&parsed) {
        return Err(anyhow!(
            "forbidden target: {}",
            parsed.host_str().unwrap_or_default()
        ));
    }

    let response = client.get(parsed).send().await?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "image fetch failed with status {}",
            response.status()
        ));
    }

    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let bytes = response.bytes().await?;
    enforce_max_size(bytes.len(), max_image_bytes)?;

    Ok(FetchedInlineData {
        mime_type: normalize_image_mime_type(content_type.as_deref(), raw_url),
        bytes,
    })
}

pub fn hostname_matches_domain_patterns(hostname: &str, patterns: &[String]) -> bool {
    let hostname = hostname.trim().to_ascii_lowercase();
    if hostname.is_empty() {
        return false;
    }

    patterns.iter().any(|pattern| {
        let pattern = pattern.trim().to_ascii_lowercase();
        if pattern.is_empty() {
            return false;
        }

        if let Some(suffix) = pattern.strip_prefix('.') {
            hostname == suffix || hostname.ends_with(&pattern)
        } else {
            hostname == pattern
        }
    })
}

pub fn is_forbidden_fetch_target(url: &Url) -> bool {
    let hostname = url.host_str().unwrap_or_default().trim();
    if hostname.eq_ignore_ascii_case("localhost") {
        return true;
    }

    match hostname.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_unspecified()
        }
        Ok(IpAddr::V6(ip)) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        }
        Err(_) => false,
    }
}

pub fn normalize_image_mime_type(content_type: Option<&str>, raw_url: &str) -> String {
    let normalized = content_type
        .unwrap_or("image/png")
        .split(';')
        .next()
        .unwrap_or("image/png")
        .trim()
        .to_ascii_lowercase();

    if normalized == "application/octet-stream" || normalized.is_empty() {
        return guess_image_mime_type_from_url(raw_url);
    }
    normalized
}

fn guess_image_mime_type_from_url(raw_url: &str) -> String {
    let lower = raw_url.to_ascii_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        return "image/jpeg".to_string();
    }
    if lower.ends_with(".webp") {
        return "image/webp".to_string();
    }
    if lower.ends_with(".gif") {
        return "image/gif".to_string();
    }
    "image/png".to_string()
}
