use crate::hash::Hash;
use crate::object::{Object, Tree, MODE_DIR, MODE_EXEC, MODE_FILE};
use crate::repository::Repository;
use crate::tree_builder::TreeBuilder;

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::fs;

use anyhow::{Result, bail};

const INDEX_MAGIC: &[u8; 4] = b"VXIX";
const INDEX_VERSION: u32 = 1;

// On-disk binary layout:
//
// [magic: 4]
// [version: u32]
// [count: u32]
// [modes: u32 * count]
// [hashes: [u8; 32] * count]
// [mtimes: i64 * count]
// [sizes: u64 * count]
// [path_offsets: u32 * count]
// [paths_blob_len: u32]
// [paths_blob: u8 * paths_blob_len]
//
// Per-entry fixed cost: 4 + 32 + 8 + 8 + 4 = 56 bytes
// Total = 12 + count * 56 + 4 + paths_blob_len

pub const MINIMAL_HEADER_SIZE_IN_BYTES: usize = 12; // magic, version and count
pub const PATHS_BLOB_LEN_SIZE_IN_BYTES: usize = 4;
pub const ENTRY_SIZE_IN_BYTES: usize = 56;

#[derive(Default)]
pub struct Index {
    pub count: usize,

    pub modes:  Vec<u32>,
    pub hashes: Vec<Hash>,
    pub mtimes: Vec<i64>,
    pub sizes:  Vec<u64>,

    // (only touched when path is actually needed)
    pub path_offsets: Vec<u32>,
    pub paths_blob:   Vec<u8>,
}

pub struct IndexEntryRef<'a> {
    pub mode:  u32,
    pub hash:  &'a Hash,
    pub mtime: i64,
    pub size:  u64,
    pub path:  &'a str,
}

pub struct IndexIter<'index> {
    mog_index: &'index Index,
    index: usize,
}

impl<'index> Iterator for IndexIter<'index> {
    type Item = IndexEntryRef<'index>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.mog_index.count {
            return None;
        }

        let e = IndexEntryRef {
            mode:  self.mog_index.modes[self.index],
            hash:  &self.mog_index.hashes[self.index],
            mtime: self.mog_index.mtimes[self.index],
            size:  self.mog_index.sizes[self.index],
            path:  self.mog_index.get_path(self.index),
        };

        self.index += 1;

        Some(e)
    }
}

impl Index {
    #[inline]
    pub fn load(repo_root: &Path) -> Result<Self> {
        let path = repo_root.join(".vx/index");
        if !path.exists() {
            return Ok(Self::default());
        }

        let data = fs::read(path)?;
        Self::decode(&data)
    }

    #[inline]
    pub fn save(&self, repo_root: &Path) -> Result<()> {
        let path = repo_root.join(".vx/index");
        fs::write(path, self.encode())?;
        Ok(())
    }

    #[inline]
    fn total_size_in_bytes(&self) -> usize {
        MINIMAL_HEADER_SIZE_IN_BYTES +
            (self.count * ENTRY_SIZE_IN_BYTES) +
            PATHS_BLOB_LEN_SIZE_IN_BYTES +
            self.paths_blob.len()
    }

    fn encode(&self) -> Vec<u8> {
        let fixed = self.total_size_in_bytes();
        let mut buf = Vec::with_capacity(fixed);

        //
        // Header
        //
        buf.extend_from_slice(INDEX_MAGIC);
        buf.extend_from_slice(&INDEX_VERSION.to_le_bytes());
        buf.extend_from_slice(&(self.count as u32).to_le_bytes());

        for m in &self.modes        { buf.extend_from_slice(&m.to_le_bytes()); }
        for h in &self.hashes       { buf.extend_from_slice(h); }
        for t in &self.mtimes       { buf.extend_from_slice(&t.to_le_bytes()); }
        for s in &self.sizes        { buf.extend_from_slice(&s.to_le_bytes()); }
        for o in &self.path_offsets { buf.extend_from_slice(&o.to_le_bytes()); }

        //
        // Paths blob
        //
        buf.extend_from_slice(&(self.paths_blob.len() as u32).to_le_bytes());
        buf.extend_from_slice(&self.paths_blob);

        buf
    }

    fn decode(data: &[u8]) -> Result<Self> {
        if data.len() < MINIMAL_HEADER_SIZE_IN_BYTES {
            bail!("index too short");
        }

        //
        //
        // Header
        //
        //

        if &data[0..4] != INDEX_MAGIC {
            bail!("invalid index magic, expected VXIX");
        }

        let version = u32::from_le_bytes(data[4..8].try_into()?);
        if version != INDEX_VERSION {
            bail!("unsupported index version {version}");
        }

        let count = u32::from_le_bytes(data[8..MINIMAL_HEADER_SIZE_IN_BYTES].try_into()?) as usize;
        let mut cur = MINIMAL_HEADER_SIZE_IN_BYTES;

        // Helper macro to read a fixed-size field and advance cursor
        macro_rules! read_u32 {
            () => {{
                let v = u32::from_le_bytes(data[cur..cur+4].try_into()?);
                cur += 4;
                v
            }};
        }
        macro_rules! read_i64 {
            () => {{
                let v = i64::from_le_bytes(data[cur..cur+8].try_into()?);
                cur += 8;
                v
            }};
        }
        macro_rules! read_u64 {
            () => {{
                let v = u64::from_le_bytes(data[cur..cur+8].try_into()?);
                cur += 8;
                v
            }};
        }
        macro_rules! read_u256 {
            () => {{
                let mut h = [0u8; 32];
                h.copy_from_slice(&data[cur..cur+32]);
                cur += 32;
                h
            }};
        }

        //
        //
        // Actual SOA data
        //
        //

        // Modes
        let mut modes = Vec::with_capacity(count);
        for _ in 0..count { modes.push(read_u32!()); }

        // Hashes
        let mut hashes = Vec::with_capacity(count);
        for _ in 0..count { hashes.push(read_u256!()); }

        // Mtimes
        let mut mtimes = Vec::with_capacity(count);
        for _ in 0..count { mtimes.push(read_i64!()); }

        // Sizes
        let mut sizes = Vec::with_capacity(count);
        for _ in 0..count { sizes.push(read_u64!()); }

        // Path offsets
        let mut path_offsets = Vec::with_capacity(count);
        for _ in 0..count { path_offsets.push(read_u32!()); }

        // Paths blob
        let blob_len = read_u32!() as usize;
        let paths_blob = data[cur..cur+blob_len].to_vec();

        Ok(Self {
            count,
            modes,
            hashes,
            mtimes,
            sizes,
            path_offsets,
            paths_blob,
        })
    }

    // Get path string for entry i
    #[inline]
    pub fn get_path_impl<'a>(count: usize, path_offsets: &[u32], paths_blob: &'a [u8], i: usize) -> &'a str {
        let start = path_offsets[i] as usize;
        let end = if i + 1 < count {
            path_offsets[i + 1] as usize
        } else {
            paths_blob.len()
        };
        std::str::from_utf8(&paths_blob[start..end]).expect("invalid utf8 in index path")
    }

    // Get path string for entry i
    #[inline]
    pub fn get_path(&self, i: usize) -> &str {
        Self::get_path_impl(self.count, &self.path_offsets, &self.paths_blob, i)
    }

    // Linear scan for path -> index.
    // Good enough for typical repo sizes (<10k files).
    // Can be replaced with sorted index + binary search later.
    #[inline]
    pub fn find(&self, path: &Path) -> Option<usize> {
        let target = path.to_str()?;
        (0..self.count).find(|&i| self.get_path(i) == target)
    }

    // Add or update a file entry.
    // If the path already exists in the index, update it in place.
    // If it's new, append to all arrays.
    pub fn add(&mut self, path: &Path, hash: Hash, meta: &fs::Metadata) {
        let path_str = path.to_str().expect("non-utf8 path");

        let mtime = meta
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let mode = if is_executable(meta) { MODE_EXEC } else { MODE_FILE };
        let size = meta.len();

        if let Some(i) = self.find(path) {
            self.modes[i]  = mode;
            self.hashes[i] = hash;
            self.mtimes[i] = mtime;
            self.sizes[i]  = size;
        } else {
            self.modes.push(mode);
            self.hashes.push(hash);
            self.mtimes.push(mtime);
            self.sizes.push(size);
            self.path_offsets.push(self.paths_blob.len() as u32);
            self.paths_blob.extend_from_slice(path_str.as_bytes());
            self.count += 1;
        }
    }

    // Remove an entry by path.
    // Rebuilds paths_blob (unavoidable with variable-length strings).
    // Returns true if the entry existed.
    pub fn remove(&mut self, path: &Path) -> bool {
        let Some(i) = self.find(path) else { return false; };

        self.modes.remove(i);
        self.hashes.remove(i);
        self.mtimes.remove(i);
        self.sizes.remove(i);

        let owned_path_offsets = core::mem::take(&mut self.path_offsets);
        let owned_path_blob = core::mem::take(&mut self.paths_blob);

        let filtered_path_indexes = (0..self.count).filter(|&j| j != i);

        for index in filtered_path_indexes {
            let p = Self::get_path_impl(self.count, &owned_path_offsets, &owned_path_blob, index);

            self.path_offsets.push(self.paths_blob.len() as u32);
            self.paths_blob.extend_from_slice(p.as_bytes());
        }

        self.count -= 1;
        true
    }

    /// Recursively update index entries for all files under a checked-out tree.
    #[inline]
    pub fn update_from_tree_recursive(
        &mut self,
        repo: &Repository,
        tree: &Tree,
        prefix: &str,
    ) -> Result<()> {
        for entry in tree {
            let path = format!("{prefix}/{}", entry.name);

            match repo.storage.read(entry.hash)? {
                Object::Blob(_) => {
                    let abs      = repo.root.join(&path);
                    let metadata = fs::metadata(&abs)?;
                    self.add(path.as_ref(), *entry.hash, &metadata);
                }
                Object::Tree(subtree) => self.update_from_tree_recursive(repo, &subtree, &path)?,
                Object::Commit(_) => {}
            }
        }

        Ok(())
    }

    // Fast dirty check: compare mtime + size before hashing.
    // Returns true if the file MIGHT be modified (triggers full hash check).
    pub fn is_dirty(&self, i: usize, metadata: &fs::Metadata) -> bool {
        let mtime = metadata
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        self.mtimes[i] != mtime || self.sizes[i] != metadata.len()
    }

    #[inline]
    pub fn iter(&self) -> IndexIter<'_> {
        IndexIter { mog_index: self, index: 0 }
    }

    // Build and write tree objects from index entries.
    // Groups entries by directory, recursively builds subtrees bottom-up.
    #[inline]
    pub fn write_tree_recursive(&self, repo: &Repository) -> Result<Hash> {
        //
        // Sort entries by path
        //

        let mut order = (0..self.count).collect::<Vec<_>>();
        order.sort_unstable_by_key(|&i| self.get_path(i));

        let sorted_paths  = order.iter().map(|&i| self.get_path(i)).collect::<Vec<_>>();
        let sorted_modes  = order.iter().map(|&i| self.modes[i]).collect::<Vec<_>>();;
        let sorted_hashes = order.iter().map(|&i| self.hashes[i]).collect::<Vec<_>>();;

        // Single recursive pass over the sorted array
        // Each call consumes a contiguous slice = one directory
        let (hash, consumed) = build_tree_recursive(
            repo,
            &sorted_paths,
            &sorted_modes,
            &sorted_hashes,
            "",   // current directory prefix
            0,    // start index
        )?;

        Ok(hash)
    }
}

// Builds a tree for `dir` by consuming a contiguous slice of sorted entries.
// Returns (tree_hash, how_many_entries_consumed).
// No HashMap, no String allocation, no cloning - just slices.
fn build_tree_recursive(
    repo: &Repository,
    paths:  &[&str],
    modes:  &[u32],
    hashes: &[Hash],
    dir:    &str,         // current directory prefix e.g. "src/foo"
    start:  usize,        // where in the slice we are
) -> Result<(Hash, usize)> {
    let mut builder = TreeBuilder::new();
    let mut i = start;

    while i < paths.len() {
        let path = paths[i];

        //
        // Stop if this path is no longer inside our directory
        //
        let inside = if dir.is_empty() {
            true
        } else {
            path.starts_with(dir) && path.as_bytes().get(dir.len()) == Some(&b'/')
        };

        if !inside { break; }

        // Get the part of the path relative to current dir
        let rel = if dir.is_empty() {
            path
        } else {
            &path[dir.len() + 1..]  // skip "dir/"
        };

        // Is this a direct child or does it go deeper?
        match rel.find('/') {
            None => {
                //
                // Direct file child - add blob entry
                //
                builder.add(modes[i], hashes[i], rel);
                i += 1;
            }
            Some(slash) => {
                //
                // Goes into a subdirectory - find the subdir name
                //
                let subdir_name = &rel[..slash];
                let subdir_full = if dir.is_empty() {
                    Cow::Borrowed(subdir_name)
                } else {
                    Cow::Owned(format!("{dir}/{subdir_name}"))
                };

                //
                // Recursively build subtree, consuming all entries under it
                //
                let (subtree_hash, consumed) = build_tree_recursive(
                    repo,
                    paths,
                    modes,
                    hashes,
                    &subdir_full,
                    i,
                )?;

                builder.add(MODE_DIR, subtree_hash, subdir_name);
                i += consumed;
            }
        }
    }

    let tree = builder.build();
    let hash = repo.storage.write(&Object::Tree(tree))?;
    Ok((hash, i - start))
}

#[cfg(unix)]
fn is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_: &fs::Metadata) -> bool {
    false
}
