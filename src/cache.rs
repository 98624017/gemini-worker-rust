use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use bytes::Bytes;
use lru::LruCache;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Notify};
use url::Url;
use url::form_urlencoded;

use crate::image_io::{
    FetchedInlineData, fetch_image_as_inline_data_with_options, hostname_matches_domain_patterns,
    maybe_convert_large_png_to_lossless_webp,
};

const EXTERNAL_IMAGE_FETCH_PROXY_PREFIX: &str = "https://gemini.xinbaoai.com/proxy/image?url=";
const INLINE_DATA_FETCH_RETRY_DELAY: Duration = Duration::from_secs(1);

#[derive(Clone, Debug)]
pub struct CachedInlineData {
    pub mime_type: Arc<str>,
    pub bytes: Bytes,
}

#[derive(Clone, Debug)]
pub struct FetchResult {
    pub mime_type: String,
    pub bytes: Bytes,
    pub from_cache: bool,
}

#[derive(Debug)]
pub struct BackgroundFetchWaitTimeoutError {
    pub wait_timeout: Duration,
    pub total_timeout: Duration,
}

impl Display for BackgroundFetchWaitTimeoutError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "inlineData 图片抓取超时（已转后台继续下载）: wait={:?} total={:?}",
            self.wait_timeout, self.total_timeout
        )
    }
}

impl std::error::Error for BackgroundFetchWaitTimeoutError {}

#[derive(Debug)]
pub(crate) struct ImageFetchStatusError {
    pub(crate) status: StatusCode,
}

impl Display for ImageFetchStatusError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "image fetch failed with status {}", self.status)
    }
}

impl std::error::Error for ImageFetchStatusError {}

#[derive(Clone)]
pub struct InlineDataUrlFetchService {
    client: reqwest::Client,
    max_image_bytes: usize,
    allow_private_networks: bool,
    optimize_large_png_as_webp: bool,
    external_proxy_domains: Vec<String>,
    memory_cache: Option<Arc<MemoryCache>>,
    disk_cache: Option<Arc<DiskCache>>,
    inflight: Arc<Mutex<HashMap<String, Arc<FetchTask>>>>,
    wait_timeout: Duration,
    total_timeout: Duration,
    max_inflight: usize,
}

struct FetchTask {
    notify: Notify,
    result: Mutex<Option<Result<CachedInlineData, String>>>,
}

#[derive(Clone)]
struct MemoryEntry {
    mime_type: Arc<str>,
    bytes: Bytes,
    size: u64,
}

struct MemoryCache {
    inner: Mutex<MemoryCacheInner>,
}

struct MemoryCacheInner {
    items: LruCache<String, MemoryEntry>,
    max_bytes: u64,
    cur_bytes: u64,
}

#[derive(Clone)]
struct DiskCache {
    dir: PathBuf,
    ttl: Duration,
    max_bytes: u64,
    io_lock: Arc<Mutex<()>>,
}

#[derive(Serialize, Deserialize)]
struct DiskMeta {
    mime_type: String,
    expires_at_unix_ms: u64,
    size_bytes: u64,
}

impl InlineDataUrlFetchService {
    pub fn from_config(
        config: &crate::config::Config,
        client: reqwest::Client,
        max_image_bytes: usize,
        allow_private_networks: bool,
    ) -> Option<Arc<Self>> {
        Self::from_config_with_png_optimization(
            config,
            client,
            max_image_bytes,
            allow_private_networks,
            config.enable_request_image_webp_optimization,
            config.inline_data_url_background_fetch_max_inflight,
        )
    }

    pub fn from_response_config(
        config: &crate::config::Config,
        client: reqwest::Client,
        max_image_bytes: usize,
        allow_private_networks: bool,
    ) -> Option<Arc<Self>> {
        Self::from_config_with_png_optimization(
            config,
            client,
            max_image_bytes,
            allow_private_networks,
            false,
            0,
        )
    }

    fn from_config_with_png_optimization(
        config: &crate::config::Config,
        client: reqwest::Client,
        max_image_bytes: usize,
        allow_private_networks: bool,
        optimize_large_png_as_webp: bool,
        max_inflight: usize,
    ) -> Option<Arc<Self>> {
        if config.inline_data_url_cache_dir.trim().is_empty()
            && config
                .inline_data_url_background_fetch_total_timeout
                .is_zero()
            && config.inline_data_url_memory_cache_max_bytes == 0
        {
            return None;
        }

        let memory_cache = if config.inline_data_url_memory_cache_max_bytes > 0 {
            Some(Arc::new(MemoryCache::new(
                config.inline_data_url_memory_cache_max_bytes,
            )))
        } else {
            None
        };
        let disk_cache = if !config.inline_data_url_cache_dir.trim().is_empty()
            && !config.inline_data_url_cache_ttl.is_zero()
            && config.inline_data_url_cache_max_bytes > 0
        {
            Some(Arc::new(DiskCache::new(
                PathBuf::from(config.inline_data_url_cache_dir.clone()),
                config.inline_data_url_cache_ttl,
                config.inline_data_url_cache_max_bytes,
            )))
        } else {
            None
        };

        Some(Arc::new(Self {
            client,
            max_image_bytes,
            allow_private_networks,
            optimize_large_png_as_webp,
            external_proxy_domains: config.image_fetch_external_proxy_domains.clone(),
            memory_cache,
            disk_cache,
            inflight: Arc::new(Mutex::new(HashMap::new())),
            wait_timeout: config.inline_data_url_background_fetch_wait_timeout,
            total_timeout: config.inline_data_url_background_fetch_total_timeout,
            max_inflight,
        }))
    }

    pub async fn fetch(self: &Arc<Self>, raw_url: &str) -> Result<FetchResult> {
        if let Some(cache) = &self.memory_cache {
            if let Some(hit) = cache.get(raw_url).await {
                return Ok(FetchResult {
                    mime_type: hit.mime_type.to_string(),
                    bytes: hit.bytes,
                    from_cache: true,
                });
            }
        }

        if let Some(cache) = &self.disk_cache {
            if let Some(hit) = cache.get(raw_url).await? {
                if let Some(memory) = &self.memory_cache {
                    memory.set(raw_url, &hit).await;
                }
                return Ok(FetchResult {
                    mime_type: hit.mime_type.to_string(),
                    bytes: hit.bytes,
                    from_cache: true,
                });
            }
        }

        let existing = {
            let inflight = self.inflight.lock().await;
            inflight.get(raw_url).map(Arc::clone)
        };
        if let Some(task) = existing {
            return self.wait_for_task(&task).await;
        }

        let task = {
            let mut inflight = self.inflight.lock().await;
            if let Some(task) = inflight.get(raw_url) {
                let task = Arc::clone(task);
                drop(inflight);
                return self.wait_for_task(&task).await;
            }
            if inflight.len() >= self.max_inflight {
                drop(inflight);
                let fetched = self.direct_fetch(raw_url).await?;
                let cached = CachedInlineData {
                    mime_type: Arc::from(fetched.mime_type.as_str()),
                    bytes: fetched.bytes.clone(),
                };
                self.store_in_caches(raw_url, &cached).await;
                return Ok(FetchResult {
                    mime_type: fetched.mime_type,
                    bytes: fetched.bytes,
                    from_cache: false,
                });
            }
            let task = Arc::new(FetchTask {
                notify: Notify::new(),
                result: Mutex::new(None),
            });
            inflight.insert(raw_url.to_string(), Arc::clone(&task));
            task
        };

        let service = Arc::clone(self);
        let url = raw_url.to_string();
        let task_for_spawn = Arc::clone(&task);
        tokio::spawn(async move {
            let result = service
                .direct_fetch(&url)
                .await
                .map(|fetched| CachedInlineData {
                    mime_type: Arc::from(fetched.mime_type.as_str()),
                    bytes: fetched.bytes,
                });
            let published = result
                .as_ref()
                .map(Clone::clone)
                .map_err(|err| err.to_string());
            let mut guard = task_for_spawn.result.lock().await;
            *guard = Some(published);
            drop(guard);
            task_for_spawn.notify.notify_waiters();
            service.inflight.lock().await.remove(&url);
            if let Ok(hit) = &result {
                service.store_in_caches(&url, hit).await;
            }
        });

        self.wait_for_task(&task).await
    }

    async fn direct_fetch(&self, raw_url: &str) -> Result<FetchedInlineData> {
        let fetch_url = maybe_wrap_external_proxy_url(raw_url, &self.external_proxy_domains)?;
        for attempt in 0..2 {
            let result = self.fetch_once(&fetch_url).await;
            match result {
                Ok(fetched) => return Ok(fetched),
                Err(err) if attempt == 0 && should_retry_fetch_error(&err) => {
                    tokio::time::sleep(INLINE_DATA_FETCH_RETRY_DELAY).await;
                }
                Err(err) => return Err(err),
            }
        }

        unreachable!("image fetch retry loop must return within bounded attempts");
    }

    async fn fetch_once(&self, fetch_url: &str) -> Result<FetchedInlineData> {
        let fetched = fetch_and_optimize_with_total_timeout(
            self.total_timeout,
            fetch_image_as_inline_data_with_options(
                &self.client,
                fetch_url,
                self.max_image_bytes,
                self.allow_private_networks,
            ),
            |fetched| async move { Ok(fetched) },
        )
        .await?;

        if self.optimize_large_png_as_webp {
            maybe_convert_large_png_to_lossless_webp(fetched).await
        } else {
            Ok(fetched)
        }
    }

    async fn store_in_caches(&self, raw_url: &str, hit: &CachedInlineData) {
        if let Some(memory) = &self.memory_cache {
            memory.set(raw_url, hit).await;
        }
        if let Some(disk) = &self.disk_cache {
            let _ = disk.set(raw_url, hit).await;
        }
    }

    async fn wait_for_task(self: &Arc<Self>, task: &Arc<FetchTask>) -> Result<FetchResult> {
        let notified = task.notify.notified();
        if let Some(result) = Self::published_result(task).await? {
            return Ok(result);
        }

        let wait_timeout = if self.wait_timeout.is_zero() {
            self.total_timeout
        } else {
            self.wait_timeout
        };

        if tokio::time::timeout(wait_timeout, notified).await.is_err() {
            if let Some(result) = Self::published_result(task).await? {
                return Ok(result);
            }
            return Err(BackgroundFetchWaitTimeoutError {
                wait_timeout,
                total_timeout: self.total_timeout,
            }
            .into());
        }

        match Self::published_result(task).await? {
            Some(result) => Ok(result),
            None => Err(anyhow!("background fetch finished without result")),
        }
    }

    async fn published_result(task: &Arc<FetchTask>) -> Result<Option<FetchResult>> {
        let guard = task.result.lock().await;
        match guard.as_ref() {
            Some(Ok(hit)) => Ok(Some(FetchResult {
                mime_type: hit.mime_type.to_string(),
                bytes: hit.bytes.clone(),
                from_cache: false,
            })),
            Some(Err(message)) => Err(anyhow!(message.clone())),
            None => Ok(None),
        }
    }
}

async fn fetch_and_optimize_with_total_timeout<Fut, Opt, OptFut>(
    total_timeout: Duration,
    fetch_future: Fut,
    optimize: Opt,
) -> Result<FetchedInlineData>
where
    Fut: Future<Output = Result<FetchedInlineData>>,
    Opt: FnOnce(FetchedInlineData) -> OptFut,
    OptFut: Future<Output = Result<FetchedInlineData>>,
{
    let pipeline = async move {
        let fetched = fetch_future.await?;
        optimize(fetched).await
    };

    if total_timeout.is_zero() {
        pipeline.await
    } else {
        tokio::time::timeout(total_timeout, pipeline)
            .await
            .map_err(|_| anyhow!("inlineData 图片抓取超时"))?
    }
}

impl MemoryCache {
    fn new(max_bytes: u64) -> Self {
        Self {
            inner: Mutex::new(MemoryCacheInner {
                items: LruCache::unbounded(),
                max_bytes,
                cur_bytes: 0,
            }),
        }
    }

    async fn get(&self, url: &str) -> Option<CachedInlineData> {
        let mut guard = self.inner.lock().await;
        let item = guard.items.get(url)?;
        Some(CachedInlineData {
            mime_type: item.mime_type.clone(),
            bytes: item.bytes.clone(),
        })
    }

    async fn set(&self, url: &str, value: &CachedInlineData) {
        let mut guard = self.inner.lock().await;
        let size = value.bytes.len() as u64;
        if size > guard.max_bytes {
            return;
        }
        if let Some(previous) = guard.items.pop(url) {
            guard.cur_bytes = guard.cur_bytes.saturating_sub(previous.size);
        }
        while guard.cur_bytes + size > guard.max_bytes {
            let Some((_key, evicted)) = guard.items.pop_lru() else {
                break;
            };
            guard.cur_bytes = guard.cur_bytes.saturating_sub(evicted.size);
        }
        guard.items.put(
            url.to_string(),
            MemoryEntry {
                mime_type: value.mime_type.clone(),
                bytes: value.bytes.clone(),
                size,
            },
        );
        guard.cur_bytes += size;
    }
}

impl DiskCache {
    fn new(dir: PathBuf, ttl: Duration, max_bytes: u64) -> Self {
        Self {
            dir,
            ttl,
            max_bytes,
            io_lock: Arc::new(Mutex::new(())),
        }
    }

    async fn get(&self, url: &str) -> Result<Option<CachedInlineData>> {
        let _guard = self.io_lock.lock().await;
        let stem = cache_key(url);
        let meta_path = self.dir.join(format!("{stem}.json"));
        let body_path = self.dir.join(format!("{stem}.bin"));
        if !meta_path.exists() || !body_path.exists() {
            return Ok(None);
        }

        let meta: DiskMeta = serde_json::from_slice(&std::fs::read(&meta_path)?)?;
        if now_unix_ms() > meta.expires_at_unix_ms {
            let _ = std::fs::remove_file(&meta_path);
            let _ = std::fs::remove_file(&body_path);
            return Ok(None);
        }

        let bytes = Bytes::from(std::fs::read(body_path)?);
        Ok(Some(CachedInlineData {
            mime_type: Arc::from(meta.mime_type.as_str()),
            bytes,
        }))
    }

    async fn set(&self, url: &str, value: &CachedInlineData) -> Result<()> {
        let _guard = self.io_lock.lock().await;
        std::fs::create_dir_all(&self.dir)?;

        let stem = cache_key(url);
        let meta_path = self.dir.join(format!("{stem}.json"));
        let body_path = self.dir.join(format!("{stem}.bin"));
        std::fs::write(&body_path, value.bytes.as_ref())?;
        std::fs::write(
            &meta_path,
            serde_json::to_vec(&DiskMeta {
                mime_type: value.mime_type.to_string(),
                expires_at_unix_ms: now_unix_ms() + self.ttl.as_millis() as u64,
                size_bytes: value.bytes.len() as u64,
            })?,
        )?;
        self.prune_if_needed()?;
        Ok(())
    }

    fn prune_if_needed(&self) -> Result<()> {
        let mut total_bytes = 0u64;
        let mut entries = Vec::new();
        if !self.dir.exists() {
            return Ok(());
        }

        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Ok(meta_bytes) = std::fs::read(&path) else {
                continue;
            };
            let Ok(meta) = serde_json::from_slice::<DiskMeta>(&meta_bytes) else {
                continue;
            };
            total_bytes += meta.size_bytes;
            let modified = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            entries.push((modified, path, meta.size_bytes));
        }

        if total_bytes <= self.max_bytes {
            return Ok(());
        }

        entries.sort_by_key(|(modified, _, _)| *modified);
        for (_, meta_path, size_bytes) in entries {
            if total_bytes <= self.max_bytes {
                break;
            }
            let body_path = meta_path.with_extension("bin");
            let _ = std::fs::remove_file(&meta_path);
            let _ = std::fs::remove_file(&body_path);
            total_bytes = total_bytes.saturating_sub(size_bytes);
        }
        Ok(())
    }
}

fn maybe_wrap_external_proxy_url(raw_url: &str, patterns: &[String]) -> Result<String> {
    if patterns.is_empty() {
        return Ok(raw_url.to_string());
    }
    let parsed = Url::parse(raw_url)?;
    let hostname = parsed.host_str().unwrap_or_default();
    if !hostname_matches_domain_patterns(hostname, patterns) {
        return Ok(raw_url.to_string());
    }
    Ok(format!(
        "{EXTERNAL_IMAGE_FETCH_PROXY_PREFIX}{}",
        form_urlencoded::byte_serialize(raw_url.as_bytes()).collect::<String>()
    ))
}

fn should_retry_fetch_error(err: &anyhow::Error) -> bool {
    if err.to_string() == "inlineData 图片抓取超时" {
        return false;
    }

    if let Some(status_err) = err.downcast_ref::<ImageFetchStatusError>() {
        return matches!(
            status_err.status,
            StatusCode::REQUEST_TIMEOUT
                | StatusCode::TOO_EARLY
                | StatusCode::TOO_MANY_REQUESTS
                | StatusCode::INTERNAL_SERVER_ERROR
                | StatusCode::BAD_GATEWAY
                | StatusCode::SERVICE_UNAVAILABLE
                | StatusCode::GATEWAY_TIMEOUT
        );
    }

    let Some(reqwest_err) = err.downcast_ref::<reqwest::Error>() else {
        return false;
    };

    if reqwest_err.is_body() || reqwest_err.is_decode() {
        return false;
    }

    reqwest_err.is_connect() || reqwest_err.is_request()
}

fn cache_key(url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(url.as_bytes());
    hex::encode(hasher.finalize())
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[allow(dead_code)]
fn _is_subpath(path: &Path, dir: &Path) -> bool {
    path.starts_with(dir)
}

#[cfg(test)]
mod tests {
    use tokio::time::{Duration, sleep};

    use super::*;

    #[tokio::test]
    async fn fetch_returns_prepublished_result_without_waiting_for_notify() {
        let task = Arc::new(FetchTask {
            notify: Notify::new(),
            result: Mutex::new(Some(Ok(CachedInlineData {
                mime_type: Arc::from("image/png"),
                bytes: Bytes::from_static(&[1, 2, 3]),
            }))),
        });
        let mut inflight = HashMap::new();
        inflight.insert(
            "http://example.com/image.png".to_string(),
            Arc::clone(&task),
        );

        let service = Arc::new(InlineDataUrlFetchService {
            client: reqwest::Client::new(),
            max_image_bytes: crate::image_io::DEFAULT_MAX_IMAGE_BYTES,
            allow_private_networks: true,
            optimize_large_png_as_webp: true,
            external_proxy_domains: Vec::new(),
            memory_cache: None,
            disk_cache: None,
            inflight: Arc::new(Mutex::new(inflight)),
            wait_timeout: Duration::from_millis(1),
            total_timeout: Duration::from_millis(1),
            max_inflight: 1,
        });

        let fetched = service.fetch("http://example.com/image.png").await.unwrap();

        assert_eq!(fetched.mime_type, "image/png");
        assert_eq!(fetched.bytes, Bytes::from_static(&[1, 2, 3]));
        assert!(!fetched.from_cache);
    }

    #[tokio::test]
    async fn total_timeout_covers_optimization_stage() {
        let result = fetch_and_optimize_with_total_timeout(
            Duration::from_millis(5),
            async {
                Ok(FetchedInlineData {
                    mime_type: "image/png".to_string(),
                    bytes: Bytes::from(vec![
                        1_u8;
                        crate::image_io::REQUEST_PNG_WEBP_THRESHOLD_BYTES + 1
                    ]),
                })
            },
            |fetched| async move {
                sleep(Duration::from_millis(50)).await;
                Ok(fetched)
            },
        )
        .await;

        let err = result.unwrap_err();
        assert_eq!(err.to_string(), "inlineData 图片抓取超时");
    }
}
