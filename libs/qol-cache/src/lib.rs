use std::collections::HashMap;
use std::hash::Hash;
use std::time::{Duration, Instant};

struct Entry<V> {
    value: V,
    checked_at: Instant,
}

pub struct TtlCache<K, V> {
    ttl: Duration,
    entries: HashMap<K, Entry<V>>,
}

impl<K, V> TtlCache<K, V> {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: HashMap::new(),
        }
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    pub fn insert(&mut self, key: K, value: V)
    where
        K: Eq + Hash,
    {
        self.prune_expired();
        self.entries.insert(
            key,
            Entry {
                value,
                checked_at: Instant::now(),
            },
        );
    }

    pub fn get(&mut self, key: &K) -> Option<&V>
    where
        K: Eq + Hash,
    {
        let ttl = self.ttl;
        let expired = {
            let entry = self.entries.get_mut(key)?;
            let expired = entry.checked_at.elapsed() > ttl;
            if !expired {
                entry.checked_at = Instant::now();
            }
            expired
        };
        if expired {
            self.entries.remove(key);
            return None;
        }
        self.entries.get(key).map(|entry| &entry.value)
    }

    pub fn contains_key(&mut self, key: &K) -> bool
    where
        K: Eq + Hash,
    {
        self.get(key).is_some()
    }

    pub fn remove(&mut self, key: &K) -> Option<V>
    where
        K: Eq + Hash,
    {
        self.entries.remove(key).map(|entry| entry.value)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    fn prune_expired(&mut self) {
        let now = Instant::now();
        self.entries
            .retain(|_, entry| now.duration_since(entry.checked_at) <= self.ttl);
    }
}

#[cfg(test)]
mod tests {
    use super::TtlCache;
    use std::time::Duration;

    #[test]
    fn get_returns_inserted_value() {
        let mut cache = TtlCache::new(Duration::from_secs(60));
        cache.insert("sha", "abc123");
        assert_eq!(cache.get(&"sha"), Some(&"abc123"));
    }

    #[test]
    fn get_returns_none_for_missing_key() {
        let mut cache: TtlCache<&str, i32> = TtlCache::new(Duration::from_secs(60));
        assert_eq!(cache.get(&"missing"), None);
    }

    #[test]
    fn get_expires_stale_entry_on_read() {
        let mut cache = TtlCache::new(Duration::from_millis(10));
        cache.insert(1u32, "stale");
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(cache.get(&1), None);
        assert!(cache.is_empty());
    }

    #[test]
    fn get_refreshes_checked_at_on_hit() {
        let mut cache = TtlCache::new(Duration::from_millis(100));
        cache.insert(1u32, "alive");
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(cache.get(&1), Some(&"alive"));
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(cache.get(&1), Some(&"alive"));
    }

    #[test]
    fn insert_prunes_expired_entries() {
        let mut cache = TtlCache::new(Duration::from_millis(10));
        cache.insert("old", 1);
        std::thread::sleep(Duration::from_millis(50));
        cache.insert("new", 2);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&"new"), Some(&2));
    }

    #[test]
    fn insert_keeps_fresh_entries_during_prune() {
        let mut cache = TtlCache::new(Duration::from_secs(60));
        cache.insert("a", 1);
        cache.insert("b", 2);
        cache.insert("c", 3);
        assert_eq!(cache.len(), 3);
        assert_eq!(cache.get(&"a"), Some(&1));
        assert_eq!(cache.get(&"b"), Some(&2));
        assert_eq!(cache.get(&"c"), Some(&3));
    }

    #[test]
    fn reinsert_resets_expiry() {
        let mut cache = TtlCache::new(Duration::from_millis(10));
        cache.insert("key", "first");
        std::thread::sleep(Duration::from_millis(50));
        cache.insert("key", "second");
        assert_eq!(cache.get(&"key"), Some(&"second"));
    }

    #[test]
    fn contains_key_tracks_expiry() {
        let mut cache = TtlCache::new(Duration::from_millis(10));
        cache.insert("key", ());
        assert!(cache.contains_key(&"key"));
        std::thread::sleep(Duration::from_millis(50));
        assert!(!cache.contains_key(&"key"));
    }

    #[test]
    fn remove_returns_and_drops_value() {
        let mut cache = TtlCache::new(Duration::from_secs(60));
        cache.insert("key", 42);
        assert_eq!(cache.remove(&"key"), Some(42));
        assert_eq!(cache.remove(&"key"), None);
        assert!(cache.is_empty());
    }

    #[test]
    fn clear_drops_all_entries() {
        let mut cache = TtlCache::new(Duration::from_secs(60));
        cache.insert("a", 1);
        cache.insert("b", 2);
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }
}
