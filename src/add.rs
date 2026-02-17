use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::fs;
use anyhow::Result;
use walkdir::WalkDir;
use crate::repository::Repository;
use crate::index::Index;
use crate::object::{Object, Blob};

pub fn add(repo: &Repository, paths: &[PathBuf]) -> Result<()> {
    let current_dir = std::env::current_dir()?;

    let mut index = Index::load(&repo.root)?;
    let mut count = 0;

    // No args -> add everything
    let default = vec![PathBuf::from(".")];
    let paths = if paths.is_empty() { &default } else { paths };

    for path in paths {
        let full = if path.is_absolute() {
            Cow::Borrowed(path)
        } else {
            Cow::Owned(current_dir.join(path))
        };

        //
        // Canonicalize to resolve "." and ".."
        //
        let full = full.canonicalize()?;

        if full.is_file() {
            add_file(repo, &mut index, &full)?;
            count += 1;

            continue;
        }

        if full.is_dir() {
            for entry in WalkDir::new(&full)
                .into_iter()
                .filter_entry(|e| {
                    !e.path().starts_with(repo.root.join(".vx"))
                })
            {
                let entry = entry?;
                if entry.file_type().is_file() {
                    add_file(repo, &mut index, entry.path())?;
                    count += 1;
                }
            }

            continue;
        }

        eprintln!("warning: '{}' did not match any files", path.display());
    }

    index.save(&repo.root)?;
    println!("added {count} file(s) to index");
    Ok(())
}

fn add_file(repo: &Repository, index: &mut Index, abs_path: &Path) -> Result<()> {
    let rel = abs_path.strip_prefix(&repo.root)?;
    let rel_str = rel.to_str().expect("non-utf8 path");
    //
    // Normalise to forward slashes
    //
    let rel_normalized = PathBuf::from(rel_str.replace('\\', "/"));

    let metadata = fs::metadata(abs_path)?;

    // Fast path: if mtime + size match, skip hashing and blob write entirely
    if let Some(i) = index.find(&rel_normalized) {
        if !index.is_dirty(i, &metadata) {
            return Ok(());
        }
    }

    let data     = fs::read(abs_path)?;

    //
    // Write blob to object store
    //
    let hash = repo.storage.write(&Object::Blob(Blob {
        data: crate::util::vec_into_boxed_slice_noshrink(data)
    }))?;

    index.add(&rel_normalized, hash, &metadata);
    Ok(())
}
