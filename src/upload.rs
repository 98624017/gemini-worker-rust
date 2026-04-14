use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use hmac::{Hmac, Mac};
use rand::Rng;
use reqwest::Body;
use reqwest::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, USER_AGENT};
use reqwest::multipart::{Form, Part};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::FormatItem;
use time::macros::format_description;
use tokio::io::AsyncReadExt;
use tokio_util::io::ReaderStream;
use url::Url;

use crate::blob_runtime::{BlobHandle, BlobRuntime, BlobStorage};
use crate::config::Config;

type HmacSha256 = Hmac<Sha256>;

pub type BoxUploadFuture = Pin<Box<dyn Future<Output = Result<UploadResult>> + Send>>;

const UPLOAD_USER_AGENT: &str = "ComfyUI-Banana/1.0";
const BASE64_STREAM_INPUT_CHARS: usize = 32 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageHostMode {
    Legacy,
    R2,
    R2ThenLegacy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UploadResult {
    pub url: String,
    pub provider: String,
}

#[derive(Clone)]
pub struct Uploader {
    client: reqwest::Client,
    config: Config,
}

impl Uploader {
    pub fn new(client: reqwest::Client, config: Config) -> Self {
        Self { client, config }
    }

    pub async fn upload_image(&self, data: &[u8], mime_type: &str) -> Result<UploadResult> {
        match parse_image_host_mode(&self.config.image_host_mode)? {
            ImageHostMode::Legacy => self.upload_legacy(data, mime_type).await,
            ImageHostMode::R2 => self.upload_r2(data, mime_type).await,
            ImageHostMode::R2ThenLegacy => match self.upload_r2(data, mime_type).await {
                Ok(result) => Ok(result),
                Err(_) => self.upload_legacy(data, mime_type).await,
            },
        }
    }

    pub async fn upload_blob(
        &self,
        runtime: &BlobRuntime,
        blob: &BlobHandle,
        mime_type: &str,
    ) -> Result<UploadResult> {
        match parse_image_host_mode(&self.config.image_host_mode)? {
            ImageHostMode::Legacy => self.upload_legacy_blob(runtime, blob, mime_type).await,
            ImageHostMode::R2 => self.upload_r2_blob(runtime, blob, mime_type).await,
            ImageHostMode::R2ThenLegacy => {
                match self.upload_r2_blob(runtime, blob, mime_type).await {
                    Ok(result) => Ok(result),
                    Err(_) => self.upload_legacy_blob(runtime, blob, mime_type).await,
                }
            }
        }
    }

    pub async fn upload_inline_data_base64(
        &self,
        data_base64: Arc<str>,
        mime_type: &str,
    ) -> Result<UploadResult> {
        match parse_image_host_mode(&self.config.image_host_mode)? {
            ImageHostMode::Legacy => self.upload_legacy_base64(data_base64, mime_type).await,
            ImageHostMode::R2 => self.upload_r2_base64(data_base64, mime_type).await,
            ImageHostMode::R2ThenLegacy => {
                match self
                    .upload_r2_base64(Arc::clone(&data_base64), mime_type)
                    .await
                {
                    Ok(result) => Ok(result),
                    Err(_) => self.upload_legacy_base64(data_base64, mime_type).await,
                }
            }
        }
    }

    async fn upload_legacy(&self, data: &[u8], mime_type: &str) -> Result<UploadResult> {
        if let Ok(url) = self
            .upload_to_uguu(&self.config.legacy_uguu_upload_url, data, mime_type)
            .await
        {
            return Ok(UploadResult {
                url,
                provider: "legacy".to_string(),
            });
        }

        let url = self
            .upload_to_kefan(&self.config.legacy_kefan_upload_url, data, mime_type)
            .await?;
        Ok(UploadResult {
            url,
            provider: "legacy".to_string(),
        })
    }

    async fn upload_legacy_base64(
        &self,
        data_base64: Arc<str>,
        mime_type: &str,
    ) -> Result<UploadResult> {
        if let Ok(url) = self
            .upload_base64_to_uguu(Arc::clone(&data_base64), mime_type)
            .await
        {
            return Ok(UploadResult {
                url,
                provider: "legacy".to_string(),
            });
        }

        let url = self.upload_base64_to_kefan(data_base64, mime_type).await?;
        Ok(UploadResult {
            url,
            provider: "legacy".to_string(),
        })
    }

    async fn upload_to_uguu(
        &self,
        target_url: &str,
        data: &[u8],
        mime_type: &str,
    ) -> Result<String> {
        #[derive(Deserialize)]
        struct UguuResponse {
            success: bool,
            files: Vec<UguuFile>,
        }
        #[derive(Deserialize)]
        struct UguuFile {
            url: String,
        }

        let part = Part::bytes(data.to_vec())
            .file_name(format!("image{}", extension_from_mime(mime_type)))
            .mime_str(mime_type)?;
        let form = Form::new().part("files[]", part);
        let response = self
            .client
            .post(target_url)
            .header(reqwest::header::USER_AGENT, UPLOAD_USER_AGENT)
            .header(reqwest::header::ACCEPT, "application/json")
            .multipart(form)
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(anyhow!("uguu status {}: {}", status, body.trim()));
        }
        let parsed: UguuResponse = serde_json::from_str(&body)
            .map_err(|err| anyhow!("uguu invalid json: {err}: {}", body.trim()))?;
        if !parsed.success || parsed.files.is_empty() || parsed.files[0].url.trim().is_empty() {
            return Err(anyhow!("uguu upload failed: {}", body.trim()));
        }
        Ok(parsed.files[0].url.clone())
    }

    async fn upload_to_kefan(
        &self,
        target_url: &str,
        data: &[u8],
        mime_type: &str,
    ) -> Result<String> {
        #[derive(Deserialize)]
        struct KefanResponse {
            success: bool,
            data: String,
        }

        let part = Part::bytes(data.to_vec())
            .file_name(format!("image{}", extension_from_mime(mime_type)))
            .mime_str(mime_type)?;
        let form = Form::new().part("file", part);
        let response = self
            .client
            .post(target_url)
            .header(reqwest::header::USER_AGENT, UPLOAD_USER_AGENT)
            .multipart(form)
            .send()
            .await?;
        let body = response.text().await?;
        let parsed: KefanResponse = serde_json::from_str(&body)
            .map_err(|err| anyhow!("kefan invalid json: {err}: {}", body.trim()))?;
        if !parsed.success || parsed.data.trim().is_empty() {
            return Err(anyhow!("kefan upload failed: {}", body.trim()));
        }
        Ok(parsed.data)
    }

    async fn upload_r2(&self, data: &[u8], mime_type: &str) -> Result<UploadResult> {
        let endpoint = Url::parse(&self.config.r2_endpoint)?;
        let key = build_r2_object_key(
            &self.config.r2_object_prefix,
            mime_type,
            OffsetDateTime::now_utc(),
            random_hex(4),
        );
        let body = data.to_vec();
        let (object_url, canonical_uri) =
            build_r2_object_url(&endpoint, &self.config.r2_bucket, &key)?;

        let amz_date_format: &[FormatItem<'static>] =
            format_description!("[year][month][day]T[hour][minute][second]Z");
        let date_stamp_format: &[FormatItem<'static>] = format_description!("[year][month][day]");
        let now = OffsetDateTime::now_utc();
        let amz_date = now.format(amz_date_format)?;
        let date_stamp = now.format(date_stamp_format)?;
        let payload_hash = sha256_hex(&body);

        let canonical_headers = format!(
            "content-type:{mime_type}\nhost:{}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{amz_date}\n",
            endpoint.host_str().unwrap_or_default()
        );
        let signed_headers = "content-type;host;x-amz-content-sha256;x-amz-date";
        let canonical_request = format!(
            "PUT\n{canonical_uri}\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
        );
        let credential_scope = format!("{date_stamp}/auto/s3/aws4_request");
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{}",
            sha256_hex(canonical_request.as_bytes())
        );
        let signing_key =
            derive_aws_v4_signing_key(&self.config.r2_secret_access_key, &date_stamp, "auto", "s3");
        let signature = hex::encode(hmac_sha256(&signing_key, &string_to_sign)?);
        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            self.config.r2_access_key_id, credential_scope, signed_headers, signature
        );

        let response = self
            .client
            .put(object_url)
            .header(reqwest::header::CONTENT_TYPE, mime_type)
            .header("X-Amz-Content-Sha256", payload_hash)
            .header("X-Amz-Date", amz_date)
            .header(reqwest::header::AUTHORIZATION, authorization)
            .body(body)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "r2 put object failed: status {}: {}",
                status,
                body.trim()
            ));
        }

        Ok(UploadResult {
            url: format!(
                "{}/{}",
                self.config.r2_public_base_url.trim_end_matches('/'),
                key
            ),
            provider: "r2".to_string(),
        })
    }

    async fn upload_legacy_blob(
        &self,
        runtime: &BlobRuntime,
        blob: &BlobHandle,
        mime_type: &str,
    ) -> Result<UploadResult> {
        if let Ok(url) = self.upload_blob_to_uguu(runtime, blob, mime_type).await {
            return Ok(UploadResult {
                url,
                provider: "legacy".to_string(),
            });
        }

        let url = self.upload_blob_to_kefan(runtime, blob, mime_type).await?;
        Ok(UploadResult {
            url,
            provider: "legacy".to_string(),
        })
    }

    async fn upload_blob_to_uguu(
        &self,
        runtime: &BlobRuntime,
        blob: &BlobHandle,
        mime_type: &str,
    ) -> Result<String> {
        #[derive(Deserialize)]
        struct UguuResponse {
            success: bool,
            files: Vec<UguuFile>,
        }
        #[derive(Deserialize)]
        struct UguuFile {
            url: String,
        }

        let response = self
            .send_streaming_multipart(
                runtime,
                blob,
                &self.config.legacy_uguu_upload_url,
                "files[]",
                mime_type,
                true,
            )
            .await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(anyhow!("uguu status {}: {}", status, body.trim()));
        }
        let parsed: UguuResponse = serde_json::from_str(&body)
            .map_err(|err| anyhow!("uguu invalid json: {err}: {}", body.trim()))?;
        if !parsed.success || parsed.files.is_empty() || parsed.files[0].url.trim().is_empty() {
            return Err(anyhow!("uguu upload failed: {}", body.trim()));
        }
        Ok(parsed.files[0].url.clone())
    }

    async fn upload_base64_to_uguu(
        &self,
        data_base64: Arc<str>,
        mime_type: &str,
    ) -> Result<String> {
        #[derive(Deserialize)]
        struct UguuResponse {
            success: bool,
            files: Vec<UguuFile>,
        }
        #[derive(Deserialize)]
        struct UguuFile {
            url: String,
        }

        let decoded_len = decoded_base64_len(data_base64.as_ref())?;
        let response = self
            .send_streaming_multipart_reader(
                Base64DecodedReader::new(data_base64),
                decoded_len,
                &self.config.legacy_uguu_upload_url,
                "files[]",
                mime_type,
                true,
            )
            .await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(anyhow!("uguu status {}: {}", status, body.trim()));
        }
        let parsed: UguuResponse = serde_json::from_str(&body)
            .map_err(|err| anyhow!("uguu invalid json: {err}: {}", body.trim()))?;
        if !parsed.success || parsed.files.is_empty() || parsed.files[0].url.trim().is_empty() {
            return Err(anyhow!("uguu upload failed: {}", body.trim()));
        }
        Ok(parsed.files[0].url.clone())
    }

    async fn upload_blob_to_kefan(
        &self,
        runtime: &BlobRuntime,
        blob: &BlobHandle,
        mime_type: &str,
    ) -> Result<String> {
        #[derive(Deserialize)]
        struct KefanResponse {
            success: bool,
            data: String,
        }

        let response = self
            .send_streaming_multipart(
                runtime,
                blob,
                &self.config.legacy_kefan_upload_url,
                "file",
                mime_type,
                false,
            )
            .await?;
        let body = response.text().await?;
        let parsed: KefanResponse = serde_json::from_str(&body)
            .map_err(|err| anyhow!("kefan invalid json: {err}: {}", body.trim()))?;
        if !parsed.success || parsed.data.trim().is_empty() {
            return Err(anyhow!("kefan upload failed: {}", body.trim()));
        }
        Ok(parsed.data)
    }

    async fn upload_base64_to_kefan(
        &self,
        data_base64: Arc<str>,
        mime_type: &str,
    ) -> Result<String> {
        #[derive(Deserialize)]
        struct KefanResponse {
            success: bool,
            data: String,
        }

        let decoded_len = decoded_base64_len(data_base64.as_ref())?;
        let response = self
            .send_streaming_multipart_reader(
                Base64DecodedReader::new(data_base64),
                decoded_len,
                &self.config.legacy_kefan_upload_url,
                "file",
                mime_type,
                false,
            )
            .await?;
        let body = response.text().await?;
        let parsed: KefanResponse = serde_json::from_str(&body)
            .map_err(|err| anyhow!("kefan invalid json: {err}: {}", body.trim()))?;
        if !parsed.success || parsed.data.trim().is_empty() {
            return Err(anyhow!("kefan upload failed: {}", body.trim()));
        }
        Ok(parsed.data)
    }

    async fn send_streaming_multipart(
        &self,
        runtime: &BlobRuntime,
        blob: &BlobHandle,
        target_url: &str,
        field_name: &str,
        mime_type: &str,
        accept_json: bool,
    ) -> Result<reqwest::Response> {
        let reader = runtime.open_reader(blob).await?;
        self.send_streaming_multipart_reader(
            reader,
            blob.meta().size_bytes,
            target_url,
            field_name,
            mime_type,
            accept_json,
        )
        .await
    }

    async fn send_streaming_multipart_reader<R>(
        &self,
        reader: R,
        content_length: u64,
        target_url: &str,
        field_name: &str,
        mime_type: &str,
        accept_json: bool,
    ) -> Result<reqwest::Response>
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
    {
        let body = Body::wrap_stream(ReaderStream::new(reader));
        let part = Part::stream_with_length(body, content_length)
            .file_name(format!("image{}", extension_from_mime(mime_type)))
            .mime_str(mime_type)?;
        let form = Form::new().part(field_name.to_string(), part);

        let mut request = self
            .client
            .post(target_url)
            .header(USER_AGENT, UPLOAD_USER_AGENT)
            .multipart(form);
        if accept_json {
            request = request.header(reqwest::header::ACCEPT, "application/json");
        }
        Ok(request.send().await?)
    }

    async fn upload_r2_blob(
        &self,
        runtime: &BlobRuntime,
        blob: &BlobHandle,
        mime_type: &str,
    ) -> Result<UploadResult> {
        let endpoint = Url::parse(&self.config.r2_endpoint)?;
        let key = build_r2_object_key(
            &self.config.r2_object_prefix,
            mime_type,
            OffsetDateTime::now_utc(),
            random_hex(4),
        );
        let (object_url, canonical_uri) =
            build_r2_object_url(&endpoint, &self.config.r2_bucket, &key)?;

        let amz_date_format: &[FormatItem<'static>] =
            format_description!("[year][month][day]T[hour][minute][second]Z");
        let date_stamp_format: &[FormatItem<'static>] = format_description!("[year][month][day]");
        let now = OffsetDateTime::now_utc();
        let amz_date = now.format(amz_date_format)?;
        let date_stamp = now.format(date_stamp_format)?;
        let payload_hash = sha256_hex_blob(blob, runtime).await?;

        let canonical_headers = format!(
            "content-type:{mime_type}\nhost:{}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{amz_date}\n",
            endpoint.host_str().unwrap_or_default()
        );
        let signed_headers = "content-type;host;x-amz-content-sha256;x-amz-date";
        let canonical_request = format!(
            "PUT\n{canonical_uri}\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
        );
        let credential_scope = format!("{date_stamp}/auto/s3/aws4_request");
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{}",
            sha256_hex(canonical_request.as_bytes())
        );
        let signing_key =
            derive_aws_v4_signing_key(&self.config.r2_secret_access_key, &date_stamp, "auto", "s3");
        let signature = hex::encode(hmac_sha256(&signing_key, &string_to_sign)?);
        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            self.config.r2_access_key_id, credential_scope, signed_headers, signature
        );

        let reader = runtime.open_reader(blob).await?;
        let response = put_object_stream(
            &self.client,
            object_url,
            authorization,
            amz_date,
            payload_hash,
            mime_type,
            blob.meta().size_bytes,
            reader,
        )
        .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "r2 put object failed: status {}: {}",
                status,
                body.trim()
            ));
        }

        Ok(UploadResult {
            url: format!(
                "{}/{}",
                self.config.r2_public_base_url.trim_end_matches('/'),
                key
            ),
            provider: "r2".to_string(),
        })
    }

    async fn upload_r2_base64(
        &self,
        data_base64: Arc<str>,
        mime_type: &str,
    ) -> Result<UploadResult> {
        let endpoint = Url::parse(&self.config.r2_endpoint)?;
        let key = build_r2_object_key(
            &self.config.r2_object_prefix,
            mime_type,
            OffsetDateTime::now_utc(),
            random_hex(4),
        );
        let (object_url, canonical_uri) =
            build_r2_object_url(&endpoint, &self.config.r2_bucket, &key)?;

        let amz_date_format: &[FormatItem<'static>] =
            format_description!("[year][month][day]T[hour][minute][second]Z");
        let date_stamp_format: &[FormatItem<'static>] = format_description!("[year][month][day]");
        let now = OffsetDateTime::now_utc();
        let amz_date = now.format(amz_date_format)?;
        let date_stamp = now.format(date_stamp_format)?;
        let payload_hash = sha256_hex_base64(data_base64.as_ref())?;
        let content_length = decoded_base64_len(data_base64.as_ref())?;

        let canonical_headers = format!(
            "content-type:{mime_type}\nhost:{}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{amz_date}\n",
            endpoint.host_str().unwrap_or_default()
        );
        let signed_headers = "content-type;host;x-amz-content-sha256;x-amz-date";
        let canonical_request = format!(
            "PUT\n{canonical_uri}\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
        );
        let credential_scope = format!("{date_stamp}/auto/s3/aws4_request");
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{}",
            sha256_hex(canonical_request.as_bytes())
        );
        let signing_key =
            derive_aws_v4_signing_key(&self.config.r2_secret_access_key, &date_stamp, "auto", "s3");
        let signature = hex::encode(hmac_sha256(&signing_key, &string_to_sign)?);
        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            self.config.r2_access_key_id, credential_scope, signed_headers, signature
        );

        let response = put_object_stream(
            &self.client,
            object_url,
            authorization,
            amz_date,
            payload_hash,
            mime_type,
            content_length,
            Base64DecodedReader::new(data_base64),
        )
        .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "r2 put object failed: status {}: {}",
                status,
                body.trim()
            ));
        }

        Ok(UploadResult {
            url: format!(
                "{}/{}",
                self.config.r2_public_base_url.trim_end_matches('/'),
                key
            ),
            provider: "r2".to_string(),
        })
    }
}

pub async fn upload_image_with_mode<R2, Legacy>(
    mode: ImageHostMode,
    data: &[u8],
    mime_type: &str,
    r2_uploader: &R2,
    legacy_uploader: &Legacy,
) -> Result<UploadResult>
where
    R2: Fn(Vec<u8>, String) -> BoxUploadFuture + Sync,
    Legacy: Fn(Vec<u8>, String) -> BoxUploadFuture + Sync,
{
    match mode {
        ImageHostMode::Legacy => legacy_uploader(data.to_vec(), mime_type.to_string()).await,
        ImageHostMode::R2 => r2_uploader(data.to_vec(), mime_type.to_string()).await,
        ImageHostMode::R2ThenLegacy => {
            let owned_data = data.to_vec();
            let owned_mime = mime_type.to_string();
            match r2_uploader(owned_data.clone(), owned_mime.clone()).await {
                Ok(result) => Ok(result),
                Err(_) => legacy_uploader(owned_data, owned_mime).await,
            }
        }
    }
}

pub fn parse_image_host_mode(raw: &str) -> Result<ImageHostMode> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "legacy" => Ok(ImageHostMode::Legacy),
        "r2" => Ok(ImageHostMode::R2),
        "r2_then_legacy" => Ok(ImageHostMode::R2ThenLegacy),
        other => Err(anyhow!("unsupported IMAGE_HOST_MODE {}", other)),
    }
}

pub fn wrap_external_proxy_url(external_proxy_prefix: &str, target_url: &str) -> String {
    let encoded: String = url::form_urlencoded::byte_serialize(target_url.as_bytes()).collect();
    format!("{}{}", external_proxy_prefix.trim(), encoded)
}

pub fn build_r2_object_key(
    prefix: &str,
    mime_type: &str,
    now: OffsetDateTime,
    random_hex: String,
) -> String {
    let prefix = prefix.trim().trim_matches('/');
    let prefix = if prefix.is_empty() { "images" } else { prefix };
    format!(
        "{}/{:04}/{:02}/{:02}/{}-{}.{}",
        prefix,
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.unix_timestamp_nanos() / 1_000_000,
        random_hex,
        extension_from_mime(mime_type).trim_start_matches('.')
    )
}

fn extension_from_mime(mime_type: &str) -> &'static str {
    match mime_type {
        "image/jpeg" => ".jpg",
        "image/webp" => ".webp",
        "image/gif" => ".gif",
        _ => ".png",
    }
}

fn build_r2_object_url(endpoint: &Url, bucket: &str, key: &str) -> Result<(String, String)> {
    let bucket = bucket.trim();
    let key = key.trim().trim_start_matches('/');
    let plain_path = format!("/{bucket}/{key}");
    let canonical_uri = format!(
        "/{}/{}",
        url::form_urlencoded::byte_serialize(bucket.as_bytes()).collect::<String>(),
        key.split('/')
            .map(|part| url::form_urlencoded::byte_serialize(part.as_bytes()).collect::<String>())
            .collect::<Vec<_>>()
            .join("/")
    );
    let mut object_url = endpoint.clone();
    object_url.set_path(&(endpoint.path().trim_end_matches('/').to_string() + &plain_path));
    Ok((object_url.to_string(), canonical_uri))
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

fn decoded_base64_len(data_base64: &str) -> Result<u64> {
    if data_base64.is_empty() {
        return Ok(0);
    }
    if data_base64.len() % 4 != 0 {
        return Err(anyhow!("invalid base64 length"));
    }

    let padding = if data_base64.ends_with("==") {
        2
    } else if data_base64.ends_with('=') {
        1
    } else {
        0
    };
    Ok(((data_base64.len() / 4) * 3 - padding) as u64)
}

fn sha256_hex_base64(data_base64: &str) -> Result<String> {
    let mut hasher = Sha256::new();
    let mut offset = 0;
    while offset < data_base64.len() {
        let end = next_base64_chunk_end(data_base64.len(), offset);
        let decoded = decode_base64_chunk(&data_base64.as_bytes()[offset..end])?;
        hasher.update(&decoded);
        offset = end;
    }
    Ok(hex::encode(hasher.finalize()))
}

fn next_base64_chunk_end(total_len: usize, offset: usize) -> usize {
    let remaining = total_len.saturating_sub(offset);
    if remaining <= BASE64_STREAM_INPUT_CHARS {
        total_len
    } else {
        offset + BASE64_STREAM_INPUT_CHARS
    }
}

fn decode_base64_chunk(chunk: &[u8]) -> Result<Vec<u8>> {
    let mut decoded = vec![0_u8; chunk.len() / 4 * 3];
    let written = STANDARD.decode_slice(chunk, &mut decoded)?;
    decoded.truncate(written);
    Ok(decoded)
}

struct Base64DecodedReader {
    data_base64: Arc<str>,
    offset: usize,
    pending: Vec<u8>,
    pending_offset: usize,
}

impl Base64DecodedReader {
    fn new(data_base64: Arc<str>) -> Self {
        Self {
            data_base64,
            offset: 0,
            pending: Vec::new(),
            pending_offset: 0,
        }
    }
}

impl tokio::io::AsyncRead for Base64DecodedReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        while buf.remaining() > 0 {
            if self.pending_offset < self.pending.len() {
                let available = self.pending.len() - self.pending_offset;
                let to_copy = available.min(buf.remaining());
                buf.put_slice(&self.pending[self.pending_offset..self.pending_offset + to_copy]);
                self.pending_offset += to_copy;
                if self.pending_offset < self.pending.len() {
                    return std::task::Poll::Ready(Ok(()));
                }
                self.pending.clear();
                self.pending_offset = 0;
                continue;
            }

            if self.offset >= self.data_base64.len() {
                return std::task::Poll::Ready(Ok(()));
            }

            let end = next_base64_chunk_end(self.data_base64.len(), self.offset);
            let decoded = decode_base64_chunk(&self.data_base64.as_bytes()[self.offset..end])
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            self.pending = decoded;
            self.offset = end;
        }

        std::task::Poll::Ready(Ok(()))
    }
}

async fn sha256_hex_blob(blob: &BlobHandle, runtime: &BlobRuntime) -> Result<String> {
    match blob.storage() {
        BlobStorage::Inline(bytes) | BlobStorage::Shared(bytes) => Ok(sha256_hex(bytes)),
        BlobStorage::Spilled(_) => {
            let mut reader = runtime.open_reader(blob).await?;
            let mut hasher = Sha256::new();
            let mut chunk = [0_u8; 16 * 1024];
            loop {
                let read = reader.read(&mut chunk).await?;
                if read == 0 {
                    break;
                }
                hasher.update(&chunk[..read]);
            }
            Ok(hex::encode(hasher.finalize()))
        }
    }
}

async fn put_object_stream<R>(
    client: &reqwest::Client,
    object_url: String,
    authorization: String,
    amz_date: String,
    payload_hash: String,
    mime_type: &str,
    content_length: u64,
    reader: R,
) -> Result<reqwest::Response>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let body = Body::wrap_stream(ReaderStream::new(reader));
    Ok(client
        .put(object_url)
        .header(CONTENT_TYPE, mime_type)
        .header(CONTENT_LENGTH, content_length.to_string())
        .header("X-Amz-Content-Sha256", payload_hash)
        .header("X-Amz-Date", amz_date)
        .header(AUTHORIZATION, authorization)
        .body(body)
        .send()
        .await?)
}

fn hmac_sha256(key: &[u8], data: &str) -> Result<Vec<u8>> {
    let mut mac = HmacSha256::new_from_slice(key)?;
    mac.update(data.as_bytes());
    Ok(mac.finalize().into_bytes().to_vec())
}

fn derive_aws_v4_signing_key(
    secret_access_key: &str,
    date_stamp: &str,
    region: &str,
    service: &str,
) -> Vec<u8> {
    let k_date =
        hmac_sha256(format!("AWS4{secret_access_key}").as_bytes(), date_stamp).unwrap_or_default();
    let k_region = hmac_sha256(&k_date, region).unwrap_or_default();
    let k_service = hmac_sha256(&k_region, service).unwrap_or_default();
    hmac_sha256(&k_service, "aws4_request").unwrap_or_default()
}

fn random_hex(bytes_len: usize) -> String {
    let mut bytes = vec![0u8; bytes_len];
    rand::rng().fill(bytes.as_mut_slice());
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll};

    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::extract::{Request, State};
    use axum::http::StatusCode;
    use axum::routing::put;
    use base64::Engine;
    use tokio::io::{AsyncRead, ReadBuf};
    use tokio::net::TcpListener;
    use tokio::sync::{Mutex, oneshot};
    use tokio::time::{Duration, timeout};

    use super::{Uploader, put_object_stream};

    #[derive(Clone)]
    struct R2StreamState {
        request_started: Arc<Mutex<Option<oneshot::Sender<()>>>>,
        body: Arc<Mutex<Vec<u8>>>,
    }

    struct GateReader {
        first: &'static [u8],
        second: &'static [u8],
        gate: Pin<Box<oneshot::Receiver<()>>>,
        stage: GateStage,
    }

    enum GateStage {
        First,
        Waiting,
        Second,
        Done,
    }

    #[tokio::test]
    async fn put_object_stream_starts_request_before_reader_finishes() {
        let (started_tx, started_rx) = oneshot::channel();
        let state = R2StreamState {
            request_started: Arc::new(Mutex::new(Some(started_tx))),
            body: Arc::new(Mutex::new(Vec::new())),
        };
        let server_addr = spawn_r2_server(state.clone()).await;

        let response = timeout(
            Duration::from_millis(500),
            put_object_stream(
                &reqwest::Client::new(),
                format!("http://{server_addr}/bucket/demo.png"),
                "AWS4-HMAC-SHA256 Credential=test".to_string(),
                "20260409T000000Z".to_string(),
                "deadbeef".to_string(),
                "image/png",
                6,
                GateReader::new(started_rx),
            ),
        )
        .await
        .expect("streaming request should start before reader fully drains")
        .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(state.body.lock().await.as_slice(), b"abcdef");
    }

    #[tokio::test]
    async fn uploader_r2_base64_path_streams_decoded_bytes() {
        let state = R2StreamState {
            request_started: Arc::new(Mutex::new(None)),
            body: Arc::new(Mutex::new(Vec::new())),
        };
        let server_addr = spawn_r2_server(state.clone()).await;

        let mut config = crate::test_config();
        config.image_host_mode = "r2".to_string();
        config.r2_endpoint = format!("http://{server_addr}");
        config.r2_bucket = "images".to_string();
        config.r2_access_key_id = "key".to_string();
        config.r2_secret_access_key = "secret".to_string();
        config.r2_public_base_url = "https://img.example.com".to_string();
        let uploader = Uploader::new(reqwest::Client::new(), config);

        let result = uploader
            .upload_inline_data_base64(
                Arc::from(base64::engine::general_purpose::STANDARD.encode(b"abcdef")),
                "image/png",
            )
            .await
            .unwrap();

        assert!(result.url.starts_with("https://img.example.com/images/"));
        assert_eq!(state.body.lock().await.as_slice(), b"abcdef");
    }

    impl GateReader {
        fn new(gate: oneshot::Receiver<()>) -> Self {
            Self {
                first: b"abc",
                second: b"def",
                gate: Box::pin(gate),
                stage: GateStage::First,
            }
        }
    }

    impl AsyncRead for GateReader {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            loop {
                match self.stage {
                    GateStage::First => {
                        buf.put_slice(self.first);
                        self.stage = GateStage::Waiting;
                        return Poll::Ready(Ok(()));
                    }
                    GateStage::Waiting => match self.gate.as_mut().poll(cx) {
                        Poll::Ready(_) => {
                            self.stage = GateStage::Second;
                            continue;
                        }
                        Poll::Pending => return Poll::Pending,
                    },
                    GateStage::Second => {
                        buf.put_slice(self.second);
                        self.stage = GateStage::Done;
                        return Poll::Ready(Ok(()));
                    }
                    GateStage::Done => return Poll::Ready(Ok(())),
                }
            }
        }
    }

    async fn spawn_r2_server(app_state: R2StreamState) -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new()
            .route("/{*path}", put(handle_r2_stream))
            .with_state(app_state);
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        address
    }

    async fn handle_r2_stream(
        State(state): State<R2StreamState>,
        request: Request<Body>,
    ) -> StatusCode {
        if let Some(sender) = state.request_started.lock().await.take() {
            let _ = sender.send(());
        }
        let body = to_bytes(request.into_body(), usize::MAX).await.unwrap();
        *state.body.lock().await = body.to_vec();
        StatusCode::OK
    }
}
