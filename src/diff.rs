use crate::hash::{hash_bytes, hex_to_hash};
use crate::index::Index;
use crate::repository::Repository;
use crate::status::SortedFlatTree;

use std::io::Write as _;
use std::io::BufWriter;

use anyhow::Result;
use imara_diff::{Algorithm, BasicLineDiffPrinter, Diff, InternedInput, UnifiedDiffConfig};

pub enum DiffTarget<'a> {
    /// `mog diff` - working directory vs index
    WorkingVsIndex,
    /// `mog diff --staged` - index vs HEAD
    Staged,
    /// `mog diff <branch>` - working directory vs branch tip
    Branch(&'a str),
    /// `mog diff <hex>` - working directory vs commit
    Commit(&'a str),
}

#[inline]
pub fn diff(repo: &mut Repository, target: DiffTarget<'_>) -> Result<()> {
    match target {
        DiffTarget::WorkingVsIndex => diff_working_vs_index(repo),
        DiffTarget::Staged         => diff_staged(repo),
        DiffTarget::Branch(name)   => {
            let flat = resolve_to_flat_tree(repo, name)?;
            diff_working_vs_tree(repo, flat)
        }
        DiffTarget::Commit(hex) => {
            let flat = resolve_commit_to_flat_tree(repo, hex)?;
            diff_working_vs_tree(repo, flat)
        }
    }
}

//
// @Note: To be consistent these diff functions should take in a `&mut dyn Write` param instead of creating a local `BufWriter`.
//

//
//
// Diff implementations
//
//

fn diff_working_vs_index(repo: &mut Repository) -> Result<()> {
    let index = Index::load(&repo.root)?;

    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    for entry in &index {
        if repo.ignore.is_ignored_rel(entry.path) {
            continue;
        }

        let Ok(on_disk) = std::fs::read(repo.root.join(entry.path)) else {
            continue;
        };
        if hash_bytes(&on_disk) == *entry.hash {
            continue; // Unchanged!
        }

        let Ok(after) = std::str::from_utf8(&on_disk) else {
            writeln!(out, "Binary files differ: {}", entry.path)?;
            continue;
        };

        let Ok(before_bytes) = repo.read_blob_bytes_without_touching_cache(&entry.hash) else {
            continue;
        };
        let Ok(before) = std::str::from_utf8(&before_bytes) else {
            writeln!(out, "Binary files differ: {}", entry.path)?;
            continue;
        };

        print_diff(before, after, entry.path, &mut out)?;
    }

    Ok(())
}

fn diff_staged(repo: &mut Repository) -> Result<()> {
    let index = Index::load(&repo.root)?;

    let head_flat = match resolve_head_to_flat_tree(repo) {
        Ok(flat) => flat,
        Err(_)   => SortedFlatTree::default(), // No commits yet, empty tree!
    };

    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    for entry in &index {
        if repo.ignore.is_ignored_rel(entry.path) {
            continue;
        }

        match head_flat.lookup(entry.path) {
            Some(head_hash) => {
                if head_hash == *entry.hash {
                    continue; // Unchanged!
                }

                let Ok(before_bytes) = repo.read_blob_bytes_without_touching_cache(&head_hash) else {
                    continue;
                };
                let Ok(before) = std::str::from_utf8(&before_bytes) else {
                    writeln!(out, "Binary files differ: {}", entry.path)?;
                    continue;
                };

                let Ok(after_bytes) = repo.read_blob_bytes_without_touching_cache(&entry.hash) else {
                    continue;
                };
                let Ok(after) = std::str::from_utf8(&after_bytes) else {
                    writeln!(out, "Binary files differ: {}", entry.path)?;
                    continue;
                };

                print_diff(before, after, entry.path, &mut out)?;
            }
            None => {
                // New file - didn't exist in HEAD.
                let Ok(after_bytes) = repo.read_blob_bytes_without_touching_cache(&entry.hash) else {
                    continue;
                };
                let Ok(after) = std::str::from_utf8(&after_bytes) else {
                    writeln!(out, "Binary files differ: {}", entry.path)?;
                    continue;
                };

                print_diff("", after, entry.path, &mut out)?;
            }
        }
    }

    Ok(())
}

fn diff_working_vs_tree(repo: &mut Repository, flat: SortedFlatTree) -> Result<()> {
    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    for i in 0..flat.len() {
        let path = flat.get_path(i);

        if repo.ignore.is_ignored_rel(path) {
            continue;
        }

        let blob_hash = flat.hashes[i];

        let Ok(on_disk) = std::fs::read(repo.root.join(path)) else {
            //
            // File deleted locally vs target - show as pure removal.
            //

            let Ok(before_bytes) = repo.read_blob_bytes_without_touching_cache(&blob_hash) else {
                continue;
            };
            let Ok(before) = std::str::from_utf8(&before_bytes) else {
                writeln!(out, "Binary files differ: {path}")?;
                continue;
            };
            print_diff(before, "", path, &mut out)?;
            continue;
        };

        if hash_bytes(&on_disk) == blob_hash {
            continue; // Unchanged!
        }

        let Ok(after) = std::str::from_utf8(&on_disk) else {
            writeln!(out, "Binary files differ: {path}")?;
            continue;
        };

        let Ok(before_bytes) = repo.read_blob_bytes_without_touching_cache(&blob_hash) else {
            continue;
        };
        let Ok(before) = std::str::from_utf8(&before_bytes) else {
            writeln!(out, "Binary files differ: {path}")?;
            continue;
        };

        print_diff(before, after, path, &mut out)?;
    }

    //
    //
    // Files in index but not in target tree - show as pure additions.
    //
    //

    let index = Index::load(&repo.root)?;
    for entry in &index {
        if repo.ignore.is_ignored_rel(entry.path) || flat.lookup(entry.path).is_some() {
            continue;
        }

        let Ok(on_disk) = std::fs::read(repo.root.join(entry.path)) else {
            continue;
        };
        let Ok(after) = std::str::from_utf8(&on_disk) else {
            writeln!(out, "Binary files differ: {}", entry.path)?;
            continue;
        };

        print_diff("", after, entry.path, &mut out)?;
    }

    Ok(())
}

//
//
// Resolve helpers
//
//

#[inline]
fn resolve_to_flat_tree(repo: &mut Repository, branch: &str) -> Result<SortedFlatTree> {
    let branch_ref = format!("refs/heads/{branch}");
    let hash = repo.read_ref(&branch_ref)?;
    let obj = repo.read_object(&hash)?;
    let commit_id = obj.try_as_commit_id()?;
    let tree_hash = repo.commit.get_tree(commit_id);
    crate::status::flatten_tree(repo, tree_hash)
}

#[inline]
fn resolve_commit_to_flat_tree(repo: &mut Repository, commit_hex: &str) -> Result<SortedFlatTree> {
    let hash = hex_to_hash(commit_hex)?;
    let obj = repo.read_object(&hash)?;
    let commit_id = obj.try_as_commit_id()?;
    let tree_hash = repo.commit.get_tree(commit_id);
    crate::status::flatten_tree(repo, tree_hash)
}

#[inline]
fn resolve_head_to_flat_tree(repo: &mut Repository) -> Result<SortedFlatTree> {
    let head_hash = repo.read_head_commit()?;
    let obj = repo.read_object(&head_hash)?;
    let commit_id = obj.try_as_commit_id()?;
    let tree_hash = repo.commit.get_tree(commit_id);
    crate::status::flatten_tree(repo, tree_hash)
}

#[inline]
fn print_diff(
    before: &str,
    after: &str,
    path: &str,
    out: &mut BufWriter<impl std::io::Write>,
) -> Result<()> {
    let input = InternedInput::new(before, after);
    let mut diff = Diff::compute(Algorithm::Histogram, &input);
    diff.postprocess_lines(&input);

    if diff.hunks().next().is_none() {
        return Ok(()); // Empty diff!
    }

    let printer = BasicLineDiffPrinter(&input.interner);
    let unified = diff.unified_diff(&printer, UnifiedDiffConfig::default(), &input);

    writeln!(out, "--- a/{path}")?;
    writeln!(out, "+++ b/{path}")?;
    writeln!(out, "{unified}")?;

    Ok(())
}
