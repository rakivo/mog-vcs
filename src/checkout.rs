use crate::hash::{hash_to_hex, hex_to_hash, Hash};
use crate::index::Index;
use crate::repository::Repository;
use crate::object::{Object, MODE_DIR};
use crate::storage::Storage;
use crate::store::{BlobId, CommitId};
use crate::tree::TreeEntry;

use anyhow::Result;

pub fn checkout(repo: &mut Repository, branch: &str) -> Result<()> {
    let branch_ref = format!("refs/heads/{branch}");
    let branch_path = repo.root.join(".mog").join(&branch_ref);

    if branch_path.exists() {
        let hash = repo.read_ref(&branch_ref)?;
        let object = repo.read_object(&hash)?;
        let commit_id = object.try_as_commit_id()?;

        std::fs::write(
            repo.root.join(".mog/HEAD"),
            format!("ref: {branch_ref}\n"),
        )?;

        println!("Switched to branch '{branch}'");
        return checkout_commit(repo, commit_id);
    }

    let hash = hex_to_hash(branch)?;
    let object = repo.read_object(&hash)?;
    checkout_commit(repo, object.try_as_commit_id()?)?;

    std::fs::write(
        repo.root.join(".mog/HEAD"),
        format!("{hash}\n", hash = hash_to_hex(&hash)),
    )?;

    println!("HEAD is now at {} (detached)", &hash_to_hex(&hash)[..8]);
    println!("You are in detached HEAD state.");
    println!("If you commit, create a branch to keep your work:");
    println!("  mog branch save-my-work");

    Ok(())
}

pub fn checkout_path(repo: &mut Repository, target: &str, path: &str) -> Result<()> {
    let (_commit_hash, commit_id) = repo.resolve_to_commit(target)?;
    let tree_hash = repo.commit.get_tree(commit_id);
    let (object, obj_hash) = repo.walk_tree_path(&tree_hash, path)?;
    let mut index = Index::load(&repo.root)?;

    match object {
        Object::Blob(blob_id) => {
            checkout_blob_to(repo, blob_id, path)?;
            let abs = repo.root.join(path);
            let metadata = std::fs::metadata(&abs)?;
            index.add(path, obj_hash, &metadata);
            index.save(&repo.root)?;
            println!("restored '{path}'");
        }
        Object::Tree(tree_id) => {
            checkout_tree_impl(repo, tree_hash, path)?;
            index.update_from_tree_recursive(repo, tree_id, path)?;
            index.save(&repo.root)?;
            println!("restored '{path}/'");
        }
        Object::Commit(_) => anyhow::bail!("unexpected commit object at '{path}'"),
    }

    Ok(())
}

#[inline]
pub fn checkout_blob_to(repo: &Repository, blob_id: BlobId, to: &str) -> Result<()> {
    let path = repo.root.join(to);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data = repo.blob.get(blob_id);
    std::fs::write(&path, data)?;
    Ok(())
}

#[inline]
pub fn checkout_commit(repo: &mut Repository, commit_id: CommitId) -> Result<()> {
    let tree_hash = repo.commit.get_tree(commit_id);
    checkout_tree(repo, tree_hash)
}

#[inline]
pub fn checkout_tree(repo: &mut Repository, tree_hash: Hash) -> Result<()> {
    checkout_tree_impl(repo, tree_hash, "")
}

pub fn checkout_tree_impl(
    repo: &mut Repository,
    tree_hash: Hash,
    prefix: &str,
) -> Result<()> {
    // Flatten the target tree to know which paths should exist.
    let target_flat = crate::status::flatten_tree(repo, tree_hash)?;

    //
    // Delete tracked files not present in the target tree.
    //

    let index = crate::index::Index::load(&repo.root)?;
    for i in 0..index.count {
        let path_str = index.get_path(i);
        if target_flat.lookup(path_str).is_none() {
            let abs = repo.root.join(path_str);
            let _ = std::fs::remove_file(&abs);
        }
    }
    crate::discard::remove_empty_dirs(&repo.root)?;

    // Then rebuild index from target tree.
    let mut new_index = crate::index::Index::default();

    struct Frame {
        tree_hash: Hash,
        prefix: Box<str>,
    }

    let mut stack = vec![Frame { tree_hash, prefix: prefix.into() }];
    while let Some(Frame { tree_hash, prefix: frame_prefix }) = stack.pop() {
        let entries = {
            let raw = repo.storage.read(&tree_hash)?;
            let entries = crate::object::decode_tree_entries(raw)?;
            Storage::evict_pages(raw);
            entries
        };

        for TreeEntry { mode, hash, name } in entries {
            let child_path = if frame_prefix.is_empty() {
                name
            } else {
                format!("{frame_prefix}/{name}").into()
            };

            if mode == MODE_DIR {
                //
                // Tree: read into store and recurse.
                //
                std::fs::create_dir_all(repo.root.join(child_path.as_ref()))?;
                stack.push(Frame { tree_hash: hash, prefix: child_path });
            } else {
                //
                // Blob: read raw bytes directly, bypassing the blob store entirely.
                //
                let path = repo.root.join(child_path.as_ref());
                _ = repo.with_blob_bytes_without_touching_cache_and_evict_the_pages(
                    &hash,
                    |_repo, data| std::fs::write(&path, data)
                )?;

                let meta = std::fs::metadata(&path)?;
                new_index.add(&child_path, hash, &meta);
            }
        }
    }

    new_index.save(&repo.root)?;

    Ok(())
}
