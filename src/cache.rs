// Encoded object bytes cache. 1MB cap (jj-style); FIFO eviction.

use crate::hash::Hash;
use std::collections::VecDeque;

const CACHE_MAX_BYTES: usize = 1024 * 1024; // 1 MiB

pub struct EncodedCache {
    max_bytes: usize,
    total_bytes: usize,
    entries: VecDeque<(Hash, Vec<u8>)>,
}

impl Default for EncodedCache {
    fn default() -> Self {
        Self {
            max_bytes: CACHE_MAX_BYTES,
            total_bytes: 0,
            entries: VecDeque::new(),
        }
    }
}

impl EncodedCache {
    /// Get encoded bytes by hash, if present. Reference is valid until the next mutating call.
    #[inline]
    #[must_use] 
    pub fn get(&self, hash: &Hash) -> Option<&[u8]> {
        self.entries
            .iter()
            .find(|(h, _)| h == hash)
            .map(|(_, v)| v.as_slice())
    }

    /// Insert encoded bytes. Evicts oldest entries until total size <= `max_bytes`.
    pub fn insert(&mut self, hash: Hash, data: Vec<u8>) {
        let len = data.len();
        self.total_bytes += len;
        self.entries.push_back((hash, data));

        while self.total_bytes > self.max_bytes {
            if let Some((_, evicted)) = self.entries.pop_front() {
                self.total_bytes = self.total_bytes.saturating_sub(evicted.len());
            } else {
                break;
            }
        }
    }
}
