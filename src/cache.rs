use std::collections::{HashMap, VecDeque};
use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Notify};
use url::Url;
use url::form_urlencoded;

use crate::image_io::{FetchedInlineData, fetch_image_as_inline_data_with_options};
use crate::proxy_image::hostname_matches_domain_patterns;

const EXTERNAL_IMAGE_FETCH_PROXY_PREFIX: &str = "https://gemini.xinbaoai.com/proxy/image?url=";

#[derive(Clone, Debug)]
pub struct CachedInlineData {
    pub mime_type: String,
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

#[derive(Clone)]
pub struct InlineDataUrlFetchService {
    client: reqwest::Client,
    max_image_bytes: usize,
    allow_private_networks: bool,
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
    mime_type: String,
    bytes: Bytes,
    size: u64,
}

struct MemoryCache {
    inner: Mutex<MemoryCacheInner>,
}

struct MemoryCacheInner {
    items: HashMap<String, MemoryEntry>,
    order: VecDeque<String>,
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
        if config.inline_data_url_cache_dir.trim().is_empty()
            && config.inline_data_url_background_fetch_total_timeout_ms == 0
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
            && config.inline_data_url_cache_ttl_ms > 0
            && config.inline_data_url_cache_max_bytes > 0
        {
            Some(Arc::new(DiskCache::new(
                PathBuf::from(config.inline_data_url_cache_dir.clone()),
                Duration::from_millis(config.inline_data_url_cache_ttl_ms),
                config.inline_data_url_cache_max_bytes,
            )))
        } else {
            None
        };

        Some(Arc::new(Self {
            client,
            max_image_bytes,
            allow_private_networks,
            external_proxy_domains: config.image_fetch_external_proxy_domains.clone(),
            memory_cache,
            disk_cache,
            inflight: Arc::new(Mutex::new(HashMap::new())),
            wait_timeout: Duration::from_millis(
                config.inline_data_url_background_fetch_wait_timeout_ms,
            ),
            total_timeout: Duration::from_millis(
                config.inline_data_url_background_fetch_total_timeout_ms,
            ),
            max_inflight: config.inline_data_url_background_fetch_max_inflight,
        }))
    }

    pub async fn fetch(self: &Arc<Self>, raw_url: &str) -> Result<FetchResult> {
        if let Some(cache) = &self.memory_cache {
            if let Some(hit) = cache.get(raw_url).await {
                return Ok(FetchResult {
                    mime_type: hit.mime_type,
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
                    mime_type: hit.mime_type,
                    bytes: hit.bytes,
                    from_cache: true,
                });
            }
        }

        let task = {
            let mut inflight = self.inflight.lock().await;
            if let Some(task) = inflight.get(raw_url) {
                Arc::clone(task)
            } else {
                if inflight.len() >= self.max_inflight {
                    drop(inflight);
                    let fetched = self.direct_fetch(raw_url).await?;
                    let cached = CachedInlineData {
                        mime_type: fetched.mime_type.clone(),
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
                let service = Arc::clone(self);
                let url = raw_url.to_string();
                let task_for_spawn = Arc::clone(&task);
                tokio::spawn(async move {
                    let result = service
                        .direct_fetch(&url)
                        .await
                        .map(|fetched| CachedInlineData {
                            mime_type: fetched.mime_type,
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
                task
            }
        };

        let notified = task.notify.notified();
        if let Some(result) = Self::published_result(&task).await? {
            return Ok(result);
        }

        let wait_timeout = if self.wait_timeout.is_zero() {
            self.total_timeout
        } else {
            self.wait_timeout
        };

        if tokio::time::timeout(wait_timeout, notified).await.is_err() {
            if let Some(result) = Self::published_result(&task).await? {
                return Ok(result);
            }
            return Err(BackgroundFetchWaitTimeoutError {
                wait_timeout,
                total_timeout: self.total_timeout,
            }
            .into());
        }

        match Self::published_result(&task).await? {
            Some(result) => Ok(result),
            None => Err(anyhow!("background fetch finished without result")),
        }
    }

    async fn direct_fetch(&self, raw_url: &str) -> Result<FetchedInlineData> {
        let fetch_url = maybe_wrap_external_proxy_url(raw_url, &self.external_proxy_domains)?;
        let future = fetch_image_as_inline_data_with_options(
            &self.client,
            &fetch_url,
            self.max_image_bytes,
            self.allow_private_networks,
        );
        if self.total_timeout.is_zero() {
            future.await
        } else {
            tokio::time::timeout(self.total_timeout, future)
                .await
                .map_err(|_| anyhow!("inlineData 图片抓取超时"))?
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

    async fn published_result(task: &Arc<FetchTask>) -> Result<Option<FetchResult>> {
        let guard = task.result.lock().await;
        match guard.as_ref() {
            Some(Ok(hit)) => Ok(Some(FetchResult {
                mime_type: hit.mime_type.clone(),
                bytes: hit.bytes.clone(),
                from_cache: false,
            })),
            Some(Err(message)) => Err(anyhow!(message.clone())),
            None => Ok(None),
        }
    }
}

impl MemoryCache {
    fn new(max_bytes: u64) -> Self {
        Self {
            inner: Mutex::new(MemoryCacheInner {
                items: HashMap::new(),
                order: VecDeque::new(),
                max_bytes,
                cur_bytes: 0,
            }),
        }
    }

    async fn get(&self, url: &str) -> Option<CachedInlineData> {
        let mut guard = self.inner.lock().await;
        let item = guard.items.get(url).cloned()?;
        move_to_back(&mut guard.order, url);
        Some(CachedInlineData {
            mime_type: item.mime_type,
            bytes: item.bytes,
        })
    }

    async fn set(&self, url: &str, value: &CachedInlineData) {
        let mut guard = self.inner.lock().await;
        let size = value.bytes.len() as u64;
        if size > guard.max_bytes {
            return;
        }
        if let Some(previous) = guard.items.remove(url) {
            guard.cur_bytes = guard.cur_bytes.saturating_sub(previous.size);
            remove_from_order(&mut guard.order, url);
        }
        while guard.cur_bytes + size > guard.max_bytes {
            let Some(oldest) = guard.order.pop_front() else {
                break;
            };
            if let Some(previous) = guard.items.remove(&oldest) {
                guard.cur_bytes = guard.cur_bytes.saturating_sub(previous.size);
            }
        }
        guard.order.push_back(url.to_string());
        guard.items.insert(
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
            mime_type: meta.mime_type,
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
                mime_type: value.mime_type.clone(),
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

fn move_to_back(order: &mut VecDeque<String>, url: &str) {
    remove_from_order(order, url);
    order.push_back(url.to_string());
}

fn remove_from_order(order: &mut VecDeque<String>, url: &str) {
    if let Some(position) = order.iter().position(|key| key == url) {
        order.remove(position);
    }
}

#[allow(dead_code)]
fn _is_subpath(path: &Path, dir: &Path) -> bool {
    path.starts_with(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fetch_returns_prepublished_result_without_waiting_for_notify() {
        let task = Arc::new(FetchTask {
            notify: Notify::new(),
            result: Mutex::new(Some(Ok(CachedInlineData {
                mime_type: "image/png".to_string(),
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
}
