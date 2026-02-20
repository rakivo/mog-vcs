use crate::{hash::Hash, util::Xxh3HashMap};
use std::collections::VecDeque;

const CACHE_MAX_BYTES: usize = 1024 * 1024; // 1 MiB

#[derive(Default)]
pub struct ObjectCache {
    map:         Xxh3HashMap<Hash, Box<[u8]>>,
    order:       VecDeque<Hash>,
    total_bytes: usize,
}

impl ObjectCache {
    /// Get encoded bytes by hash, if present. Reference is valid until the next mutating call.
    #[inline]
    #[must_use]
    pub fn get(&self, hash: &Hash) -> Option<&[u8]> {
        self.map.get(hash).map(|v| &**v)
    }

    #[inline]
    #[must_use]
    pub fn contains(&self, hash: &Hash) -> bool {
        self.map.contains_key(hash)
    }

    /// Insert encoded bytes. Evicts oldest entries until total size <= `max_bytes`.
    #[inline]
    pub fn insert(&mut self, hash: Hash, data: impl Into<Box<[u8]>>) {
        if self.map.contains_key(&hash) {
            return;
        }

        let data = data.into();
        self.total_bytes += data.len();

        self.map.insert(hash, data);
        self.order.push_back(hash);

        while self.total_bytes > CACHE_MAX_BYTES {
            if let Some(evicted_hash) = self.order.pop_front() {
                if let Some(evicted) = self.map.remove(&evicted_hash) {
                    self.total_bytes = self.total_bytes.saturating_sub(evicted.len());
                }
            } else {
                break;
            }
        }
    }
}
