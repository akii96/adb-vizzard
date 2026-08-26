//! Session-scoped artifact cache, held in memory.
//!
//! Artifacts are never written to disk. Two reasons:
//!
//! 1. **Size.** A child run contributes about 3.6 KB, so a 20-child sweep is ~72 KB.
//!    Keeping a whole working session in RAM costs less than a single screenshot,
//!    and there is nothing to persist that would pay for the complexity.
//! 2. **Secrets.** `commands.txt` embeds the `-e` environment of the docker run,
//!    which in real pulls includes `HF_TOKEN`. Spooling that to disk would leave
//!    third-party credentials sitting in a cache directory indefinitely.
//!
//! Everything is dropped when the process exits, and buffers are zeroized on the
//! way out so a token does not linger in freed memory.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;
use zeroize::Zeroize;

/// Default ceiling on cached bytes. Generous for v1's metrics-only payloads, and
/// already sized for v2's megabyte docker logs.
pub const DEFAULT_CAP_MB: u64 = 256;

struct Entry {
    bytes: Vec<u8>,
    /// Monotonic counter rather than a timestamp: LRU ordering must not depend on
    /// the wall clock, which can jump.
    last_used: u64,
}

impl Drop for Entry {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

#[derive(Default)]
struct Inner {
    entries: HashMap<String, Entry>,
    total_bytes: u64,
    tick: u64,
}

impl Inner {
    fn touch(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }

    /// Drops least-recently-used entries until the total fits `cap`.
    fn evict_to(&mut self, cap: u64) {
        if self.total_bytes <= cap {
            return;
        }

        let mut order: Vec<(String, u64, u64)> = self
            .entries
            .iter()
            .map(|(key, entry)| (key.clone(), entry.last_used, entry.bytes.len() as u64))
            .collect();
        order.sort_by_key(|(_, last_used, _)| *last_used);

        for (key, _, size) in order {
            if self.total_bytes <= cap {
                break;
            }
            // Removing drops the Entry, which zeroizes its buffer.
            if self.entries.remove(&key).is_some() {
                self.total_bytes = self.total_bytes.saturating_sub(size);
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CacheStats {
    pub entries: usize,
    pub bytes: u64,
    pub cap_bytes: u64,
}

pub struct ArtifactCache {
    inner: Mutex<Inner>,
    cap_bytes: Mutex<u64>,
}

impl ArtifactCache {
    pub fn new(cap_mb: u64) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            cap_bytes: Mutex::new(cap_mb.saturating_mul(1024 * 1024)),
        }
    }

    fn key(run_id: &str, artifact_path: &str) -> String {
        format!("{run_id}::{artifact_path}")
    }

    fn cap(&self) -> u64 {
        self.cap_bytes
            .lock()
            .map(|guard| *guard)
            .unwrap_or(u64::MAX)
    }

    /// Returns a copy of the cached bytes, refreshing the entry's LRU position.
    pub fn get(&self, run_id: &str, artifact_path: &str) -> Option<Vec<u8>> {
        let mut inner = self.inner.lock().ok()?;
        let tick = inner.touch();
        let entry = inner.entries.get_mut(&Self::key(run_id, artifact_path))?;
        entry.last_used = tick;
        Some(entry.bytes.clone())
    }

    /// Stores bytes, evicting older entries if this pushes past the cap.
    ///
    /// A payload larger than the whole cap is simply not stored, rather than
    /// evicting everything to make room for something that cannot fit.
    pub fn put(&self, run_id: &str, artifact_path: &str, bytes: &[u8]) {
        let cap = self.cap();
        let size = bytes.len() as u64;
        if size > cap {
            return;
        }

        let Ok(mut inner) = self.inner.lock() else {
            return;
        };

        let tick = inner.touch();
        let key = Self::key(run_id, artifact_path);

        if let Some(previous) = inner.entries.insert(
            key,
            Entry {
                bytes: bytes.to_vec(),
                last_used: tick,
            },
        ) {
            inner.total_bytes = inner
                .total_bytes
                .saturating_sub(previous.bytes.len() as u64);
        }
        inner.total_bytes += size;

        inner.evict_to(cap);
    }

    pub fn stats(&self) -> CacheStats {
        let cap_bytes = self.cap();
        match self.inner.lock() {
            Ok(inner) => CacheStats {
                entries: inner.entries.len(),
                bytes: inner.total_bytes,
                cap_bytes,
            },
            Err(_) => CacheStats {
                entries: 0,
                bytes: 0,
                cap_bytes,
            },
        }
    }

    /// Drops everything held for this session.
    pub fn clear(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.entries.clear();
            inner.total_bytes = 0;
        }
    }

    /// Applies a new ceiling, evicting immediately if the cache is now over it.
    pub fn set_cap_mb(&self, cap_mb: u64) {
        let cap = cap_mb.saturating_mul(1024 * 1024);
        if let Ok(mut guard) = self.cap_bytes.lock() {
            *guard = cap;
        }
        if let Ok(mut inner) = self.inner.lock() {
            inner.evict_to(cap);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_bytes() {
        let cache = ArtifactCache::new(8);
        assert!(cache.get("run1", "commands.txt").is_none());

        cache.put("run1", "commands.txt", b"hello");
        assert_eq!(
            cache.get("run1", "commands.txt").as_deref(),
            Some(&b"hello"[..])
        );

        let stats = cache.stats();
        assert_eq!(stats.entries, 1);
        assert_eq!(stats.bytes, 5);
    }

    #[test]
    fn keys_are_scoped_per_run_and_path() {
        let cache = ArtifactCache::new(8);
        cache.put("run1", "commands.txt", b"one");
        cache.put("run2", "commands.txt", b"two");
        cache.put("run1", "benchmark_results/0_vllm_bench_serve/yaml", b"{}");

        assert_eq!(
            cache.get("run1", "commands.txt").as_deref(),
            Some(&b"one"[..])
        );
        assert_eq!(
            cache.get("run2", "commands.txt").as_deref(),
            Some(&b"two"[..])
        );
        assert_eq!(cache.stats().entries, 3);
    }

    #[test]
    fn overwriting_a_key_does_not_double_count_bytes() {
        let cache = ArtifactCache::new(8);
        cache.put("run1", "a", b"1234");
        cache.put("run1", "a", b"12345678");

        let stats = cache.stats();
        assert_eq!(stats.entries, 1);
        assert_eq!(stats.bytes, 8);
    }

    #[test]
    fn clear_drops_everything() {
        let cache = ArtifactCache::new(8);
        cache.put("run1", "a", b"data");
        cache.clear();

        assert!(cache.get("run1", "a").is_none());
        assert_eq!(cache.stats().bytes, 0);
    }

    #[test]
    fn evicts_least_recently_used_over_the_cap() {
        // 10-byte cap, exercised directly rather than through the MB constructor.
        let cache = ArtifactCache {
            inner: Mutex::new(Inner::default()),
            cap_bytes: Mutex::new(10),
        };

        cache.put("run1", "a", b"1234");
        cache.put("run1", "b", b"5678");
        // Touch "a" so "b" becomes the least recently used.
        assert!(cache.get("run1", "a").is_some());
        cache.put("run1", "c", b"9012");

        assert!(
            cache.get("run1", "b").is_none(),
            "b should have been evicted"
        );
        assert!(cache.get("run1", "a").is_some());
        assert!(cache.get("run1", "c").is_some());
        assert!(cache.stats().bytes <= 10);
    }

    #[test]
    fn a_payload_larger_than_the_cap_is_not_stored() {
        let cache = ArtifactCache {
            inner: Mutex::new(Inner::default()),
            cap_bytes: Mutex::new(4),
        };

        cache.put("run1", "small", b"ab");
        cache.put("run1", "huge", b"way too many bytes");

        assert!(cache.get("run1", "huge").is_none());
        // The oversized write must not have evicted what was already there.
        assert!(cache.get("run1", "small").is_some());
    }

    #[test]
    fn lowering_the_cap_evicts_immediately() {
        let cache = ArtifactCache::new(1);
        cache.put("run1", "a", &vec![0u8; 600 * 1024]);
        assert_eq!(cache.stats().entries, 1);

        // Drop below what is held; the entry has to go.
        cache.set_cap_mb(0);
        assert_eq!(cache.stats().entries, 0);
    }
}
