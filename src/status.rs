// Status: DOD. Flat HEAD tree, parallel path/hash arrays, single pass over index + working tree.

use crate::hash::Hash;
use crate::ignore::Ignore;
use crate::index::Index;
use crate::object::MODE_DIR;
use crate::repository::Repository;
use crate::store::TreeId;
use crate::tree::TreeEntry;

use std::path::Path;
use std::fs;

use anyhow::Result;
use walkdir::WalkDir;

// --- HEAD tree as flat SoA: one blob per path, sorted for binary search ---

pub struct HeadTreeFlat {
    /// Path strings concatenated; no trailing slash.
    path_blob: Vec<u8>,
    /// Start offset of path i in `path_blob`. len+1 entries (last = `path_blob.len()`).
    path_offsets: Vec<u32>,
    /// Hash for path at index i.
    hashes: Vec<Hash>,
    /// Sorted by path for lookup: `sorted_order`[j] = index into `path_offsets/hashes`.
    sorted_order: Vec<usize>,
}

impl HeadTreeFlat {
    #[inline]
    pub fn len(&self) -> usize {
        self.hashes.len()
    }

    #[inline]
    pub fn get_path(&self, i: usize) -> &str {
        let start = self.path_offsets[i] as usize;
        let end = self.path_offsets[i + 1] as usize;
        std::str::from_utf8(&self.path_blob[start..end]).expect("utf8")
    }

    /// Binary search by path. Returns Some(hash) if path is a blob in HEAD tree.
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

/// Iterative stack walk: flatten tree to (path, hash) for blobs only. No recursion.
fn flatten_head_tree(repo: &mut Repository, tree_hash: Hash) -> Result<HeadTreeFlat> {
    struct Frame {
        tree_id: TreeId,
        prefix: Box<str>
    }

    let mut path_blob = Vec::new();
    let mut path_offsets = Vec::new();
    let mut hashes = Vec::new();

    let obj = repo.read_object(&tree_hash)?;
    let root_id = obj.try_as_tree_id()?;
    let mut stack = vec![Frame {
        tree_id: root_id,
        prefix: Box::default(),
    }];

    while let Some(frame) = stack.pop() {
        let n = repo.tree_store.entry_count(frame.tree_id);
        for j in 0..n {
            let TreeEntry { mode, hash, name } = repo.tree_store.get_entry(frame.tree_id, j);

            if mode == MODE_DIR {
                let obj = repo.read_object(&hash)?;
                let sub_id = obj.try_as_tree_id()?;
                let path = if frame.prefix.is_empty() {
                    name
                } else {
                    format!("{}/{}", frame.prefix, name).into()
                };
                stack.push(Frame {
                    tree_id: sub_id,
                    prefix: path,
                });
            } else {
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
    }
    path_offsets.push(path_blob.len() as u32);

    let n = hashes.len();
    let mut sorted_order: Vec<usize> = (0..n).collect();
    sorted_order.sort_by(|&a, &b| {
        let sa = head_tree_path_at(&path_blob, &path_offsets, a);
        let sb = head_tree_path_at(&path_blob, &path_offsets, b);
        sa.cmp(sb)
    });

    Ok(HeadTreeFlat {
        path_blob,
        path_offsets,
        hashes,
        sorted_order,
    })
}

#[inline]
fn head_tree_path_at<'a>(path_blob: &'a [u8], path_offsets: &[u32], i: usize) -> &'a [u8] {
    let start = path_offsets[i] as usize;
    let end = path_offsets[i + 1] as usize;
    &path_blob[start..end]
}

// --- Status buckets: one Vec per category (DOD: parallel to path strings) ---

pub struct StatusBuckets {
    /// Staged: in index, (new or index.hash != head hash).
    pub staged_new_modified: Vec<String>,
    /// Staged delete: in HEAD, not in index.
    pub staged_deleted: Vec<String>,
    /// In index, file on disk exists but content differs (mtime/size).
    pub modified: Vec<String>,
    /// In index, file missing on disk.
    pub deleted: Vec<String>,
    /// Not in index, file on disk (under repo, not .vx).
    pub untracked: Vec<String>,
}

/// Single pass: scan index (stat + compare to head), then walk dir for untracked.
fn collect_status(
    index: &Index,
    head: &HeadTreeFlat,
    repo_root: &Path,
    ignore: &Ignore,
) -> StatusBuckets {
    let mut staged_new_modified = Vec::new();
    let mut staged_deleted = Vec::new();
    let mut modified = Vec::new();
    let mut deleted = Vec::new();

    for i in 0..index.count {
        let path_str = index.get_path(i);
        let rel_path = Path::new(path_str);
        let abs = repo_root.join(rel_path);

        let head_hash = head.lookup(path_str);
        let index_hash = index.hashes[i];

        let staged = head_hash != Some(index_hash);
        if staged {
            staged_new_modified.push(path_str.to_string());
        }

        match fs::metadata(&abs) {
            Ok(meta) => {
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs() as i64);

                let size = meta.len();
                if index.mtimes[i] != mtime || index.sizes[i] != size {
                    if !staged {
                        // could be staged and modified; we already added to staged above
                    }
                    modified.push(path_str.to_string());
                }
            }
            Err(_) => {
                deleted.push(path_str.to_string());
            }
        }
    }

    for j in 0..head.len() {
        let path_str = head.get_path(head.sorted_order[j]);
        if index.find(Path::new(path_str)).is_none() {
            staged_deleted.push(path_str.to_string());
        }
    }

    let mut untracked = Vec::new();
    for entry in WalkDir::new(repo_root)
        .into_iter()
        .filter_entry(|e| !ignore.is_ignored_abs(e.path()))
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let Ok(rel) = path.strip_prefix(repo_root) else { continue };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if rel_str.is_empty() {
            continue;
        }
        if ignore.is_ignored_rel(&rel_str) {
            continue;
        }
        if index.find(Path::new(&rel_str)).is_none() {
            untracked.push(rel_str);
        }
    }
    untracked.sort();
    staged_new_modified.sort();
    staged_deleted.sort();
    modified.sort();
    deleted.sort();

    StatusBuckets {
        staged_new_modified,
        staged_deleted,
        modified,
        deleted,
        untracked,
    }
}

// --- Output: sections, optional color ---

fn stdout_is_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

const GREEN:  &str = "\x1b[32m";
const RED:    &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const BOLD:  &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

fn section_header(f: &mut (impl std::io::Write + ?Sized), color: &str, title: &str) -> std::io::Result<()> {
    if stdout_is_tty() {
        writeln!(f, "  {}{}{}", color, title, RESET)?;
    } else {
        writeln!(f, "  {}", title)?;
    }
    Ok(())
}

fn path_line(f: &mut (impl std::io::Write + ?Sized), color: &str, path: &str) -> std::io::Result<()> {
    if stdout_is_tty() {
        writeln!(f, "    {}{}{}", color, path, RESET)?;
    } else {
        writeln!(f, "    {}", path)?;
    }
    Ok(())
}

pub fn print_status(buckets: &StatusBuckets, out: &mut (impl std::io::Write + ?Sized)) -> std::io::Result<()> {
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
                writeln!(out, "    {}... and {} more untracked{}\n", YELLOW, rest, RESET)?;
            } else {
                writeln!(out, "    ... and {} more untracked\n", rest)?;
            }
        }
    }

    Ok(())
}

pub fn status(repo: &mut Repository) -> Result<()> {
    let index = Index::load(&repo.root)?;
    let head_commit = repo.read_head_commit().ok();
    let head_tree = head_commit
        .and_then(|hash| repo.read_object(&hash).ok())
        .and_then(|obj| obj.try_as_commit_id().ok())
        .map(|id| repo.commit_store.get_tree(id));
    let head_flat = match head_tree {
        Some(tree_hash) => flatten_head_tree(repo, tree_hash)?,
        None => HeadTreeFlat {
            path_blob: Vec::new(),
            path_offsets: vec![0],
            hashes: Vec::new(),
            sorted_order: Vec::new(),
        },
    };
    let buckets = collect_status(&index, &head_flat, &repo.root, &repo.ignore);
    print_status(&buckets, &mut std::io::stdout())?;
    Ok(())
}
