use anyhow::Result;
use std::time::{SystemTime, UNIX_EPOCH};
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
    
    // Update refs/heads/main
    repo.write_ref("refs/heads/main", &hash)?;
    
    println!("Created commit {}", hash_to_hex(&hash));
    
    Ok(hash)
}
