use crate::{
    hash::{hash_to_hex, hex_to_hash},
    repository::Repository,
    util::Xxh3HashSet,
};

use std::path::PathBuf;

use anyhow::{Result, bail};

#[inline]
fn branch_path(repo: &Repository, name: &str) -> PathBuf {
    repo.root.join(".mog/refs/heads").join(name)
}

#[inline]
fn branch_exists(repo: &Repository, name: &str) -> bool {
    branch_path(repo, name).exists()
}

/// Print all local branches, marking the current one with *.
pub fn list(repo: &Repository) -> Result<()> {
    let heads_dir = repo.root.join(".mog/refs/heads");
    if !heads_dir.exists() {
        println!("no branches yet");
        return Ok(());
    }

    let current = repo.current_branch().unwrap_or(None);

    let mut branches = std::fs::read_dir(&heads_dir)?
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .collect::<Vec<_>>();

    branches.sort_unstable();

    for branch in branches {
        let marker = if current.as_deref() == Some(&branch) { "* " } else { "  " };
        let hash = repo.read_ref(&format!("refs/heads/{branch}")).map_or_else(
            |_| "?".to_string(),
            |h| hash_to_hex(&h)[..8].to_string()
        );

        println!("{marker}{branch}  {hash}");
    }

    Ok(())
}

/// Create a new branch pointing at `target` (branch name, commit hash, or HEAD).
pub fn create(repo: &mut Repository, name: &str, target: Option<&str>) -> Result<()> {
    if branch_exists(repo, name) {
        bail!("branch '{name}' already exists");
    }

    validate_branch_name(name)?;

    let hash = match target {
        Some(t) => {
            let branch_ref = format!("refs/heads/{t}");
            let branch_path = repo.root.join(".mog").join(&branch_ref);
            if branch_path.exists() {
                repo.read_ref(&branch_ref)?
            } else {
                hex_to_hash(t)?
            }
        }
        None => repo.read_head_commit()?,
    };

    let object = repo.read_object(&hash)?;
    object.try_as_commit_id().map_err(|_| anyhow::anyhow!("target does not resolve to a commit"))?;

    repo.write_ref(&format!("refs/heads/{name}"), &hash)?;
    println!("created branch '{name}' at {}", &hash_to_hex(&hash)[..8]);

    Ok(())
}

/// Safe delete - refuses if the branch has commits not reachable from any other branch.
pub fn delete(repo: &mut Repository, name: &str) -> Result<()> {
    if !branch_exists(repo, name) {
        bail!("branch '{name}' not found");
    }

    if repo.current_branch()?.as_deref() == Some(name) {
        bail!("cannot delete branch '{name}': it is currently checked out");
    }

    let branch_hash = repo.read_ref(&format!("refs/heads/{name}"))?;

    //
    // Check if branch_hash is reachable from any OTHER branch
    //
    let heads_dir = repo.root.join(".mog/refs/heads");
    let other_heads = std::fs::read_dir(&heads_dir)?
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|b| b != name)
        .filter_map(|b| repo.read_ref(&format!("refs/heads/{b}")).ok())
        .collect::<Vec<_>>();
    let mut other_reachable = Xxh3HashSet::default();
    for h in other_heads {
        other_reachable.extend(repo.reachable_commits(&h));
    }

    if !other_reachable.contains(&branch_hash) {
        bail!(
            "branch '{name}' has commits that are not merged into any other branch.\n\
             use 'mog branch -D {name}' to force delete."
        );
    }

    std::fs::remove_file(branch_path(repo, name))?;
    println!("deleted branch '{name}'");
    Ok(())
}

/// Force delete - no safety check.
pub fn force_delete(repo: &mut Repository, name: &str) -> Result<()> {
    if !branch_exists(repo, name) {
        bail!("branch '{name}' not found");
    }

    if repo.current_branch()?.as_deref() == Some(name) {
        bail!("cannot delete branch '{name}': it is currently checked out");
    }

    let hash = repo.read_ref(&format!("refs/heads/{name}"))?;
    std::fs::remove_file(branch_path(repo, name))?;
    println!("force-deleted branch '{name}' (was {})", &hash_to_hex(&hash)[..8]);
    Ok(())
}

pub fn rename(repo: &Repository, old: &str, new: &str) -> Result<()> {
    if !branch_exists(repo, old) {
        bail!("branch '{old}' not found");
    }

    if branch_exists(repo, new) {
        bail!("branch '{new}' already exists");
    }

    validate_branch_name(new)?;

    let hash = repo.read_ref(&format!("refs/heads/{old}"))?;
    repo.write_ref(&format!("refs/heads/{new}"), &hash)?;
    std::fs::remove_file(branch_path(repo, old))?;

    //
    // If we renamed the currently checked out branch, update HEAD too
    //
    if repo.current_branch()?.as_deref() == Some(old) {
        std::fs::write(
            repo.root.join(".mog/HEAD"),
            format!("ref: refs/heads/{new}\n"),
        )?;
    }

    println!("renamed branch '{old}' to '{new}'");
    Ok(())
}

// Reject names that would break the filesystem or confuse path parsing.
fn validate_branch_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("branch name cannot be empty");
    }
    if name.contains('/') {
        bail!("branch name cannot contain '/' (namespaced branches not yet supported)");
    }
    if name.contains(' ') || name.contains('\t') {
        bail!("branch name cannot contain whitespace");
    }
    if name.starts_with('-') {
        bail!("branch name cannot start with '-'");
    }
    if name == "HEAD" {
        bail!("'HEAD' is not a valid branch name");
    }
    Ok(())
}
