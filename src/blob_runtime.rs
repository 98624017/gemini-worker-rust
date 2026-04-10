use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyhow::Result;
use bytes::Bytes;
use tokio::fs::{self, File};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

#[derive(Clone, Debug)]
pub struct BlobRuntimeConfig {
    pub inline_max_bytes: u64,
    pub request_hot_budget_bytes: u64,
    pub global_hot_budget_bytes: u64,
    pub spill_dir: PathBuf,
}

#[derive(Clone, Debug)]
pub struct BlobMeta {
    pub mime_type: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug)]
pub enum BlobStorage {
    Inline(Bytes),
    Spilled(PathBuf),
}

#[derive(Debug)]
struct BlobInner {
    id: u64,
    meta: BlobMeta,
    storage: BlobStorage,
    removed: AtomicBool,
}

#[derive(Clone, Debug)]
pub struct BlobHandle {
    inner: Arc<BlobInner>,
}

impl BlobHandle {
    pub fn id(&self) -> u64 {
        self.inner.id
    }

    pub fn meta(&self) -> &BlobMeta {
        &self.inner.meta
    }

    pub fn storage(&self) -> &BlobStorage {
        &self.inner.storage
    }
}

#[derive(Clone, Debug)]
pub struct BlobRuntime {
    inner: Arc<BlobRuntimeInner>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlobRuntimeStatsSnapshot {
    pub spill_count: u64,
    pub spill_bytes_total: u64,
}

#[derive(Debug)]
struct BlobRuntimeInner {
    cfg: BlobRuntimeConfig,
    next_id: AtomicU64,
    global_hot_bytes: AtomicU64,
    spill_count: AtomicU64,
    spill_bytes_total: AtomicU64,
}

impl BlobRuntime {
    pub fn new(cfg: BlobRuntimeConfig) -> Self {
        Self {
            inner: Arc::new(BlobRuntimeInner {
                cfg,
                next_id: AtomicU64::new(1),
                global_hot_bytes: AtomicU64::new(0),
                spill_count: AtomicU64::new(0),
                spill_bytes_total: AtomicU64::new(0),
            }),
        }
    }

    pub async fn store_bytes(&self, bytes: Vec<u8>, mime_type: String) -> Result<BlobHandle> {
        let size_bytes = bytes.len() as u64;
        let id = self.next_id();
        let storage = if self.try_inline_reserve(size_bytes) {
            BlobStorage::Inline(Bytes::from(bytes))
        } else {
            let spilled_path = self.write_spilled_bytes(id, &bytes).await?;
            self.record_spill(size_bytes);
            BlobStorage::Spilled(spilled_path)
        };

        Ok(self.build_handle(id, mime_type, size_bytes, storage))
    }

    pub async fn store_stream<R>(&self, mut reader: R, mime_type: String) -> Result<BlobHandle>
    where
        R: AsyncRead + Unpin + Send,
    {
        let inline_limit = self
            .inner
            .cfg
            .inline_max_bytes
            .min(self.inner.cfg.request_hot_budget_bytes) as usize;
        let id = self.next_id();

        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 16 * 1024];
        let mut spill_file: Option<File> = None;
        let mut spill_path: Option<PathBuf> = None;

        loop {
            let read = reader.read(&mut chunk).await?;
            if read == 0 {
                break;
            }

            if let Some(file) = spill_file.as_mut() {
                file.write_all(&chunk[..read]).await?;
                continue;
            }

            if buffer.len() + read <= inline_limit {
                buffer.extend_from_slice(&chunk[..read]);
                continue;
            }

            let path = self.spill_path(id);
            ensure_parent_dir(&path).await?;
            let mut file = File::create(&path).await?;
            if !buffer.is_empty() {
                file.write_all(&buffer).await?;
                buffer.clear();
            }
            file.write_all(&chunk[..read]).await?;
            spill_file = Some(file);
            spill_path = Some(path);
        }

        let size_bytes = if let Some(mut file) = spill_file {
            file.flush().await?;
            let path = spill_path.expect("spill path must exist");
            let metadata = fs::metadata(&path).await?;
            self.record_spill(metadata.len());
            return Ok(self.build_handle(
                id,
                mime_type,
                metadata.len(),
                BlobStorage::Spilled(path),
            ));
        } else {
            buffer.len() as u64
        };

        let storage = if self.try_inline_reserve(size_bytes) {
            BlobStorage::Inline(Bytes::from(buffer))
        } else {
            let spilled_path = self.write_spilled_bytes(id, &buffer).await?;
            self.record_spill(size_bytes);
            BlobStorage::Spilled(spilled_path)
        };

        Ok(self.build_handle(id, mime_type, size_bytes, storage))
    }

    pub async fn open_reader(
        &self,
        handle: &BlobHandle,
    ) -> Result<Pin<Box<dyn AsyncRead + Send + 'static>>> {
        match handle.storage() {
            BlobStorage::Inline(bytes) => Ok(Box::pin(Cursor::new(bytes.clone()))),
            BlobStorage::Spilled(path) => Ok(Box::pin(File::open(path).await?)),
        }
    }

    pub async fn read_bytes(&self, handle: &BlobHandle) -> Result<Bytes> {
        match handle.storage() {
            BlobStorage::Inline(bytes) => Ok(bytes.clone()),
            BlobStorage::Spilled(path) => Ok(fs::read(path).await?.into()),
        }
    }

    pub async fn remove(&self, handle: &BlobHandle) -> Result<()> {
        if handle.inner.removed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }

        match handle.storage() {
            BlobStorage::Inline(bytes) => {
                self.release_inline_bytes(bytes.len() as u64);
                Ok(())
            }
            BlobStorage::Spilled(path) => match fs::remove_file(path).await {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(err.into()),
            },
        }
    }

    pub async fn is_inline(&self, handle: &BlobHandle) -> bool {
        matches!(handle.storage(), BlobStorage::Inline(_))
    }

    pub async fn is_spilled(&self, handle: &BlobHandle) -> bool {
        matches!(handle.storage(), BlobStorage::Spilled(_))
    }

    pub fn stats_snapshot(&self) -> BlobRuntimeStatsSnapshot {
        BlobRuntimeStatsSnapshot {
            spill_count: self.inner.spill_count.load(Ordering::Relaxed),
            spill_bytes_total: self.inner.spill_bytes_total.load(Ordering::Relaxed),
        }
    }

    fn build_handle(
        &self,
        id: u64,
        mime_type: String,
        size_bytes: u64,
        storage: BlobStorage,
    ) -> BlobHandle {
        BlobHandle {
            inner: Arc::new(BlobInner {
                id,
                meta: BlobMeta {
                    mime_type,
                    size_bytes,
                },
                storage,
                removed: AtomicBool::new(false),
            }),
        }
    }

    fn next_id(&self) -> u64 {
        self.inner.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn try_inline_reserve(&self, size_bytes: u64) -> bool {
        if size_bytes > self.inner.cfg.inline_max_bytes
            || size_bytes > self.inner.cfg.request_hot_budget_bytes
        {
            return false;
        }

        loop {
            let current = self.inner.global_hot_bytes.load(Ordering::Relaxed);
            let next = match current.checked_add(size_bytes) {
                Some(next) => next,
                None => return false,
            };
            if next > self.inner.cfg.global_hot_budget_bytes {
                return false;
            }
            if self
                .inner
                .global_hot_bytes
                .compare_exchange(current, next, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return true;
            }
        }
    }

    fn release_inline_bytes(&self, size_bytes: u64) {
        self.inner
            .global_hot_bytes
            .fetch_sub(size_bytes, Ordering::AcqRel);
    }

    fn record_spill(&self, size_bytes: u64) {
        self.inner.spill_count.fetch_add(1, Ordering::Relaxed);
        self.inner
            .spill_bytes_total
            .fetch_add(size_bytes, Ordering::Relaxed);
    }

    async fn write_spilled_bytes(&self, id: u64, bytes: &[u8]) -> Result<PathBuf> {
        let path = self.spill_path(id);
        ensure_parent_dir(&path).await?;
        fs::write(&path, bytes).await?;
        Ok(path)
    }

    fn spill_path(&self, id: u64) -> PathBuf {
        self.inner.cfg.spill_dir.join(format!("blob-{id}.bin"))
    }
}

async fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    Ok(())
}
