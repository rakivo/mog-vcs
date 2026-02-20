use crate::hash::{hash_to_hex, hex_to_hash};
use crate::index::Index;
use crate::repository::Repository;
use crate::object::Object;
use crate::store::{BlobId, CommitId, TreeId};
use crate::tree::TreeEntry;

use anyhow::Result;

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
    let obj = repo.read_object(&tree_hash)?;
    let tree_id = obj.try_as_tree_id()?;
    checkout_tree(repo, tree_id)
}

#[inline]
pub fn checkout_tree(repo: &mut Repository, tree_id: TreeId) -> Result<()> {
    checkout_tree_impl(repo, tree_id, None)
}

pub fn checkout_tree_impl(
    repo: &mut Repository,
    tree_id: TreeId,
    tree_path: Option<&str>,
) -> Result<()> {
    let n = repo.tree.entry_count(tree_id);
    for j in 0..n {
        let TreeEntry { hash, name, .. } = repo.tree.get_entry(tree_id, j);
        let obj = repo.read_object(&hash)?;

        let full_path = if let Some(tp) = tree_path {
            format!("{tp}/{name}").into()
        } else {
            name
        };

        match obj {
            Object::Blob(blob_id) => checkout_blob_to(repo, blob_id, &full_path)?,
            Object::Tree(sub_id) => checkout_tree_impl(repo, sub_id, Some(&full_path))?,
            Object::Commit(_) => {}
        }
    }
    Ok(())
}

pub fn checkout(repo: &mut Repository, branch: &str) -> Result<()> {
    let branch_ref = format!("refs/heads/{branch}");
    let branch_path = repo.root.join(".mog").join(&branch_ref);

    if branch_path.exists() {
        let hash = repo.read_ref(&branch_ref)?;
        let obj = repo.read_object(&hash)?;
        let commit_id = obj.try_as_commit_id()?;

        std::fs::write(
            repo.root.join(".mog/HEAD"),
            format!("ref: {branch_ref}\n"),
        )?;

        println!("Switched to branch '{branch}'");
        return checkout_commit(repo, commit_id);
    }

    let hash = hex_to_hash(branch)?;
    let obj = repo.read_object(&hash)?;
    checkout_commit(repo, obj.try_as_commit_id()?)?;

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
    let (obj, obj_hash) = repo.walk_tree_path(&tree_hash, path)?;

    let mut index = Index::load(&repo.root)?;

    match obj {
        Object::Blob(blob_id) => {
            checkout_blob_to(repo, blob_id, path)?;
            let abs = repo.root.join(path);
            let metadata = std::fs::metadata(&abs)?;
            index.add(path, obj_hash, &metadata);
            index.save(&repo.root)?;
            println!("restored '{path}'");
        }
        Object::Tree(tree_id) => {
            checkout_tree_impl(repo, tree_id, Some(path))?;
            index.update_from_tree_recursive(repo, tree_id, path)?;
            index.save(&repo.root)?;
            println!("restored '{path}/'");
        }
        Object::Commit(_) => anyhow::bail!("unexpected commit object at '{path}'"),
    }

    Ok(())
}
