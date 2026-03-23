use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Cached token entry
#[derive(Clone, Debug)]
struct TokenCacheEntry {
    jti: String,
    username: String,
    user_type: String,
    cached_at: Instant,
}

/// Simple in-memory token cache with TTL
pub struct TokenCache {
    inner: Arc<Mutex<HashMap<String, TokenCacheEntry>>>,
    ttl: Duration,
}

impl TokenCache {
    /// Create new cache with specified TTL
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    /// Get cached user info by token (jti)
    pub fn get(&self, jti: &str) -> Option<(String, String)> {
        let mut cache = self.inner.lock().ok()?;
        let now = Instant::now();

        // Clean expired entries occasionally (1% chance)
        if rand::random::<f32>() < 0.01 {
            cache.retain(|_, entry| now.duration_since(entry.cached_at) < self.ttl);
        }

        // Check if entry exists and not expired
        if let Some(entry) = cache.get(jti) {
            if now.duration_since(entry.cached_at) < self.ttl {
                return Some((entry.username.clone(), entry.user_type.clone()));
            }
        }
        None
    }

    /// Insert token into cache
    pub fn insert(&self, jti: String, username: String, user_type: String) {
        if let Ok(mut cache) = self.inner.lock() {
            cache.insert(
                jti.clone(),
                TokenCacheEntry {
                    jti,
                    username,
                    user_type,
                    cached_at: Instant::now(),
                },
            );
        }
    }

    /// Remove token from cache (e.g., on logout)
    pub fn remove(&self, jti: &str) {
        if let Ok(mut cache) = self.inner.lock() {
            cache.remove(jti);
        }
    }

    /// Clear all entries for a user (e.g., on password change)
    pub fn remove_by_username(&self, username: &str) {
        if let Ok(mut cache) = self.inner.lock() {
            cache.retain(|_, entry| entry.username != username);
        }
    }

    /// Clear entire cache
    pub fn clear(&self) {
        if let Ok(mut cache) = self.inner.lock() {
            cache.clear();
        }
    }
}

// Global cache instance (TTL: 5 minutes)
lazy_static::lazy_static! {
    pub static ref TOKEN_CACHE: TokenCache = TokenCache::new(300);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_cache_basic() {
        let cache = TokenCache::new(1); // 1 second TTL for test
        
        // Insert
        cache.insert("jti-1".to_string(), "admin".to_string(), "admin".to_string());
        
        // Get immediately
        let result = cache.get("jti-1");
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "admin");
        
        // Wait for expiration
        std::thread::sleep(Duration::from_secs(2));
        assert!(cache.get("jti-1").is_none());
    }

    #[test]
    fn test_token_cache_remove() {
        let cache = TokenCache::new(300);
        
        cache.insert("jti-1".to_string(), "user1".to_string(), "share".to_string());
        cache.remove("jti-1");
        
        assert!(cache.get("jti-1").is_none());
    }
}
