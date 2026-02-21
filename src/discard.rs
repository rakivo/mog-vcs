use crate::{index::Index, repository::Repository};

use std::path::{Path, PathBuf};

use anyhow::Result;
use walkdir::WalkDir;
use rayon::prelude::*;

pub fn discard(repo: &mut Repository, patterns: &[PathBuf]) -> Result<()> {
    let index = Index::load(&repo.root)?;

    if patterns.is_empty() {
        return discard_all(repo, &index);
    }

    let current_dir = std::env::current_dir()?;
    let (literal_roots, combined_re) = crate::stage::classify_patterns(patterns, &current_dir);
    let matched = crate::stage::walk_matching(&repo.root, &repo.ignore, &literal_roots, combined_re.as_ref());

    let mut restored = 0usize;
    for (_abs, rel_str) in matched {
        let Some(i) = index.find(&rel_str) else {
            eprintln!("not in index, skipping: {rel_str}");
            continue;
        };

        let hash = index.hashes[i];
        let abs = repo.root.join(rel_str.as_ref());
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent)?;
        }

        {
            let raw = repo.storage.read(&hash)?;
            let data = crate::object::decode_blob_bytes(raw)?;
            std::fs::write(&abs, data)?;
            repo.storage.evict_pages(raw);
        }

        restored += 1;
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
            let raw = repo.storage.read(&hash)?;
            let data = crate::object::decode_blob_bytes(raw)?.into();
            repo.storage.evict_pages(raw);
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
