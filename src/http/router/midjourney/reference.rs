use std::io::Cursor;
use std::sync::Arc;

use axum::http::StatusCode;
use bytes::Bytes;
use image::codecs::webp::WebPEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ExtendedColorType, ImageEncoder, ImageFormat, ImageReader, RgbImage};
use jpeg_encoder::{ColorType, Encoder, SamplingFactor};
use tokio::sync::Semaphore;

use crate::blob_runtime::{BlobHandle, BlobRuntime};
use crate::image_io::{REQUEST_MAX_IMAGE_BYTES, fetch_image_as_inline_data_with_options};
use crate::midjourney::Error;

const MAX_EDGE: u32 = 2048;
const JPEG_QUALITY: u8 = 98;
const MAX_DECODE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_CONCURRENT_PREPARATIONS: usize = 4;
static PREPARATION_SLOTS: Semaphore = Semaphore::const_new(MAX_CONCURRENT_PREPARATIONS);

pub(super) struct RequestBlobs {
    runtime: BlobRuntime,
    handles: Vec<BlobHandle>,
}

impl RequestBlobs {
    pub(super) fn new(runtime: &BlobRuntime) -> Self {
        Self {
            runtime: runtime.clone(),
            handles: Vec::new(),
        }
    }

    pub(super) fn track(&mut self, blob: BlobHandle) -> BlobHandle {
        self.handles.push(blob.clone());
        blob
    }

    pub(super) async fn cleanup(&mut self) {
        if !self.handles.is_empty() {
            let runtime = self.runtime.clone();
            let handles = std::mem::take(&mut self.handles);
            let _ = tokio::spawn(cleanup(runtime, handles)).await;
        }
    }

    pub(super) async fn release(&mut self, blob: &BlobHandle) -> anyhow::Result<()> {
        let index = self
            .handles
            .iter()
            .position(|handle| handle.id() == blob.id())
            .expect("request blob must be tracked");
        let handle = self.handles.swap_remove(index);
        let runtime = self.runtime.clone();
        tokio::spawn(async move { runtime.remove(&handle).await }).await??;
        Ok(())
    }
}

impl Drop for RequestBlobs {
    fn drop(&mut self) {
        if self.handles.is_empty() {
            return;
        }
        let runtime = self.runtime.clone();
        let handles = std::mem::take(&mut self.handles);
        tokio::spawn(cleanup(runtime, handles));
    }
}

async fn cleanup(runtime: BlobRuntime, handles: Vec<BlobHandle>) {
    for handle in handles {
        if let Err(error) = runtime.remove(&handle).await {
            tracing::warn!(%error, "Magic request blob cleanup failed");
        }
    }
}

pub(super) async fn prepare(
    client: &reqwest::Client,
    runtime: &BlobRuntime,
    resources: &mut RequestBlobs,
    url: &str,
) -> Result<BlobHandle, Error> {
    let permit = Arc::new(
        PREPARATION_SLOTS
            .acquire()
            .await
            .map_err(|error| storage_error(error.into()))?,
    );
    // shortcut: 通用缓存尚无原始载荷标记，MJ 先绕过；支持原始变体后再复用。
    let fetched =
        fetch_image_as_inline_data_with_options(client, url, REQUEST_MAX_IMAGE_BYTES, false)
            .await
            .map_err(|error| {
                let mut result = Error::new(
                    StatusCode::BAD_GATEWAY,
                    "proxy",
                    "magic_prepare_image",
                    "image_fetch_failed",
                    "参考图片下载失败，请确认 HTTP(S) 地址可访问且图片不超过 20MiB",
                );
                result.detail = Some(match error.downcast::<reqwest::Error>() {
                    Ok(error) => super::reqwest_error_detail(&error.without_url()),
                    Err(error) => error.to_string(),
                });
                result
            })?;
    let original = resources.track(
        runtime
            .store_shared_bytes(fetched.bytes, fetched.mime_type)
            .await
            .map_err(storage_error)?,
    );
    let bytes = runtime.read_bytes(&original).await.map_err(storage_error)?;
    let blocking_permit = permit.clone();
    let (mime_type, bytes) = tokio::task::spawn_blocking(move || {
        // 已开始的编解码不能被 abort；取消请求后仍占槽直到阻塞任务退出。
        let _permit = blocking_permit;
        transform(bytes)
    })
    .await
    .map_err(|error| storage_error(error.into()))?
    .map_err(|error| {
        let mut result = Error::new(
            StatusCode::BAD_REQUEST,
            "proxy",
            "magic_prepare_image",
            "invalid_reference_image",
            "参考图片无法解码或不受支持，请使用有效的 JPEG、PNG 或 WebP 图片",
        );
        result.detail = Some(error.to_string());
        result
    })?;
    let blob = resources.track(
        runtime
            .store_shared_bytes(bytes, mime_type.to_string())
            .await
            .map_err(storage_error)?,
    );
    resources.release(&original).await.map_err(storage_error)?;
    Ok(blob)
}

fn storage_error(error: anyhow::Error) -> Error {
    let mut result = Error::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "proxy",
        "magic_prepare_image",
        "image_storage_failed",
        "参考图片处理资源不可用，请稍后重试",
    );
    result.detail = Some(error.to_string());
    result
}

fn transform(bytes: Bytes) -> anyhow::Result<(&'static str, Bytes)> {
    let format = image::guess_format(&bytes)?;
    let mime = match format {
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Png => "image/jpeg",
        ImageFormat::WebP => "image/webp",
        _ => anyhow::bail!("unsupported reference image format"),
    };
    let mut reader = ImageReader::with_format(Cursor::new(&bytes), format);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let mut image = reader.decode()?;
    let longest = image.width().max(image.height());
    if longest <= MAX_EDGE && format != ImageFormat::Png {
        return Ok((mime, bytes));
    }
    drop(bytes);
    if longest > MAX_EDGE {
        let scaled = |edge| {
            ((u64::from(edge) * u64::from(MAX_EDGE) + u64::from(longest) / 2) / u64::from(longest))
                .max(1) as u32
        };
        let width = scaled(image.width());
        let height = scaled(image.height());
        image = image.resize_exact(width, height, FilterType::Lanczos3);
    }
    let mut encoded = Vec::new();
    if format == ImageFormat::WebP {
        let rgba = image.into_rgba8();
        WebPEncoder::new_lossless(&mut encoded).write_image(
            rgba.as_raw(),
            rgba.width(),
            rgba.height(),
            ExtendedColorType::Rgba8,
        )?;
    } else {
        let rgb = if format == ImageFormat::Png {
            composite_white(image)
        } else {
            image.into_rgb8()
        };
        let mut encoder = Encoder::new(&mut encoded, JPEG_QUALITY);
        encoder.set_sampling_factor(SamplingFactor::R_4_4_4);
        encoder.encode(
            rgb.as_raw(),
            u16::try_from(rgb.width())?,
            u16::try_from(rgb.height())?,
            ColorType::Rgb,
        )?;
    }
    Ok((mime, Bytes::from(encoded)))
}

fn composite_white(image: DynamicImage) -> RgbImage {
    let rgba = image.into_rgba8();
    RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let pixel = rgba.get_pixel(x, y);
        let alpha = u32::from(pixel[3]);
        image::Rgb([0, 1, 2].map(|channel| {
            ((u32::from(pixel[channel]) * alpha + 255 * (255 - alpha) + 127) / 255) as u8
        }))
    })
}
