use crate::hash::Hash;
use crate::ignore::Ignore;
use crate::index::Index;
use crate::object::MODE_DIR;
use crate::repository::Repository;
use crate::storage::MogStorage;
use crate::store::TreeId;
use crate::tree::TreeEntryRef;
use crate::util::{stdout_is_tty, str_from_utf8_data_shouldve_been_valid_or_we_got_hacked};

use std::borrow::Cow;
use std::path::Path;
use std::fs;

use anyhow::Result;
use walkdir::WalkDir;
use rayon::prelude::*;

pub fn status(repo: &mut Repository) -> Result<()> {
    let index = Index::load(&repo.root)?;
    let head_commit = repo.read_head_commit().ok();
    let head_tree = head_commit
        .and_then(|hash| repo.read_object(&hash).ok())
        .and_then(|obj| obj.try_as_commit_id().ok())
        .map(|id| repo.commit.get_tree(id));

    let head_flat = match head_tree {
        Some(tree_hash) => flatten_tree(repo, tree_hash)?,
        None => SortedFlatTree {
            path_blob: Box::default(),
            path_offsets: [0].into(),
            hashes: Box::default(),
            sorted_order: Box::default(),
        },
    };

    let buckets = collect_status_impl(&index, &head_flat, &repo.root, &repo.ignore);
    print_status(&buckets, &mut std::io::stdout())?;
    Ok(())
}

pub struct FlatTreeBuilder {
    path_blob:    Vec<u8>,
    path_offsets: Vec<u32>,
    hashes:       Vec<Hash>,
}

impl FlatTreeBuilder {
    #[inline]
    pub fn new() -> Self {
        Self {
            path_blob:    Vec::new(),
            path_offsets: Vec::new(),
            hashes:       Vec::new(),
        }
    }

    #[inline]
    pub fn with_capacity(n: usize) -> Self {
        Self {
            path_blob:    Vec::with_capacity(n * 16),
            path_offsets: Vec::with_capacity(n + 1),
            hashes:       Vec::with_capacity(n),
        }
    }

    #[inline]
    pub fn push(&mut self, path: &str, hash: Hash) {
        self.path_offsets.push(self.path_blob.len() as u32);
        self.path_blob.extend_from_slice(path.as_bytes());
        self.hashes.push(hash);
    }

    #[inline]
    pub fn build(mut self) -> SortedFlatTree {
        //
        // Sentinel entry so get_path can always use path_offsets[i+1].
        //
        self.path_offsets.push(self.path_blob.len() as u32);

        let path_blob    = self.path_blob.into_boxed_slice();
        let path_offsets = self.path_offsets.into_boxed_slice();
        let hashes       = self.hashes.into_boxed_slice();

        let mut sorted_order = (0..hashes.len()).collect::<Vec<_>>();
        sorted_order.sort_unstable_by(|&a, &b| {
            let sa = {
                let start = path_offsets[a] as usize;
                let end   = path_offsets[a + 1] as usize;
                &path_blob[start..end]
            };
            let sb = {
                let start = path_offsets[b] as usize;
                let end   = path_offsets[b + 1] as usize;
                &path_blob[start..end]
            };
            sa.cmp(sb)
        });

        SortedFlatTree {
            path_blob,
            path_offsets,
            hashes,
            sorted_order: sorted_order.into_boxed_slice(),
        }
    }
}

impl Default for FlatTreeBuilder {
    fn default() -> Self { Self::new() }
}

// Sorted tree for binary search
#[derive(Default)]
pub struct SortedFlatTree {
    /// Path strings concatenated; no trailing slash.
    pub path_blob: Box<[u8]>,

    /// Start offset of path i in `path_blob`. len+1 entries (last = `path_blob.len()`).
    pub path_offsets: Box<[u32]>,

    /// Hash for path at index i.
    pub hashes: Box<[Hash]>,

    /// Sorted by path for lookup: `sorted_order`[j] = index into `path_offsets/hashes`.
    pub sorted_order: Box<[usize]>,
}

impl SortedFlatTree {
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.hashes.len()
    }

    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    #[must_use]
    pub fn get_path(&self, i: usize) -> &str {
        let start = self.path_offsets[i] as usize;
        let end = self.path_offsets[i + 1] as usize;
        str_from_utf8_data_shouldve_been_valid_or_we_got_hacked(&self.path_blob[start..end])
    }

    /// Binary search by path. Returns Some(hash) if path is a blob in HEAD tree.
    #[inline]
    #[must_use]
    pub fn lookup(&self, path: &str) -> Option<Hash> {
        let sorted = &self.sorted_order;
        let mut lo = 0;
        let mut hi = sorted.len();
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let i = sorted[mid];
            let p = self.get_path(i);
            match path.as_bytes().cmp(p.as_bytes()) {
                std::cmp::Ordering::Less => hi = mid,
                std::cmp::Ordering::Equal => return Some(self.hashes[i]),
                std::cmp::Ordering::Greater => lo = mid + 1,
            }
        }
        None
    }
}

pub fn flatten_tree(repo: &mut Repository<impl MogStorage>, tree_hash: Hash) -> Result<SortedFlatTree> {
    struct Frame {
        tree_id: TreeId,
        prefix: Box<str>
    }

    #[inline]
    fn head_tree_path_at<'a>(path_blob: &'a [u8], path_offsets: &[u32], i: usize) -> &'a [u8] {
        let start = path_offsets[i] as usize;
        let end = path_offsets[i + 1] as usize;
        &path_blob[start..end]
    }

    let mut path_blob = Vec::new();
    let mut path_offsets = Vec::new();
    let mut hashes = Vec::new();

    let object = repo.read_object(&tree_hash)?;
    let root_id = object.try_as_tree_id()?;
    let mut stack = vec![Frame {
        tree_id: root_id,
        prefix: Box::default(),
    }];

    while let Some(frame) = stack.pop() {
        let n = repo.tree.entry_count(frame.tree_id);

        for j in 0..n {
            let TreeEntryRef { mode, hash, name } = repo.tree.get_entry_ref(frame.tree_id, j);

            if mode == MODE_DIR {
                let path = if frame.prefix.is_empty() {
                    Cow::Borrowed(name)
                } else {
                    format!("{}/{}", frame.prefix, name).into()
                }.into();

                let object = repo.read_object(&hash)?;
                let sub_id = object.try_as_tree_id()?;
                stack.push(Frame {
                    tree_id: sub_id,
                    prefix: path
                });

                continue;
            }

            if frame.prefix.is_empty() {
                path_offsets.push(path_blob.len() as u32);
                path_blob.extend_from_slice(name.as_bytes());
            } else {
                path_offsets.push(path_blob.len() as u32);
                path_blob.extend_from_slice(frame.prefix.as_bytes());
                path_blob.push(b'/');
                path_blob.extend_from_slice(name.as_bytes());
            }
            hashes.push(hash);
        }
    }
    path_offsets.push(path_blob.len() as u32);

    let n = hashes.len();
    let mut sorted_order: Box<[_]> = (0..n).collect();
    sorted_order.sort_by(|&a, &b| {
        let sa = head_tree_path_at(&path_blob, &path_offsets, a);
        let sb = head_tree_path_at(&path_blob, &path_offsets, b);
        sa.cmp(sb)
    });

    Ok(SortedFlatTree {
        path_blob: crate::util::vec_into_boxed_slice_noshrink(path_blob),
        path_offsets: crate::util::vec_into_boxed_slice_noshrink(path_offsets),
        hashes: crate::util::vec_into_boxed_slice_noshrink(hashes),
        sorted_order,
    })
}

pub struct StatusBuckets {
    /// Staged: in index, (new or index.hash != head hash).
    pub staged_new_modified: Vec<Box<str>>,

    /// Staged delete: in HEAD, not in index.
    pub staged_deleted: Vec<Box<str>>,

    /// In index, file on disk exists but content differs (mtime/size).
    pub modified: Vec<Box<str>>,
    /// In index, file missing on disk.
    pub deleted: Vec<Box<str>>,

    /// Not in index, file on disk (under repo, not .mog).
    pub untracked: Vec<Box<str>>,
}

pub fn collect_status(repo: &mut Repository) -> Result<StatusBuckets> {
    let index    = Index::load(&repo.root)?;
    let head_flat = match repo.read_head_commit().ok() {
        Some(h) => {
            let obj = repo.read_object(&h)?;
            let cid = obj.try_as_commit_id()?;
            flatten_tree(repo, repo.commit.get_tree(cid))?
        }
        None => SortedFlatTree::default(),
    };
    Ok(collect_status_impl(&index, &head_flat, &repo.root, &repo.ignore))
}

fn collect_status_impl(
    index: &Index,
    head: &SortedFlatTree,
    repo_root: &Path,
    ignore: &Ignore,
) -> StatusBuckets {
    struct IndexResult {
        path: Box<str>,
        staged: bool,
        disk: DiskState,
    }

    enum DiskState { Clean, Modified, Deleted }

    let index_results = (0..index.count).into_par_iter().map(|i| {
        let path_str = index.get_path(i);
        let abs = repo_root.join(path_str);
        let head_hash = head.lookup(path_str);
        let index_hash = index.hashes[i];

        let staged = head_hash != Some(index_hash);

        let disk = match fs::metadata(&abs) {
            Ok(meta) => {
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs() as i64);

                let size = meta.len();
                if index.mtimes[i] != mtime || index.sizes[i] != size {
                    DiskState::Modified
                } else {
                    DiskState::Clean
                }
            }

            Err(_) => DiskState::Deleted,
        };

        IndexResult { path: path_str.into(), staged, disk }
    }).collect::<Vec<_>>();

    let mut staged_new_modified = Vec::new();
    let mut modified            = Vec::new();
    let mut deleted             = Vec::new();

    for r in index_results {
        if r.staged { staged_new_modified.push(r.path.clone()); } // @Clone

        match r.disk {
            DiskState::Modified => modified.push(r.path),
            DiskState::Deleted  => deleted.push(r.path),
            DiskState::Clean    => {}
        }
    }

    //
    // Staged deletes (in HEAD, not in index)
    //
    let mut staged_deleted = Vec::new();
    for j in 0..head.len() {
        let path_str = head.get_path(head.sorted_order[j]);
        if index.find(path_str).is_none() {
            staged_deleted.push(path_str.into());
        }
    }

    let mut untracked = Vec::new();
    for entry in WalkDir::new(repo_root)
        .into_iter()
        .filter_entry(|e| !ignore.is_ignored_abs(e.path()))
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() { continue; }

        let path = entry.path();

        let Ok(rel) = path.strip_prefix(repo_root) else { continue };

        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if rel_str.is_empty() || ignore.is_ignored_rel(&rel_str) { continue; }

        if index.find(&rel_str).is_none() {
            untracked.push(rel_str.into());
        }
    }

    staged_new_modified.sort_unstable();
    staged_deleted.sort_unstable();
    modified.sort_unstable();
    deleted.sort_unstable();
    untracked.sort_unstable();

    StatusBuckets { staged_new_modified, staged_deleted, modified, deleted, untracked }
}

pub fn print_status(buckets: &StatusBuckets, out: &mut (impl std::io::Write + ?Sized)) -> std::io::Result<()> {
    const GREEN:  &str = "\x1b[32m";
    const RED:    &str = "\x1b[31m";
    const YELLOW: &str = "\x1b[33m";
    const BOLD:   &str = "\x1b[1m";
    const RESET:  &str = "\x1b[0m";

    fn section_header(f: &mut (impl std::io::Write + ?Sized), color: &str, title: &str) -> std::io::Result<()> {
        if stdout_is_tty() {
            writeln!(f, "  {color}{title}{RESET}")?;
        } else {
            writeln!(f, "  {title}")?;
        }
        Ok(())
    }

    fn path_line(f: &mut (impl std::io::Write + ?Sized), color: &str, path: &str) -> std::io::Result<()> {
        if stdout_is_tty() {
            writeln!(f, "    {color}{path}{RESET}")?;
        } else {
            writeln!(f, "    {path}")?;
        }
        Ok(())
    }

    let has_staged = !buckets.staged_new_modified.is_empty() || !buckets.staged_deleted.is_empty();
    let has_working = !buckets.modified.is_empty() || !buckets.deleted.is_empty();
    let has_untracked = !buckets.untracked.is_empty();

    if !has_staged && !has_working && !has_untracked {
        writeln!(out, "nothing to commit, working tree clean")?;
        return Ok(());
    }

    if has_staged {
        section_header(out, BOLD, "Changes to be committed:")?;
        for p in &buckets.staged_new_modified {
            path_line(out, GREEN, p)?;
        }
        for p in &buckets.staged_deleted {
            path_line(out, RED, p)?;
        }
        writeln!(out)?;
    }

    if has_working {
        section_header(out, BOLD, "Changes not staged for commit:")?;
        for p in &buckets.modified {
            path_line(out, YELLOW, p)?;
        }
        for p in &buckets.deleted {
            path_line(out, RED, p)?;
        }
        writeln!(out)?;
    }

    if has_untracked {
        const SHOW_UNTRACKED_MAX: usize = 50;

        section_header(out, BOLD, "Untracked files:")?;

        let (show, rest) = if buckets.untracked.len() > SHOW_UNTRACKED_MAX {
            (&buckets.untracked[..SHOW_UNTRACKED_MAX], buckets.untracked.len() - SHOW_UNTRACKED_MAX)
        } else {
            (buckets.untracked.as_slice(), 0)
        };
        for p in show {
            path_line(out, "", p)?;
        }
        if rest > 0 {
            if stdout_is_tty() {
                writeln!(out, "    {YELLOW}... and {rest} more untracked{RESET}\n")?;
            } else {
                writeln!(out, "    ... and {rest} more untracked\n")?;
            }
        }
    }

    Ok(())
}
