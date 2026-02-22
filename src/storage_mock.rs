use crate::hash::Hash;
use crate::storage::MogStorage;
use crate::util::Xxh3HashMap;

use anyhow::Result;

/// In-memory object store for tests. No disk, no mmap, no eviction.
#[derive(Default)]
pub struct MockStorage {
    objects: Xxh3HashMap<Hash, Box<[u8]>>,
}

impl MockStorage {
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    #[must_use]
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }
}

impl MogStorage for MockStorage {
    #[inline]
    fn exists(&self, hash: &Hash) -> bool {
        self.objects.contains_key(hash)
    }

    #[inline]
    fn read<'a>(&'a self, hash: &Hash) -> Result<&'a [u8]> {
        self.objects.get(hash)
            .map(AsRef::as_ref)
            .ok_or_else(|| anyhow::anyhow!("object not found: {}", crate::hash::hash_to_hex(hash)))
    }

    #[inline]
    fn write(&mut self, hash: Hash, data: impl Into<Box<[u8]>>) {
        self.objects.entry(hash).or_insert_with(|| data.into());
    }

    #[inline]
    fn write_batch<'a>(&mut self, writes: impl Iterator<Item = (Hash, &'a [u8])>) -> Result<()> {
        for (hash, data) in writes {
            self.objects.entry(hash).or_insert_with(|| data.into());
        }
        Ok(())
    }

    #[inline]
    fn flush(&mut self) -> Result<()> { Ok(()) }

    #[inline]
    fn sync(&mut self) -> Result<()> { Ok(()) }

    #[inline]
    fn evict_pages(_data: &[u8]) {}
}
