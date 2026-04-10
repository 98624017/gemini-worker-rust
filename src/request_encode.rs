use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::blob_runtime::{BlobHandle, BlobRuntime};
use crate::request_materialize::RequestReplacement;

pub struct EncodedRequestBody {
    pub body_blob: BlobHandle,
    pub content_length: u64,
}

pub async fn encode_request_body(
    mut request: Value,
    replacements: Vec<RequestReplacement>,
    runtime: &BlobRuntime,
) -> Result<EncodedRequestBody> {
    strip_output_from_value(&mut request);

    let replacements = replacements
        .into_iter()
        .map(|replacement| (replacement.json_pointer.clone(), replacement))
        .collect::<HashMap<_, _>>();

    let (writer, reader) = tokio::io::duplex(64 * 1024);
    let runtime_for_store = runtime.clone();
    let store_task = tokio::spawn(async move {
        runtime_for_store
            .store_stream(reader, "application/json".to_string())
            .await
    });

    let mut writer = writer;
    let mut content_length = 0_u64;
    let write_result = write_json_value(
        &mut writer,
        "",
        &request,
        &replacements,
        runtime,
        &mut content_length,
    )
    .await;

    match write_result {
        Ok(()) => {
            writer.shutdown().await?;
            let body_blob = store_task.await??;
            Ok(EncodedRequestBody {
                body_blob,
                content_length,
            })
        }
        Err(err) => {
            store_task.abort();
            Err(err)
        }
    }
}

fn write_json_value<'a, W: AsyncWrite + Unpin + Send + 'a>(
    writer: &'a mut W,
    path: &'a str,
    value: &'a Value,
    replacements: &'a HashMap<String, RequestReplacement>,
    runtime: &'a BlobRuntime,
    content_length: &'a mut u64,
) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
    Box::pin(async move {
        if let Some(replacement) = replacements.get(path) {
            write_replacement(writer, replacement, runtime, content_length).await?;
            return Ok(());
        }

        match value {
            Value::Object(map) => {
                write_counted(writer, b"{", content_length).await?;
                let mut first = true;
                for (key, child) in map {
                    if !first {
                        write_counted(writer, b",", content_length).await?;
                    }
                    first = false;

                    let key_bytes = serde_json::to_vec(key)?;
                    write_counted(writer, &key_bytes, content_length).await?;
                    write_counted(writer, b":", content_length).await?;

                    let child_path = format!("{path}/{}", escape_json_pointer_token(key));
                    write_json_value(
                        writer,
                        &child_path,
                        child,
                        replacements,
                        runtime,
                        content_length,
                    )
                    .await?;
                }
                write_counted(writer, b"}", content_length).await?;
            }
            Value::Array(items) => {
                write_counted(writer, b"[", content_length).await?;
                for (index, child) in items.iter().enumerate() {
                    if index > 0 {
                        write_counted(writer, b",", content_length).await?;
                    }
                    let child_path = format!("{path}/{index}");
                    write_json_value(
                        writer,
                        &child_path,
                        child,
                        replacements,
                        runtime,
                        content_length,
                    )
                    .await?;
                }
                write_counted(writer, b"]", content_length).await?;
            }
            _ => {
                let bytes = serde_json::to_vec(value)?;
                write_counted(writer, &bytes, content_length).await?;
            }
        }

        Ok(())
    })
}

async fn write_replacement<W: AsyncWrite + Unpin>(
    writer: &mut W,
    replacement: &RequestReplacement,
    runtime: &BlobRuntime,
    content_length: &mut u64,
) -> Result<()> {
    let mime_key = serde_json::to_vec("mimeType")?;
    let mime_value = serde_json::to_vec(&replacement.mime_type)?;
    let data_key = serde_json::to_vec("data")?;

    write_counted(writer, b"{", content_length).await?;
    write_counted(writer, &mime_key, content_length).await?;
    write_counted(writer, b":", content_length).await?;
    write_counted(writer, &mime_value, content_length).await?;
    write_counted(writer, b",", content_length).await?;
    write_counted(writer, &data_key, content_length).await?;
    write_counted(writer, b":\"", content_length).await?;
    write_blob_as_base64(writer, runtime, &replacement.blob, content_length).await?;
    write_counted(writer, b"\"}", content_length).await?;
    Ok(())
}

async fn write_blob_as_base64<W: AsyncWrite + Unpin>(
    writer: &mut W,
    runtime: &BlobRuntime,
    blob: &BlobHandle,
    content_length: &mut u64,
) -> Result<()> {
    let mut reader = runtime.open_reader(blob).await?;
    let mut pending = Vec::new();
    let mut chunk = [0_u8; 16 * 1024];

    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            break;
        }

        pending.extend_from_slice(&chunk[..read]);
        let complete_len = pending.len() / 3 * 3;
        if complete_len == 0 {
            continue;
        }

        let mut out = vec![0_u8; complete_len / 3 * 4];
        let written = STANDARD.encode_slice(&pending[..complete_len], &mut out)?;
        write_counted(writer, &out[..written], content_length).await?;
        pending.drain(..complete_len);
    }

    if !pending.is_empty() {
        let mut out = vec![0_u8; pending.len().div_ceil(3) * 4];
        let written = STANDARD.encode_slice(&pending, &mut out)?;
        write_counted(writer, &out[..written], content_length).await?;
    }

    Ok(())
}

async fn write_counted<W: AsyncWrite + Unpin>(
    writer: &mut W,
    bytes: &[u8],
    content_length: &mut u64,
) -> Result<()> {
    writer.write_all(bytes).await?;
    *content_length += bytes.len() as u64;
    Ok(())
}

fn strip_output_from_value(body: &mut Value) {
    if let Some(map) = body.as_object_mut() {
        map.remove("output");
    }

    if let Some(image_config) = body.pointer_mut("/generationConfig/imageConfig") {
        if let Some(map) = image_config.as_object_mut() {
            map.remove("output");
        }
    }

    if let Some(image_config) = body.pointer_mut("/generation_config/image_config") {
        if let Some(map) = image_config.as_object_mut() {
            map.remove("output");
        }
    }
}

fn escape_json_pointer_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}
