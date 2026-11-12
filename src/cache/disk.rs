use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use bytes::Bytes;
use lru::LruCache;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use super::CachedInlineData;

type SharedIndex = Mutex<Option<DiskIndex>>;

#[derive(Clone)]
pub(super) struct DiskCache {
    dir: PathBuf,
    ttl: Duration,
    max_bytes: u64,
    index: Arc<SharedIndex>,
}

#[derive(Serialize, Deserialize)]
struct DiskMeta {
    mime_type: String,
    expires_at_unix_ms: u64,
    size_bytes: u64,
}

struct DiskIndex {
    // 只在写入时更新顺序，保持原有按文件修改时间淘汰的行为。
    entries: LruCache<String, u64>,
    total_bytes: u64,
}

impl DiskCache {
    pub(super) fn new(dir: PathBuf, ttl: Duration, max_bytes: u64) -> Self {
        static INDEXES: OnceLock<StdMutex<HashMap<PathBuf, Weak<SharedIndex>>>> = OnceLock::new();
        let mut indexes = INDEXES
            .get_or_init(StdMutex::default)
            .lock()
            .expect("disk cache registry poisoned");
        indexes.retain(|_, index| index.strong_count() > 0);
        // 请求侧和响应侧使用同一目录，必须共享锁和容量账本。
        let index = indexes
            .get(&dir)
            .and_then(Weak::upgrade)
            .unwrap_or_else(|| {
                let index = Arc::new(Mutex::new(None));
                indexes.insert(dir.clone(), Arc::downgrade(&index));
                index
            });
        Self {
            dir,
            ttl,
            max_bytes,
            index,
        }
    }

    pub(super) async fn get(&self, url: &str) -> Result<Option<CachedInlineData>> {
        let mut index = Arc::clone(&self.index).lock_owned().await;
        let cache = self.clone();
        let stem = cache_key(url);
        tokio::task::spawn_blocking(move || cache.get_sync(&stem, &mut index)).await?
    }

    fn get_sync(
        &self,
        stem: &str,
        index: &mut Option<DiskIndex>,
    ) -> Result<Option<CachedInlineData>> {
        let meta_path = self.dir.join(format!("{stem}.json"));
        let body_path = self.dir.join(format!("{stem}.bin"));
        if !meta_path.exists() || !body_path.exists() {
            return Ok(None);
        }
        let meta: DiskMeta = serde_json::from_slice(&std::fs::read(&meta_path)?)?;
        if now_unix_ms() > meta.expires_at_unix_ms {
            self.remove_entry(stem)?;
            if let Some(index) = index {
                index.remove(stem);
            }
            return Ok(None);
        }
        Ok(Some(CachedInlineData {
            mime_type: Arc::from(meta.mime_type),
            bytes: Bytes::from(std::fs::read(body_path)?),
        }))
    }

    pub(super) async fn set(&self, url: &str, value: &CachedInlineData) -> Result<()> {
        let mut index = Arc::clone(&self.index).lock_owned().await;
        let cache = self.clone();
        let stem = cache_key(url);
        let value = value.clone();
        tokio::task::spawn_blocking(move || {
            let result = cache.set_sync(stem, &value, &mut index);
            if result.is_err() {
                // 写入失败可能只更新了一半文件，下次重新从磁盘核对容量。
                *index = None;
            }
            result
        })
        .await?
    }

    fn set_sync(
        &self,
        stem: String,
        value: &CachedInlineData,
        index: &mut Option<DiskIndex>,
    ) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        if index.is_none() {
            *index = Some(self.load_index()?);
        }
        let index = index.as_mut().expect("disk index initialized");
        let size = value.bytes.len() as u64;
        std::fs::write(self.dir.join(format!("{stem}.bin")), value.bytes.as_ref())?;
        std::fs::write(
            self.dir.join(format!("{stem}.json")),
            serde_json::to_vec(&DiskMeta {
                mime_type: value.mime_type.to_string(),
                expires_at_unix_ms: now_unix_ms().saturating_add(self.ttl.as_millis() as u64),
                size_bytes: size,
            })?,
        )?;
        index.remove(&stem);
        index.entries.put(stem, size);
        index.total_bytes += size;
        while index.total_bytes > self.max_bytes {
            let Some((stem, size)) = index.entries.pop_lru() else {
                break;
            };
            self.remove_entry(&stem)?;
            index.total_bytes = index.total_bytes.saturating_sub(size);
        }
        Ok(())
    }

    fn load_index(&self) -> Result<DiskIndex> {
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Ok(meta) = serde_json::from_slice::<DiskMeta>(&std::fs::read(&path)?) else {
                continue;
            };
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            let modified = entry.metadata()?.modified().unwrap_or(UNIX_EPOCH);
            entries.push((modified, stem.to_string(), meta.size_bytes));
        }
        entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        let mut index = DiskIndex {
            entries: LruCache::unbounded(),
            total_bytes: 0,
        };
        for (_, stem, size) in entries {
            index.entries.put(stem, size);
            index.total_bytes += size;
        }
        Ok(index)
    }

    fn remove_entry(&self, stem: &str) -> Result<()> {
        for extension in ["json", "bin"] {
            remove_if_present(&self.dir.join(format!("{stem}.{extension}")))?;
        }
        Ok(())
    }
}

impl DiskIndex {
    fn remove(&mut self, stem: &str) {
        if let Some(size) = self.entries.pop(stem) {
            self.total_bytes = self.total_bytes.saturating_sub(size);
        }
    }
}

fn remove_if_present(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn cache_key(url: &str) -> String {
    hex::encode(Sha256::digest(url.as_bytes()))
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(bytes: &'static [u8]) -> CachedInlineData {
        CachedInlineData {
            mime_type: Arc::from("image/png"),
            bytes: Bytes::from_static(bytes),
        }
    }

    #[tokio::test]
    async fn disk_cache_shares_capacity_and_accounts_for_overwrites() {
        let directory = tempfile::tempdir().unwrap();
        let first = DiskCache::new(directory.path().into(), Duration::from_secs(60), 4);
        let second = DiskCache::new(directory.path().into(), Duration::from_secs(60), 4);
        first.set("a", &image(b"1234")).await.unwrap();
        second.set("a", &image(b"1")).await.unwrap();
        first.set("b", &image(b"23")).await.unwrap();
        assert_eq!(second.get("a").await.unwrap().unwrap().bytes, b"1"[..]);
        second.set("c", &image(b"45")).await.unwrap();
        assert!(first.get("a").await.unwrap().is_none());
        assert!(first.get("b").await.unwrap().is_some());
        assert!(first.get("c").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn disk_cache_restores_capacity_after_restart() {
        let directory = tempfile::tempdir().unwrap();
        let cache = DiskCache::new(directory.path().into(), Duration::from_secs(60), 4);
        cache.set("a", &image(b"1234")).await.unwrap();
        drop(cache);
        let reopened = DiskCache::new(directory.path().into(), Duration::from_secs(60), 4);
        assert_eq!(reopened.get("a").await.unwrap().unwrap().bytes, b"1234"[..]);
        reopened.set("b", &image(b"5678")).await.unwrap();
        assert!(reopened.get("a").await.unwrap().is_none());
        assert!(reopened.get("b").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn expired_disk_entries_release_capacity() {
        let directory = tempfile::tempdir().unwrap();
        let cache = DiskCache::new(directory.path().into(), Duration::from_secs(60), 4);
        cache.set("a", &image(b"1234")).await.unwrap();
        let path = directory.path().join(format!("{}.json", cache_key("a")));
        let mut meta: DiskMeta = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        meta.expires_at_unix_ms = 0;
        std::fs::write(&path, serde_json::to_vec(&meta).unwrap()).unwrap();
        assert!(cache.get("a").await.unwrap().is_none());
        cache.set("b", &image(b"12")).await.unwrap();
        cache.set("c", &image(b"34")).await.unwrap();
        assert!(cache.get("b").await.unwrap().is_some());
        assert!(cache.get("c").await.unwrap().is_some());
    }
}
