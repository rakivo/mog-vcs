#![allow(unused_imports)]

use std::borrow::Cow;

use crate::{hash::hash_to_hex, object::{Blob, Object, Tree}, repository::Repository};

use anyhow::Result;

pub fn checkout_blob_to(repo: &Repository, blob: &Blob, to: &str) -> Result<()> {
    if let Ok(s) = std::str::from_utf8(&blob.data) {
        let s = if s.len() < 50 {
            s
        } else {
            let b = s.char_indices().skip(50-1).next().unwrap().0;
            &s[..b]
        };
        println!("writing {s:?} to {to}");
    }

    std::fs::write(to, &blob.data)?;

    Ok(())
}

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
            Object::Blob(blob) => {
                checkout_blob_to(repo, &blob, &full_path)?;
            }

            Object::Tree(tree) => {
                checkout_tree_impl(repo, &tree, Some(&full_path))?
            }

            Object::Commit(_) => {}
        }
    }

    Ok(())
}

pub fn checkout(repo: &Repository, branch: &str) -> Result<()> {
    let branch_ref = format!("refs/heads/{branch}");

    let hash = repo.read_ref(&branch_ref)?;
    let commit = repo.storage.read(&hash)?.try_into_commit()?;
    let tree = repo.storage.read(&commit.tree)?.try_into_tree()?;

    checkout_tree(repo, &tree)
}
