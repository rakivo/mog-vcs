use crate::{index::Index, repository::Repository, stage::{classify_patterns, walk_matching}, status::SortedFlatTree};

use std::path::{Path, PathBuf};

use anyhow::Result;
use walkdir::WalkDir;
use rayon::prelude::*;

pub fn discard(repo: &mut Repository, patterns: &[PathBuf]) -> Result<()> {
    let index = Index::load(&repo.root)?;
    if patterns.is_empty() {
        return discard_all(repo, &index);
    }

    //
    // Build HEAD flat tree so we know what exists in HEAD.
    //
    let head_flat = match repo.read_head_commit().ok() {
        Some(head_hash) => {
            let obj       = repo.read_object(&head_hash)?;
            let commit_id = obj.try_as_commit_id()?;
            let tree_hash = repo.commit.get_tree(commit_id);
            crate::status::flatten_tree(repo, tree_hash)?
        }
        None => SortedFlatTree::default()
    };

    let current_dir = &repo.root;
    let (literal_roots, combined_re) = classify_patterns(patterns, &current_dir);
    let matched = walk_matching(current_dir, &repo.ignore, &literal_roots, combined_re.as_ref());

    let mut restored = 0usize;
    for (_abs, rel_str) in matched {
        let abs = repo.root.join(rel_str.as_ref());
        match head_flat.lookup(&rel_str) {
            Some(head_hash) => {
                //
                // In HEAD: restore to HEAD version.
                //
                if let Some(parent) = abs.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                repo.with_blob_bytes_without_touching_cache_and_evict_the_pages(
                    &head_hash,
                    |_repo, data| std::fs::write(&abs, data)
                )?;
                restored += 1;
            }

            None => match index.find(&rel_str) {
                Some(i) if head_flat.is_empty() => {
                    // No commits yet, index is the source of truth, restore from it.
                    let hash = index.hashes[i];
                    repo.with_blob_bytes_without_touching_cache_and_evict_the_pages(
                        &hash,
                        |_repo, data| std::fs::write(&abs, data)
                    )?;
                    restored += 1;
                }
                _ => {
                    // In HEAD but not committed (or no index entry), delete it.
                    _ = std::fs::remove_file(&abs);
                    let mut index = Index::load(&repo.root)?;
                    index.remove(&rel_str);
                    index.save(&repo.root)?;
                    restored += 1;
                }
            }
        }
    }

    println!("Discarded changes in {restored} file(s)");

    Ok(())
}

fn discard_all(repo: &mut Repository, index: &Index) -> Result<()> {
    //
    // Delete untracked files.
    //
    let mut to_delete = Vec::<Box<Path>>::new();
    for entry in WalkDir::new(&repo.root)
        .into_iter()
        .filter_entry(|e| !repo.ignore.is_ignored_abs(e.path()))
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() { continue; }

        let path = entry.path();
        let Ok(rel) = path.strip_prefix(&repo.root) else { continue };

        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if repo.ignore.is_ignored_rel(&rel_str) { continue; }

        if index.find(&rel_str).is_none() {
            to_delete.push(path.into());
        }
    }

    for path in &to_delete {
        std::fs::remove_file(path)?;
    }
    remove_empty_dirs(&repo.root)?;

    //
    // Read blobs sequentially, evict pages as we go.
    //
    let mut blobs: Vec<(Box<[u8]>, Box<Path>)> = Vec::with_capacity(index.count);
    for i in 0..index.count {
        let hash = index.hashes[i];
        let abs  = repo.root.join(index.get_path(i)).into_boxed_path();
        {
            let data = repo.with_blob_bytes_without_touching_cache_and_evict_the_pages(
                &hash,
                |_repo, data| anyhow::Ok(data.into())
            )?;

            blobs.push((data, abs));
        }
    }

    //
    // Write to disk in parallel.
    //
    blobs.par_iter().try_for_each(|(data, abs)| -> Result<()> {
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(abs, data)?;
        Ok(())
    })?;

    println!("Discarded all changes, restored {} file(s)", index.count);
    Ok(())
}

pub fn remove_empty_dirs(root: &Path) -> Result<()> {
    for entry in std::fs::read_dir(root)?.filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_dir() { continue }

        if path.ends_with(".mog") { continue; }

        remove_empty_dirs(&path)?;
        _ = std::fs::remove_dir(&path);
    }

    Ok(())
}
