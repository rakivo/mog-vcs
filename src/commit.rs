use anyhow::Result;
use std::time::{SystemTime, UNIX_EPOCH};
use std::fs;
use crate::repository::Repository;
use crate::object::{Object, Commit};
use crate::hash::{Hash, hash_to_hex};

pub fn commit(
    repo: &Repository,
    tree: Hash,
    parent: Option<Hash>,
    author: &str,
    message: &str,
) -> Result<Hash> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_secs() as i64;

    let parents = if let Some(p) = parent {
        vec![p]
    } else {
        vec![]
    };

    let commit = Commit {
        tree,
        parents,
        timestamp,
        author: author.to_string(),
        message: message.to_string(),
    };

    let hash = repo.storage.write(&Object::Commit(commit))?;

    // Read HEAD to figure out where to write
    let head = fs::read_to_string(repo.root.join(".vx/HEAD"))?;
    let head = head.trim();

    if let Some(refpath) = head.strip_prefix("ref: ") {
        // Normal: update the branch HEAD points to
        repo.write_ref(refpath.trim(), &hash)?;
    } else {
        // Detached HEAD: update HEAD directly to new commit
        fs::write(
            repo.root.join(".vx/HEAD"),
            format!("{}\n", hash_to_hex(&hash))
        )?;
        println!("Warning: committing in detached HEAD state");
        println!("Create a branch to keep this work: vx branch save-my-work");
    }

    println!("Created commit {}", hash_to_hex(&hash));
    Ok(hash)
}
