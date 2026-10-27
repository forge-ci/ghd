//! Entry- and byte-bounded least-recently-used retention.
//!
//! Weights include the caller's key, slot and retained payload accounting.
//! Oversized entries and either zero limit disable retention for that entry.

use std::collections::{HashMap, VecDeque};

pub struct Cache<V> {
    entries: HashMap<String, (V, usize)>,
    order: VecDeque<String>,
    bytes: usize,
    evictions: u64,
    max_entries: usize,
    max_bytes: usize,
}

impl<V> Cache<V> {
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
            evictions: 0,
            max_entries,
            max_bytes,
        }
    }

    pub fn get(&mut self, key: &str) -> Option<&V> {
        if self.entries.contains_key(key) {
            self.order.retain(|existing| existing != key);
            self.order.push_back(key.into());
        }
        self.entries.get(key).map(|(value, _)| value)
    }

    pub fn insert(&mut self, key: String, value: V, weight: usize) -> bool {
        self.remove(&key);
        if self.max_entries == 0 || weight > self.max_bytes || self.max_bytes == 0 {
            return false;
        }
        while self.entries.len() >= self.max_entries || self.bytes > self.max_bytes - weight {
            let Some(oldest) = self.order.front().cloned() else {
                break;
            };
            self.remove(&oldest);
            self.evictions = self.evictions.saturating_add(1);
        }
        self.bytes += weight;
        self.order.push_back(key.clone());
        self.entries.insert(key, (value, weight));
        true
    }

    pub fn remove(&mut self, key: &str) {
        if let Some((_, weight)) = self.entries.remove(key) {
            self.bytes -= weight;
            self.order.retain(|existing| existing != key);
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.bytes = 0;
    }

    pub fn ordered(&self) -> impl Iterator<Item = (&str, &V)> {
        self.order.iter().filter_map(|key| {
            self.entries
                .get(key)
                .map(|(value, _)| (key.as_str(), value))
        })
    }

    pub fn evictions(&self) -> u64 {
        self.evictions
    }

    pub fn usage(&self) -> (usize, usize) {
        (self.entries.len(), self.bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touches_and_replacements_share_entry_and_byte_limits() {
        let mut cache = Cache::new(2, 10);
        assert!(cache.insert("a".into(), 1, 4));
        assert!(cache.insert("b".into(), 2, 4));
        assert_eq!(cache.get("a"), Some(&1));
        assert!(cache.insert("c".into(), 3, 5));
        assert_eq!(cache.get("b"), None);
        assert_eq!(cache.usage(), (2, 9));
        assert!(cache.insert("a".into(), 4, 9));
        assert_eq!(cache.usage(), (1, 9));
        assert!(!cache.insert("a".into(), 5, 11));
        assert_eq!(cache.usage(), (0, 0));
    }

    #[test]
    fn disabled_retention_and_repeated_evictions_leave_no_bookkeeping() {
        for (entries, bytes) in [(0, 100), (10, 0), (1, 8)] {
            let mut cache = Cache::new(entries, bytes);
            for number in 0..10_000 {
                cache.insert(number.to_string(), (), 8);
                assert!(cache.usage().0 <= entries.min(bytes / 8));
                assert_eq!(cache.order.len(), cache.entries.len());
            }
            cache.clear();
            assert_eq!(cache.usage(), (0, 0));
            assert!(cache.order.is_empty());
        }
    }
}
