#![allow(unused_imports)]

use std::{borrow::Cow, path::Path};

use crate::{hash::{hash_to_hex, hex_to_hash}, index::Index, object::{Blob, Commit, Object, Tree}, repository::Repository};

use anyhow::Result;

#[inline]
pub fn checkout_blob_to(repo: &Repository, blob: &Blob, to: &str) -> Result<()> {
    let path = repo.root.join(to);

    // Create parent directories if they don't exist
    // e.g. "src/foo/bar.rs" needs "src/foo/" to exist first
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    _ = std::fs::write(&path, &blob.data);

    Ok(())
}

#[inline]
pub fn checkout_commit(repo: &Repository, commit: &Commit) -> Result<()> {
    let object = repo.storage.read(&commit.tree)?;
    let tree = object.try_as_tree()?;
    checkout_tree(repo, &tree)
}

#[inline]
pub fn checkout_tree(repo: &Repository, tree: &Tree) -> Result<()> {
    checkout_tree_impl(repo, tree, None)
}

pub fn checkout_tree_impl(repo: &Repository, tree: &Tree, tree_path: Option<&str>) -> Result<()> {
    for entry in tree {
        let object = repo.storage.read(entry.hash)?;

        let to = entry.name;
        let full_path = if let Some(tree_path) = tree_path {
            Cow::Owned(format!("{tree_path}/{to}"))
        } else {
            Cow::Borrowed(to)
        };

        match object {
            Object::Blob(blob) => checkout_blob_to(repo, &blob, &full_path)?,
            Object::Tree(tree) => checkout_tree_impl(repo, &tree, Some(&full_path))?,
            Object::Commit(_) => {} // submodule, ignore for now
        }
    }

    Ok(())
}

pub fn checkout(repo: &Repository, branch: &str) -> Result<()> {
    let branch_ref = format!("refs/heads/{branch}");
    let branch_path = repo.root.join(".vx").join(&branch_ref);

    if branch_path.exists() {
        //
        // It's a branch - normal checkout
        //

        let hash = repo.read_ref(&branch_ref)?;
        let commit = repo.storage.read(&hash)?.try_into_commit()?;
        let tree = repo.storage.read(&commit.tree)?.try_into_tree()?;

        std::fs::write(
            repo.root.join(".vx/HEAD"),
            format!("ref: {branch_ref}\n")
        )?;

        println!("Switched to branch '{branch}'");

        return checkout_tree(repo, &tree);
    }

    //
    // Try as commit hash - detached HEAD
    //
    let hash = hex_to_hash(branch)?;
    let object = repo.storage.read(&hash)?;
    checkout_commit(repo, object.try_as_commit()?)?;

    // HEAD points directly to commit
    std::fs::write(
        repo.root.join(".vx/HEAD"),
        format!("{branch}\n")
    )?;

    println!("HEAD is now at {} (detached)", &branch[..8]);
    println!("You are in detached HEAD state.");
    println!("If you commit, create a branch to keep your work:");
    println!("  vx branch save-my-work");

    Ok(())
}

/// Checkout a specific file or directory from a commit/branch,
/// then update the index to reflect the restored state.
pub fn checkout_path(repo: &Repository, target: &str, path: &str) -> Result<()> {
    let commit = repo.resolve_to_commit(target)?;
    let (obj, obj_hash) = repo.walk_tree_path(&commit.tree, path)?;

    let mut index = Index::load(&repo.root)?;

    match obj {
        Object::Blob(ref blob) => {
            checkout_blob_to(repo, blob, path)?;

            //
            // Update index entry
            //
            let abs = repo.root.join(path);
            let metadata = std::fs::metadata(&abs)?;
            index.add(path.as_ref(), obj_hash, &metadata);
            index.save(&repo.root)?;

            println!("restored '{path}'");
        }

        Object::Tree(ref tree) => {
            //
            // Write all files under the directory
            //
            checkout_tree_impl(repo, tree, Some(path))?;

            //
            // Update index for every file we just wrote
            //
            index.update_from_tree_recursive(repo, tree, path)?;
            index.save(&repo.root)?;

            println!("restored '{path}/'");
        }

        Object::Commit(_) => anyhow::bail!("unexpected commit object at '{path}'"),
    }

    Ok(())
}
