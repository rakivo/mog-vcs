use crate::{hash::Hash, util::Xxh3HashMap};
use std::collections::{VecDeque, HashMap};

const CACHE_MAX_BYTES: usize = 1024 * 1024; // 1 MiB

#[derive(Default)]
pub struct EncodedCache {
    map:         Xxh3HashMap<Hash, Vec<u8>>,
    order:       VecDeque<Hash>,
    total_bytes: usize,
}

impl EncodedCache {
    /// Get encoded bytes by hash, if present. Reference is valid until the next mutating call.
    #[inline]
    #[must_use]
    pub fn get(&self, hash: &Hash) -> Option<&[u8]> {
        self.map.get(hash).map(|v| v.as_slice())
    }

    #[inline]
    #[must_use]
    pub fn contains(&self, hash: &Hash) -> bool {
        self.map.contains_key(hash)
    }

    /// Insert encoded bytes. Evicts oldest entries until total size <= `max_bytes`.
    #[inline]
    pub fn insert(&mut self, hash: Hash, data: Vec<u8>) {
        if self.map.contains_key(&hash) {
            return;
        }

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
