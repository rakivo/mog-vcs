use crate::hash::Hash;
use crate::tracy;

use std::path::Path;
use std::fs::{File, OpenOptions};

use anyhow::{Result, bail};
use memmap2::{MmapMut, MmapOptions};
use libc::{madvise, MADV_DONTNEED, MADV_SEQUENTIAL, MADV_WILLNEED};

const MAGIC: &[u8; 4] = b"MOGS";
const VERSION: u32 = 1;

const HEADER_SIZE: usize = 128;
const HASH_TABLE_BUCKETS: usize = 1 << 21;  // 2M buckets
const HASH_TABLE_SIZE: usize = HASH_TABLE_BUCKETS * 8;  // 16MB
const DATA_START: u64 = (HEADER_SIZE + HASH_TABLE_SIZE) as u64;

const ENTRY_HEADER_SIZE: usize = 36; // hash(32) + size(4)

pub struct PendingStorageWrite {
    pub hash: Hash,
    pub data: Box<[u8]>,
}

pub struct Storage {
    file: File,
    mmap: MmapMut,
    /// Cached file length so `write_batch` doesn't call `metadata()` every chunk.
    file_len: u64,
    /// Encoded bytes only. No Object clone.
    pending_writes: Vec<PendingStorageWrite>,
}

impl Drop for Storage {
    fn drop(&mut self) {
        _ = self.flush();
    }
}

impl Storage {
    pub fn new(root: &Path) -> Result<Self> {
        let path = root.join("objects.bin");

        if path.exists() {
            Self::open_existing(&path)
        } else {
            Self::create_new(&path)
        }
    }

    fn create_new(path: &Path) -> Result<Self> {
        let _span = tracy::span!("Storage::create_new");

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;

        let initial_size = HEADER_SIZE + HASH_TABLE_SIZE;
        file.set_len(initial_size as u64)?;

        let mut mmap = unsafe { MmapOptions::new().map_mut(&file)? };

        unsafe {
            madvise(
                mmap.as_ptr() as *mut libc::c_void,
                mmap.len(),
                MADV_SEQUENTIAL | MADV_WILLNEED
            );
        }

        // Write header
        mmap[0..4].copy_from_slice(MAGIC);
        mmap[4..8].copy_from_slice(&VERSION.to_le_bytes());
        mmap[8..16].copy_from_slice(&0u64.to_le_bytes());  // count
        mmap[16..24].copy_from_slice(&DATA_START.to_le_bytes());

        mmap.flush()?;

        Ok(Self { file, mmap, file_len: initial_size as u64, pending_writes: Vec::new() })
    }

    fn open_existing(path: &Path) -> Result<Self> {
        let _span = tracy::span!("Storage::open_existing");

        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let mmap = unsafe { MmapOptions::new().map_mut(&file)? };

        if mmap.len() < HEADER_SIZE {
            bail!("corrupted object database");
        }

        if &mmap[0..4] != MAGIC {
            bail!("invalid object database magic");
        }

        let file_len = file.metadata()?.len();
        let ht_end = HEADER_SIZE + HASH_TABLE_SIZE;

        unsafe {
            //
            // Eagerly load only the header + hash table we'll probe it on every lookup.
            //
            madvise(
                mmap.as_ptr() as *mut libc::c_void,
                ht_end.min(mmap.len()),
                MADV_WILLNEED,
            );

            //
            // Tell the kernel it can evict all object data pages immediately.
            //
            let data_len = mmap.len().saturating_sub(ht_end);
            if data_len > 0 {
                madvise(
                    mmap.as_ptr().add(ht_end) as *mut libc::c_void,
                    data_len,
                    MADV_DONTNEED,
                );
            }
        }

        Ok(Self { file, mmap, file_len, pending_writes: Vec::new() })
    }

    #[inline]
    fn hash_to_bucket(hash: &Hash) -> usize {
        let _span = tracy::span!("Storage::hash_to_bucket");

        let h = u64::from_le_bytes(hash[..8].try_into().unwrap());
        (h as usize) % HASH_TABLE_BUCKETS
    }

    #[inline]
    fn get_bucket_offset(&self, bucket: usize) -> u64 {
        let _span = tracy::span!("Storage::get_bucket_offset");

        let offset = HEADER_SIZE + bucket * 8;
        u64::from_le_bytes(self.mmap[offset..offset + 8].try_into().unwrap())
    }

    #[inline]
    fn set_bucket_offset(&mut self, bucket: usize, value: u64) {
        let _span = tracy::span!("Storage::set_bucket_offset");

        let offset = HEADER_SIZE + bucket * 8;
        self.mmap[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    #[inline]
    #[must_use]
    pub fn exists(&self, hash: &Hash) -> bool {
        let _span = tracy::span!("Storage::exists");

        let bucket = Self::hash_to_bucket(hash);
        let mut current_bucket = bucket;

        loop {
            let offset = self.get_bucket_offset(current_bucket);

            if offset == 0 {
                return false;
            }

            let pos = offset as usize;
            if pos + 32 > self.mmap.len() {
                return false;
            }

            if self.mmap[pos..pos + 32] == hash[..] {
                return true;
            }

            current_bucket = (current_bucket + 1) % HASH_TABLE_BUCKETS;
            if current_bucket == bucket {
                return false;
            }
        }
    }

    /// Read encoded object bytes by hash.
    #[inline]
    pub fn read(&self, hash: &Hash) -> Result<&[u8]> {
        let _span = tracy::span!("Storage::read");

        let bucket = Self::hash_to_bucket(hash);
        let mut current_bucket = bucket;

        loop {
            let offset = self.get_bucket_offset(current_bucket);

            if offset == 0 {
                bail!("object not found");
            }

            let pos = offset as usize;

            if self.mmap[pos..pos + 32] == hash[..] {
                let size = u32::from_le_bytes(
                    self.mmap[pos + 32..pos + 36].try_into()?
                ) as usize;

                let data = &self.mmap[pos + 36..pos + 36 + size];
                return Ok(data);
            }

            current_bucket = (current_bucket + 1) % HASH_TABLE_BUCKETS;
            if current_bucket == bucket {
                bail!("object not found");
            }
        }
    }

    /// Push encoded bytes; caller hashes. Used by `write_object`.
    #[inline]
    pub fn write(&mut self, hash: Hash, data: impl Into<Box<[u8]>>) {
        if self.exists(&hash) {
            return;
        }

        self.pending_writes.push(PendingStorageWrite { hash, data: data.into() });
    }

    /// Flush mmap and fsync. Call once after many `write_batch` calls (e.g. at end of add).
    #[inline]
    pub fn sync(&mut self) -> Result<()> {
        self.mmap.flush()?;
        self.file.sync_all()?;
        Ok(())
    }

    #[inline]
    pub fn evict_pages(&self, data: &[u8]) {
        #[cfg(unix)] {
            unsafe {
                let ptr   = data.as_ptr() as usize;
                let end   = ptr + data.len();
                let page  = 4096usize;
                // Round down to page boundary
                let aligned_ptr = (ptr & !(page - 1)) as *mut libc::c_void;
                let aligned_len = end.next_multiple_of(page) - (ptr & !(page - 1));
                libc::madvise(aligned_ptr, aligned_len, libc::MADV_DONTNEED);
            }
        }
    }

    // @Cleanup
    /// Write encoded objects from caller buffers. One buffer, one `write_at`.
    #[inline]
    pub fn write_batch<'a>(&mut self, writes: impl Iterator<Item = (Hash, &'a [u8])>) -> Result<()> {
        self.flush_impl(writes)
    }

    #[inline]
    pub fn flush(&mut self) -> Result<()> {
        let writes = core::mem::take(&mut self.pending_writes);
        self.flush_impl(writes.iter().map(|p| (p.hash, p.data.as_ref())))?;
        self.sync()
    }

    // @Cleanup
    pub fn flush_impl<'a>(&mut self, writes: impl Iterator<Item = (Hash, &'a [u8])>) -> Result<()> {
        let _span = tracy::span!("Storage::flush");

        let mut buf        = Vec::new();
        let mut to_insert  = Vec::new();
        let mut offset     = self.file_len;

        for (hash, encoded) in writes {
            if self.exists(&hash) {
                continue;
            }

            buf.extend_from_slice(&hash);
            buf.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
            buf.extend_from_slice(encoded);
            to_insert.push((hash, offset));
            offset += (ENTRY_HEADER_SIZE + encoded.len()) as u64;
        }

        if buf.is_empty() {
            return Ok(());
        }

        let current_size = self.file_len;
        self.file_len = offset;
        self.file.set_len(self.file_len)?;

        #[cfg(unix)]
        { use std::os::unix::fs::FileExt; self.file.write_at(&buf, current_size)?; }
        #[cfg(not(unix))]
        { self.file.seek(SeekFrom::Start(current_size))?; self.file.write_all(&buf)?; }

        for (hash, offset) in &to_insert {
            let bucket = Self::hash_to_bucket(hash);
            let mut current_bucket = bucket;
            loop {
                if self.get_bucket_offset(current_bucket) == 0 {
                    self.set_bucket_offset(current_bucket, *offset);
                    break;
                }

                current_bucket = (current_bucket + 1) % HASH_TABLE_BUCKETS;
                if current_bucket == bucket { bail!("hash table full"); }
            }
        }

        let count = u64::from_le_bytes(self.mmap[8..16].try_into()?);
        self.mmap[8..16].copy_from_slice(&(count + to_insert.len() as u64).to_le_bytes());

        Ok(())
    }
}
