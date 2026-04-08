use std::future::Future;
use std::pin::Pin;

use anyhow::{Result, anyhow};
use hmac::{Hmac, Mac};
use rand::Rng;
use reqwest::multipart::{Form, Part};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::FormatItem;
use time::macros::format_description;
use tokio::io::AsyncReadExt;
use url::Url;

use crate::blob_runtime::{BlobHandle, BlobRuntime};
use crate::config::Config;

type HmacSha256 = Hmac<Sha256>;

pub type BoxUploadFuture = Pin<Box<dyn Future<Output = Result<UploadResult>> + Send>>;

const UPLOAD_USER_AGENT: &str = "ComfyUI-Banana/1.0";

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

    pub async fn upload_reader<R>(&self, mut reader: R, mime_type: &str) -> Result<UploadResult>
    where
        R: tokio::io::AsyncRead + Unpin + Send,
    {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        self.upload_image(&bytes, mime_type).await
    }

    pub async fn upload_blob(
        &self,
        runtime: &BlobRuntime,
        blob: &BlobHandle,
        mime_type: &str,
    ) -> Result<UploadResult> {
        let reader = runtime.open_reader(blob).await?;
        self.upload_reader(reader, mime_type).await
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
