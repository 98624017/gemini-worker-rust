use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use axum::http::{HeaderValue, StatusCode};
use bytes::Bytes;
use lru::LruCache;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UpstreamBlockCacheKey(String);

#[derive(Clone, Debug)]
pub struct CachedBlockResponse {
    pub status: StatusCode,
    pub content_type: HeaderValue,
    pub body: Bytes,
    pub reason: &'static str,
}

#[derive(Clone, Debug)]
pub struct BlockCacheHit {
    pub status: StatusCode,
    pub content_type: HeaderValue,
    pub body: Bytes,
    pub reason: &'static str,
}

#[derive(Clone, Debug)]
struct StoredBlockResponse {
    response: CachedBlockResponse,
    expires_at: Instant,
}

pub struct UpstreamBlockCache {
    ttl: Duration,
    entries: Mutex<LruCache<UpstreamBlockCacheKey, StoredBlockResponse>>,
}

impl UpstreamBlockCacheKey {
    pub fn new(path: &str, upstream_base_url: &str, request_body: &Value) -> Self {
        let canonical = canonicalize_json(request_body);
        let body_bytes = serde_json::to_vec(&canonical).unwrap_or_else(|_| b"null".to_vec());
        let mut hasher = Sha256::new();
        hasher.update(body_bytes);
        let body_hash = hex::encode(hasher.finalize());
        Self(format!("{path}\n{upstream_base_url}\n{body_hash}"))
    }
}

impl UpstreamBlockCache {
    pub fn new(ttl: Duration, max_entries: usize) -> Option<Self> {
        if ttl.is_zero() || max_entries == 0 {
            return None;
        }
        let capacity = NonZeroUsize::new(max_entries)?;
        Some(Self {
            ttl,
            entries: Mutex::new(LruCache::new(capacity)),
        })
    }

    pub async fn get(&self, key: &UpstreamBlockCacheKey) -> Option<BlockCacheHit> {
        let mut entries = self.entries.lock().await;
        let now = Instant::now();
        let entry = entries.get(key)?;
        if entry.expires_at <= now {
            entries.pop(key);
            return None;
        }
        Some(BlockCacheHit {
            status: entry.response.status,
            content_type: entry.response.content_type.clone(),
            body: entry.response.body.clone(),
            reason: entry.response.reason,
        })
    }

    pub async fn insert(&self, key: UpstreamBlockCacheKey, response: CachedBlockResponse) {
        let mut entries = self.entries.lock().await;
        let now = Instant::now();
        prune_expired_entries(&mut entries, now);
        entries.put(
            key,
            StoredBlockResponse {
                response,
                expires_at: now + self.ttl,
            },
        );
    }
}

pub fn classify_blockable_upstream_error(status: StatusCode, body: &[u8]) -> Option<&'static str> {
    if !matches!(status, StatusCode::BAD_REQUEST | StatusCode::BAD_GATEWAY) {
        return None;
    }
    let text = String::from_utf8_lossy(body).to_ascii_lowercase();
    if text.contains("image_unsafe") {
        return Some("image_unsafe");
    }
    if text.contains("upstream moderation triggered") {
        return Some("upstream_moderation");
    }
    if text.contains("content blocked") {
        return Some("content_blocked");
    }
    None
}

fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_json).collect()),
        Value::Object(map) => {
            let mut sorted = Map::new();
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            for key in keys {
                sorted.insert(key.clone(), canonicalize_json(&map[key]));
            }
            Value::Object(sorted)
        }
        _ => value.clone(),
    }
}

fn prune_expired_entries(
    entries: &mut LruCache<UpstreamBlockCacheKey, StoredBlockResponse>,
    now: Instant,
) {
    let expired_keys: Vec<_> = entries
        .iter()
        .filter(|(_, entry)| entry.expires_at <= now)
        .map(|(key, _)| key.clone())
        .collect();
    for key in expired_keys {
        entries.pop(&key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_ignores_object_order_and_preserves_array_order() {
        let first = json!({
            "contents": [{
                "parts": [
                    {"text": "first"},
                    {"text": "second"}
                ]
            }],
            "generationConfig": {"temperature": 0.7, "topP": 0.9}
        });
        let second = json!({
            "generationConfig": {"topP": 0.9, "temperature": 0.7},
            "contents": [{
                "parts": [
                    {"text": "first"},
                    {"text": "second"}
                ]
            }]
        });
        let reversed_array = json!({
            "generationConfig": {"temperature": 0.7, "topP": 0.9},
            "contents": [{
                "parts": [
                    {"text": "second"},
                    {"text": "first"}
                ]
            }]
        });

        let first_key = UpstreamBlockCacheKey::new(
            "/v1beta/models/demo:generateContent",
            "https://upstream.example",
            &first,
        );
        let second_key = UpstreamBlockCacheKey::new(
            "/v1beta/models/demo:generateContent",
            "https://upstream.example",
            &second,
        );
        let reversed_key = UpstreamBlockCacheKey::new(
            "/v1beta/models/demo:generateContent",
            "https://upstream.example",
            &reversed_array,
        );

        assert_eq!(first_key, second_key);
        assert_ne!(first_key, reversed_key);
    }

    #[test]
    fn classifier_requires_400_or_502_and_known_keyword() {
        assert_eq!(
            classify_blockable_upstream_error(
                StatusCode::BAD_GATEWAY,
                br#"content blocked: {"error_code":"image_unsafe"}"#
            ),
            Some("image_unsafe")
        );
        assert_eq!(
            classify_blockable_upstream_error(
                StatusCode::BAD_REQUEST,
                b"Upstream moderation triggered: output_moderation"
            ),
            Some("upstream_moderation")
        );
        assert_eq!(
            classify_blockable_upstream_error(StatusCode::BAD_REQUEST, b"content blocked"),
            Some("content_blocked")
        );
        assert_eq!(
            classify_blockable_upstream_error(StatusCode::TOO_MANY_REQUESTS, b"image_unsafe"),
            None
        );
        assert_eq!(
            classify_blockable_upstream_error(StatusCode::BAD_GATEWAY, b"temporary upstream error"),
            None
        );
    }

    #[tokio::test]
    async fn cache_returns_hits_until_ttl_expires() {
        let cache = UpstreamBlockCache::new(Duration::from_millis(50), 8).unwrap();
        let key = UpstreamBlockCacheKey::new(
            "/v1/images/generations",
            "https://upstream.example",
            &json!({"prompt": "blocked"}),
        );
        cache
            .insert(
                key.clone(),
                CachedBlockResponse {
                    status: StatusCode::BAD_GATEWAY,
                    content_type: HeaderValue::from_static("application/json"),
                    body: Bytes::from_static(br#"{"error":{"message":"content blocked"}}"#),
                    reason: "content_blocked",
                },
            )
            .await;

        let hit = cache.get(&key).await.expect("cache hit before ttl");
        assert_eq!(hit.status, StatusCode::BAD_GATEWAY);
        assert_eq!(
            hit.content_type,
            HeaderValue::from_static("application/json")
        );
        assert_eq!(hit.reason, "content_blocked");
        assert_eq!(
            hit.body,
            Bytes::from_static(br#"{"error":{"message":"content blocked"}}"#)
        );

        tokio::time::sleep(Duration::from_millis(70)).await;
        assert!(cache.get(&key).await.is_none());
    }

    #[tokio::test]
    async fn cache_evicts_least_recently_used_entry() {
        let cache = UpstreamBlockCache::new(Duration::from_secs(60), 2).unwrap();
        let key_a =
            UpstreamBlockCacheKey::new("/a", "https://upstream.example", &json!({"p": "a"}));
        let key_b =
            UpstreamBlockCacheKey::new("/b", "https://upstream.example", &json!({"p": "b"}));
        let key_c =
            UpstreamBlockCacheKey::new("/c", "https://upstream.example", &json!({"p": "c"}));
        let entry = |message: &'static str| CachedBlockResponse {
            status: StatusCode::BAD_GATEWAY,
            content_type: HeaderValue::from_static("application/json"),
            body: Bytes::from_static(message.as_bytes()),
            reason: "content_blocked",
        };

        cache.insert(key_a.clone(), entry("a")).await;
        cache.insert(key_b.clone(), entry("b")).await;
        assert!(cache.get(&key_a).await.is_some());
        cache.insert(key_c.clone(), entry("c")).await;

        assert!(cache.get(&key_a).await.is_some());
        assert!(cache.get(&key_b).await.is_none());
        assert!(cache.get(&key_c).await.is_some());
    }

    #[tokio::test]
    async fn cache_prunes_expired_entries_before_inserting_new_entry() {
        let cache = UpstreamBlockCache::new(Duration::from_secs(60), 2).unwrap();
        let key_a =
            UpstreamBlockCacheKey::new("/a", "https://upstream.example", &json!({"p": "a"}));
        let key_b =
            UpstreamBlockCacheKey::new("/b", "https://upstream.example", &json!({"p": "b"}));
        let key_c =
            UpstreamBlockCacheKey::new("/c", "https://upstream.example", &json!({"p": "c"}));
        let entry = |message: &'static str| CachedBlockResponse {
            status: StatusCode::BAD_GATEWAY,
            content_type: HeaderValue::from_static("application/json"),
            body: Bytes::from_static(message.as_bytes()),
            reason: "content_blocked",
        };

        cache.insert(key_a.clone(), entry("a")).await;
        cache.insert(key_b.clone(), entry("b")).await;
        {
            let mut entries = cache.entries.lock().await;
            entries
                .get_mut(&key_a)
                .expect("seeded entry should exist")
                .expires_at = Instant::now() - Duration::from_millis(1);
            entries.promote(&key_a);
        }

        cache.insert(key_c.clone(), entry("c")).await;

        assert!(cache.get(&key_a).await.is_none());
        assert!(cache.get(&key_b).await.is_some());
        assert!(cache.get(&key_c).await.is_some());
    }
}
