use crate::hash::Hash;
use crate::object::{MODE_DIR, MODE_EXEC, MODE_FILE};
use crate::repository::Repository;
use crate::object::Object;
use crate::store::TreeId;
use crate::tree::TreeEntry;
use crate::tracy;
use crate::util::{str_from_utf8_data_shouldve_been_valid_or_we_got_hacked, Xxh3HashMap};

use std::collections::HashMap;
use std::path::Path;
use std::fs;

use anyhow::{Result, bail};
use xxhash_rust::xxh3::xxh3_64;

const INDEX_MAGIC: &[u8; 4] = b"MOGG";
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

    pub path_offsets: Vec<u32>,
    pub paths_blob:   Vec<u8>,

    /// Path hash -> entry index (or indices on collision). No duplicate path storage.
    path_index: Xxh3HashMap<u64, Vec<usize>>,
}

pub struct IndexEntryRef<'a> {
    // align 8
    pub hash:  &'a Hash,
    pub mtime: i64,
    pub size:  u64,
    pub path:  &'a str,

    pub mode:  u32,
}

impl<'a> IntoIterator for &'a Index {
    type Item = IndexEntryRef<'a>;
    type IntoIter = IndexIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
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
        let _span = tracy::span!("Index::load");

        let path = repo_root.join(".mog/index");
        if !path.exists() {
            return Ok(Self::default());
        }

        let data = fs::read(path)?;
        Self::decode(&data)
    }

    #[inline]
    pub fn save(&self, repo_root: &Path) -> Result<()> {
        let _span = tracy::span!("Index::save");

        let path = repo_root.join(".mog/index");
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
            bail!("invalid index magic, expected MOGIX");
        }

        let version = u32::from_le_bytes(data[4..8].try_into()?);
        if version != INDEX_VERSION {
            bail!("unsupported index version {version}");
        }

        let count = u32::from_le_bytes(data[8..MINIMAL_HEADER_SIZE_IN_BYTES].try_into()?) as usize;
        let mut cur = MINIMAL_HEADER_SIZE_IN_BYTES;

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
        let paths_blob = data[cur..cur + blob_len].to_vec();

        let mut index = Self {
            count,
            modes,
            hashes,
            mtimes,
            sizes,
            path_offsets,
            paths_blob,
            path_index: HashMap::default(),
        };
        index.build_path_index();
        Ok(index)
    }

    #[inline]
    fn path_hash(path: &str) -> u64 {
        xxh3_64(path.as_bytes())
    }

    #[inline]
    fn build_path_index(&mut self) {
        self.path_index.clear();
        self.path_index.reserve(self.count);
        for i in 0..self.count {
            let h = Self::path_hash(self.get_path(i));
            self.path_index.entry(h).or_default().push(i);
        }
    }

    #[inline]
    #[must_use]
    pub fn get_path_impl<'a>(count: usize, path_offsets: &[u32], paths_blob: &'a [u8], i: usize) -> &'a str {
        let start = path_offsets[i] as usize;
        let end = if i + 1 < count {
            path_offsets[i + 1] as usize
        } else {
            paths_blob.len()
        };

        str_from_utf8_data_shouldve_been_valid_or_we_got_hacked(&paths_blob[start..end])
    }

    #[inline]
    #[must_use]
    pub fn get_path(&self, i: usize) -> &str {
        Self::get_path_impl(self.count, &self.path_offsets, &self.paths_blob, i)
    }

    #[inline]
    #[must_use]
    pub fn find(&self, path: impl AsRef<str>) -> Option<usize> {
        let path_str = path.as_ref();
        let h = Self::path_hash(path_str);
        let list = self.path_index.get(&h)?;
        list.iter().copied().find(|&i| self.get_path(i) == path_str)
    }

    pub fn add(&mut self, path: impl AsRef<str>, hash: Hash, meta: &fs::Metadata) {
        let _span = tracy::span!("Index::add");

        let path_str = path.as_ref();

        let mtime = meta
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let mode = if is_executable(meta) { MODE_EXEC } else { MODE_FILE };
        let size = meta.len();

        let h = Self::path_hash(path_str);
        if let Some(i) = self.path_index.get(&h).and_then(|list| {
            list.iter().copied().find(|&idx| self.get_path(idx) == path_str)
        }) {
            self.modes[i]  = mode;
            self.hashes[i] = hash;
            self.mtimes[i] = mtime;
            self.sizes[i]  = size;
            return;
        }

        self.modes.push(mode);
        self.hashes.push(hash);
        self.mtimes.push(mtime);
        self.sizes.push(size);
        self.path_offsets.push(self.paths_blob.len() as u32);
        self.paths_blob.extend_from_slice(path_str.as_bytes());
        self.path_index.entry(h).or_default().push(self.count);
        self.count += 1;
    }

    pub fn remove(&mut self, path: impl AsRef<str>) -> bool {
        let path_str = path.as_ref();
        let h = Self::path_hash(path_str);
        let (pos, i) = match self.path_index.get(&h) {
            Some(list) => {
                let Some(pos) = list.iter().position(|&idx| self.get_path(idx) == path_str) else {
                    return false;
                };
                (pos, list[pos])
            }
            None => return false,
        };

        self.modes.remove(i);
        self.hashes.remove(i);
        self.mtimes.remove(i);
        self.sizes.remove(i);

        let owned_path_offsets = core::mem::take(&mut self.path_offsets);
        let owned_path_blob = core::mem::take(&mut self.paths_blob);

        for index in (0..self.count).filter(|&j| j != i) {
            let p = Self::get_path_impl(self.count, &owned_path_offsets, &owned_path_blob, index);
            self.path_offsets.push(self.paths_blob.len() as u32);
            self.paths_blob.extend_from_slice(p.as_bytes());
        }

        self.count -= 1;
        let list = self.path_index.get_mut(&h).unwrap();
        list.remove(pos);

        if list.is_empty() {
            self.path_index.remove(&h);
        }

        for list in self.path_index.values_mut() {
            for idx in list.iter_mut() {
                if *idx > i {
                    *idx -= 1;
                }
            }
        }

        true
    }

    /// Recursively update index entries for all files under a checked-out tree.
    #[inline]
    pub fn update_from_tree_recursive(
        &mut self,
        repo: &mut Repository,
        tree_id: TreeId,
        prefix: &str,
    ) -> Result<()> {
        let n = repo.tree.entry_count(tree_id);
        for j in 0..n {
            let TreeEntry { hash, name, .. } = repo.tree.get_entry(tree_id, j);

            let object = repo.read_object(&hash)?;
            match object {
                Object::Blob(_) => {
                    if prefix.is_empty() {
                        let abs = repo.root.join(name.as_ref());
                        let metadata = fs::metadata(&abs)?;
                        self.add(name, hash, &metadata);
                    } else {
                        let mut path = String::with_capacity(prefix.len() + 1 + name.len());
                        path.push_str(prefix);
                        path.push('/');
                        path.push_str(&name);

                        let abs = repo.root.join(&path);
                        let metadata = fs::metadata(&abs)?;

                        self.add(&path, hash, &metadata);
                    }
                }

                Object::Tree(sub_id) => {
                    let path = if prefix.is_empty() {
                        name
                    } else {
                        let mut path = String::with_capacity(prefix.len() + 1 + name.len());
                        path.push_str(prefix);
                        path.push('/');
                        path.push_str(&name);
                        path.into()
                    };

                    self.update_from_tree_recursive(repo, sub_id, &path)?;
                }

                Object::Commit(_) => {}
            }
        }

        Ok(())
    }

    // Fast dirty check: compare mtime + size before hashing.
    // Returns true if the file MIGHT be modified (triggers full hash check).
    #[inline]
    #[must_use]
    pub fn is_dirty(&self, i: usize, metadata: &fs::Metadata) -> bool {
        let _span = tracy::span!("Index::is_dirty");

        let mtime = metadata
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        self.mtimes[i] != mtime || self.sizes[i] != metadata.len()
    }

    #[inline]
    #[must_use]
    pub fn iter(&self) -> IndexIter<'_> {
        IndexIter { mog_index: self, index: 0 }
    }

    // Build and write tree objects from index entries.
    // Groups entries by directory, recursively builds subtrees bottom-up.
    #[inline]
    pub fn write_tree(&self, repo: &mut Repository) -> Result<Hash> {
        //
        // Sort entries by path
        //

        let mut order = (0..self.count).collect::<Vec<_>>();
        order.sort_unstable_by_key(|&i| self.get_path(i));

        let sorted_paths  = order.iter().map(|&i| self.get_path(i)).collect::<Vec<_>>();
        let sorted_modes  = order.iter().map(|&i| self.modes[i]).collect::<Vec<_>>();
        let sorted_hashes = order.iter().map(|&i| self.hashes[i]).collect::<Vec<_>>();

        // Single pass over the sorted array.
        // Consumes a contiguous slice = one directory (implemented iteratively, no recursion).
        let (hash, _consumed) = build_tree(
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
//
// This is a hot path for `mog commit` and is intentionally implemented without recursion.
fn build_tree(
    repo: &mut Repository,
    paths:  &[&str],
    modes:  &[u32],
    hashes: &[Hash],
    dir:    &str,         // current directory prefix e.g. "src/foo"
    start:  usize,        // where in the slice we are
) -> Result<(Hash, usize)> {
    struct Frame<'a> {
        /// Directory prefix (repo-relative, no leading slash), e.g. "src/foo". Root is "".
        dir: &'a str,
        /// Index into `paths` where this directory starts.
        start: usize,
        /// Name to use when adding this directory to its parent. Root has None.
        name_in_parent: Option<&'a str>,
        tree_entries_buffer: Vec<TreeEntry>
    }

    let mut stack: Vec<Frame<'_>> = Vec::new();
    stack.push(Frame {
        dir,
        start,
        name_in_parent: None,
        tree_entries_buffer: Vec::new()
    });

    let mut i = start;

    loop {
        let (cur_dir, cur_dir_len) = {
            let f = stack.last().expect("non-empty stack");
            (f.dir, f.dir.len())
        };

        // Finish the current frame if the next path is outside it (or we've run out of paths).
        let finish_now = if i >= paths.len() {
            true
        } else if cur_dir.is_empty() {
            false
        } else {
            let path_norm = paths[i].trim_start_matches('/');
            !(path_norm.starts_with(cur_dir) && path_norm.as_bytes().get(cur_dir_len) == Some(&b'/'))
        };

        if finish_now {
            let done = stack.pop().expect("non-empty stack");

            let tree_id = repo.tree.extend(&done.tree_entries_buffer);
            let hash = repo.write_object(Object::Tree(tree_id));
            let consumed = i - done.start;

            if let Some(parent) = stack.last_mut() {
                let name = done.name_in_parent.expect("non-root frame must have a name");
                parent.tree_entries_buffer.push(TreeEntry {
                    mode: MODE_DIR,
                    hash,
                    name: name.into() // @Clone
                });
                continue;
            }

            return Ok((hash, consumed));
        }

        let path_norm = paths[i].trim_start_matches('/');
        let rel = if cur_dir.is_empty() {
            path_norm
        } else {
            &path_norm[cur_dir_len + 1..] // skip "dir/"
        };

        if rel.is_empty() {
            i += 1;
            continue;
        }

        match rel.find('/') {
            None => {
                // Direct file child - add blob entry
                let top = stack.last_mut().expect("non-empty stack");
                top.tree_entries_buffer.push(TreeEntry {
                    mode: modes[i],
                    hash: hashes[i],
                    name: rel.into() // @Clone
                });
                i += 1;
            }
            Some(slash) => {
                // Subdirectory - push a new frame and build it first (post-order)
                let subdir_name = &rel[..slash];
                if subdir_name.is_empty() {
                    // Defensive: avoid infinite loops if paths contain repeated/leading slashes.
                    i += 1;
                    continue;
                }

                let subdir_full = if cur_dir.is_empty() {
                    // Root: subdir is the prefix up to the first slash.
                    &path_norm[..slash]
                } else {
                    // Non-root: prefix "dir/subdir".
                    &path_norm[..cur_dir_len + 1 + slash]
                };

                stack.push(Frame {
                    dir: subdir_full,
                    start: i,
                    name_in_parent: Some(subdir_name),
                    tree_entries_buffer: Vec::new()
                });
            }
        }
    }
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
