use std::net::IpAddr;

use anyhow::{Result, anyhow};
use bytes::{Bytes, BytesMut};
use image::ExtendedColorType;
use image::ImageEncoder;
use image::ImageFormat;
use image::codecs::webp::WebPEncoder;
use jpeg_encoder::{ColorType as JpegColorType, Encoder as JpegEncoder, SamplingFactor};
use reqwest::header::CONTENT_TYPE;
use url::Url;

use crate::blob_runtime::{BlobHandle, BlobRuntime};
use crate::cache::ImageFetchStatusError;

pub const DEFAULT_MAX_IMAGE_BYTES: usize = 35 * 1024 * 1024;
pub const REQUEST_MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
pub const REQUEST_PNG_WEBP_THRESHOLD_BYTES: usize = 10 * 1024 * 1024;
pub const PNG_COMPRESSION_THRESHOLD_BYTES: usize = 15 * 1024 * 1024;
pub const DEFAULT_JPEG_QUALITY: u8 = 97;

#[derive(Clone, Debug)]
pub struct FetchedInlineData {
    pub mime_type: String,
    pub bytes: Bytes,
}

#[derive(Clone, Debug)]
pub struct OptimizedImage {
    pub mime_type: String,
    pub bytes: Bytes,
}

#[derive(Clone, Debug)]
pub struct FetchedBlob {
    pub mime_type: String,
    pub blob: BlobHandle,
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
        return Err(ImageFetchStatusError {
            status: response.status(),
        }
        .into());
    }
    if let Some(content_length) = response.content_length() {
        enforce_max_size_u64(content_length, max_image_bytes)?;
    }

    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let mut total_bytes = 0_usize;
    let mut bytes = BytesMut::new();
    let mut response = response;
    while let Some(chunk) = response.chunk().await? {
        total_bytes += chunk.len();
        enforce_max_size(total_bytes, max_image_bytes)?;
        bytes.extend_from_slice(&chunk);
    }

    Ok(FetchedInlineData {
        mime_type: normalize_image_mime_type(content_type.as_deref(), raw_url),
        bytes: bytes.freeze(),
    })
}

pub async fn fetch_image_into_blob(
    client: &reqwest::Client,
    runtime: &BlobRuntime,
    raw_url: &str,
    max_image_bytes: usize,
    allow_private_networks: bool,
) -> Result<FetchedBlob> {
    let fetched = fetch_image_as_inline_data_with_options(
        client,
        raw_url,
        max_image_bytes,
        allow_private_networks,
    )
    .await?;
    let fetched = maybe_convert_large_png_to_lossless_webp(fetched).await?;
    let blob = runtime
        .store_bytes(fetched.bytes.to_vec(), fetched.mime_type.clone())
        .await?;

    Ok(FetchedBlob {
        mime_type: fetched.mime_type,
        blob,
    })
}

pub async fn maybe_convert_large_png_to_lossless_webp(
    fetched: FetchedInlineData,
) -> Result<FetchedInlineData> {
    let fallback = fetched.clone();
    tokio::task::spawn_blocking(move || maybe_convert_large_png_to_lossless_webp_sync(fetched))
        .await
        .map_err(|err| anyhow!("request image optimization task failed: {err}"))
        .map(|optimized| optimized.unwrap_or(fallback))
}

fn maybe_convert_large_png_to_lossless_webp_sync(
    fetched: FetchedInlineData,
) -> Result<FetchedInlineData> {
    let optimized = maybe_convert_png_bytes_to_lossless_webp_with_threshold(
        &fetched.bytes,
        &fetched.mime_type,
        REQUEST_PNG_WEBP_THRESHOLD_BYTES,
    )?;

    Ok(FetchedInlineData {
        mime_type: optimized.mime_type,
        bytes: optimized.bytes,
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

pub fn maybe_compress_png_bytes(
    bytes: &[u8],
    mime_type: &str,
    enabled: bool,
) -> Result<OptimizedImage> {
    maybe_compress_png_bytes_with_options(
        bytes,
        mime_type,
        enabled,
        PNG_COMPRESSION_THRESHOLD_BYTES,
        DEFAULT_JPEG_QUALITY,
    )
}

pub fn maybe_compress_png_bytes_with_threshold(
    bytes: &[u8],
    mime_type: &str,
    enabled: bool,
    threshold_bytes: usize,
) -> Result<OptimizedImage> {
    maybe_compress_png_bytes_with_options(
        bytes,
        mime_type,
        enabled,
        threshold_bytes,
        DEFAULT_JPEG_QUALITY,
    )
}

pub fn maybe_compress_png_bytes_with_options(
    bytes: &[u8],
    mime_type: &str,
    enabled: bool,
    threshold_bytes: usize,
    jpeg_quality: u8,
) -> Result<OptimizedImage> {
    let normalized_mime = mime_type.trim().to_ascii_lowercase();
    if !enabled || normalized_mime != "image/png" || bytes.len() <= threshold_bytes {
        return Ok(OptimizedImage {
            mime_type: normalized_mime,
            bytes: Bytes::copy_from_slice(bytes),
        });
    }

    let dynamic = image::load_from_memory_with_format(bytes, ImageFormat::Png)?;
    let rgb = dynamic.to_rgb8();
    let width = u16::try_from(rgb.width()).map_err(|_| anyhow!("image width too large"))?;
    let height = u16::try_from(rgb.height()).map_err(|_| anyhow!("image height too large"))?;

    let mut encoded = Vec::new();
    let mut encoder = JpegEncoder::new(&mut encoded, jpeg_quality);
    encoder.set_sampling_factor(SamplingFactor::R_4_4_4);
    encoder.encode(rgb.as_raw(), width, height, JpegColorType::Rgb)?;

    if encoded.len() >= bytes.len() {
        return Ok(OptimizedImage {
            mime_type: normalized_mime,
            bytes: Bytes::copy_from_slice(bytes),
        });
    }

    Ok(OptimizedImage {
        mime_type: "image/jpeg".to_string(),
        bytes: Bytes::from(encoded),
    })
}

pub fn maybe_convert_png_bytes_to_lossless_webp_with_threshold(
    bytes: &[u8],
    mime_type: &str,
    threshold_bytes: usize,
) -> Result<OptimizedImage> {
    let normalized_mime = mime_type.trim().to_ascii_lowercase();
    if normalized_mime != "image/png" || bytes.len() <= threshold_bytes {
        return Ok(OptimizedImage {
            mime_type: normalized_mime,
            bytes: Bytes::copy_from_slice(bytes),
        });
    }

    let dynamic = image::load_from_memory_with_format(bytes, ImageFormat::Png)?;
    let rgba = dynamic.to_rgba8();
    let mut encoded = Vec::new();
    WebPEncoder::new_lossless(&mut encoded).write_image(
        rgba.as_raw(),
        rgba.width(),
        rgba.height(),
        ExtendedColorType::Rgba8,
    )?;

    if encoded.len() >= bytes.len() {
        return Ok(OptimizedImage {
            mime_type: normalized_mime,
            bytes: Bytes::copy_from_slice(bytes),
        });
    }

    Ok(OptimizedImage {
        mime_type: "image/webp".to_string(),
        bytes: Bytes::from(encoded),
    })
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

fn enforce_max_size_u64(actual: u64, limit: usize) -> Result<()> {
    if actual > limit as u64 {
        return Err(anyhow!("image too large: {} > {}", actual, limit));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{
        DEFAULT_JPEG_QUALITY, maybe_compress_png_bytes_with_options,
        maybe_compress_png_bytes_with_threshold,
    };

    #[test]
    fn large_png_can_be_reencoded_to_jpeg_when_compression_enabled() {
        let image = image::RgbImage::from_fn(128, 128, |x, y| {
            image::Rgb([
                ((x * 31 + y * 17) % 255) as u8,
                ((x * 13 + y * 29) % 255) as u8,
                ((x * 7 + y * 47) % 255) as u8,
            ])
        });
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();

        let optimized =
            maybe_compress_png_bytes_with_threshold(&png, "image/png", true, 1).unwrap();

        assert_eq!(optimized.mime_type, "image/jpeg");
        assert!(optimized.bytes.starts_with(&[0xFF, 0xD8, 0xFF]));
    }

    #[test]
    fn higher_jpeg_quality_produces_larger_reencoded_output() {
        let image = image::RgbImage::from_fn(128, 128, |x, y| {
            image::Rgb([
                ((x * 31 + y * 17) % 255) as u8,
                ((x * 13 + y * 29) % 255) as u8,
                ((x * 7 + y * 47) % 255) as u8,
            ])
        });
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();

        let lower_quality =
            maybe_compress_png_bytes_with_options(&png, "image/png", true, 1, 90).unwrap();
        let default_quality =
            maybe_compress_png_bytes_with_options(&png, "image/png", true, 1, DEFAULT_JPEG_QUALITY)
                .unwrap();

        assert_eq!(lower_quality.mime_type, "image/jpeg");
        assert_eq!(default_quality.mime_type, "image/jpeg");
        assert!(default_quality.bytes.len() > lower_quality.bytes.len());
    }
}
