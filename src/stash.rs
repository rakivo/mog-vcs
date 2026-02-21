use crate::repository::Repository;
use crate::index::Index;
use crate::object::{Object, MODE_FILE, MODE_EXEC};
use crate::tree::TreeEntry;
use crate::hash::{hash_to_hex, Hash};
use crate::util::is_executable;

use std::fs;
use std::path::Path;

use anyhow::{Result, bail};

// TODO: Names for stashes

pub fn stash(repo: &mut Repository) -> Result<()> {
    let index = Index::load(&repo.root)?;

    //
    //
    // Build a tree from the full index state (staged files).
    //
    //

    let staged_entries = (0..index.count).map(|i| TreeEntry {
        hash: index.hashes[i],
        name: index.get_path(i).into(),
        mode: index.modes[i],
    }).collect::<Vec<_>>();
    let staged_tree_id   = repo.tree.push(&staged_entries);
    let staged_tree_hash = repo.write_object(Object::Tree(staged_tree_id));

    //
    //
    // Build a tree from dirty disk files (disk vs index).
    //
    //

    let mut dirty_entries = Vec::new();
    for i in 0..index.count {
        let path_str = index.get_path(i);
        let abs      = repo.root.join(path_str);
        let Ok(meta) = fs::metadata(&abs) else { continue };

        let mtime = meta.modified().ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs() as i64);

        let size = meta.len();
        if index.mtimes[i] == mtime && index.sizes[i] == size {
            continue;
        }

        let data = fs::read(&abs)?;
        let hash = repo.write_blob(&data);
        dirty_entries.push(TreeEntry {
            hash,
            name: path_str.into(),
            mode: if is_executable(&meta) { MODE_EXEC } else { MODE_FILE },
        });
    }
    let dirty_tree_id   = repo.tree.push(&dirty_entries);
    let dirty_tree_hash = repo.write_object(Object::Tree(dirty_tree_id));

    if staged_entries.is_empty() && dirty_entries.is_empty() {
        println!("No local changes to stash");
        return Ok(());
    }

    //
    //
    // Write stash commit: parent = HEAD, tree = staged state.
    // Store dirty tree hash in commit message for simplicity.
    //
    //

    let stash_count = count_stashes(repo)?;
    let dirty_hex   = hash_to_hex(&dirty_tree_hash);
    let message     = format!("dirty={dirty_hex}");
    let parent      = repo.read_head_commit().ok();
    let timestamp   = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let commit_id   = repo.commit.push(
        staged_tree_hash,
        &parent.into_iter().collect::<Vec<_>>(),
        timestamp, "stash", &message,
    );
    let stash_hash = repo.write_object(Object::Commit(commit_id));
    repo.storage.flush()?;

    //
    //
    // Save stash ref
    //
    //

    let refs_dir = repo.root.join(".mog/refs/stash");
    fs::create_dir_all(&refs_dir)?;
    shift_stash_refs_up(repo)?;
    fs::write(refs_dir.join("0"), format!("{}\n", hash_to_hex(&stash_hash)))?;

    //
    //
    // Restore working dir and index to HEAD state
    //
    //

    match repo.read_head_commit().ok() {
        Some(head_hash) => {
            let object    = repo.read_object(&head_hash)?;
            let commit_id = object.try_as_commit_id()?;
            let tree_hash = repo.commit.get_tree(commit_id);
            let head_flat = crate::status::flatten_tree(repo, tree_hash)?;

            //
            // Restore index to HEAD.
            //
            let mut new_index = Index::default();
            for j in 0..head_flat.len() {
                let path_str = head_flat.get_path(j);
                let hash     = head_flat.hashes[j];
                let abs      = repo.root.join(path_str);
                let obj      = repo.read_object(&hash)?;
                let _        = obj.try_as_blob_id()?; // assert it's a blob
                let raw      = repo.storage.read(&hash)?;
                let data     = crate::object::decode_blob_bytes(raw)?;
                fs::write(&abs, data)?;
                repo.storage.evict_pages(raw);
                let meta = fs::metadata(&abs)?;
                new_index.add(path_str, hash, &meta);
            }
            new_index.save(&repo.root)?;
        }
        None => {
            //
            // No HEAD just clear the index and delete tracked files.
            //
            for i in 0..index.count {
                let abs = repo.root.join(index.get_path(i));
                _ = fs::remove_file(&abs);
            }
            crate::discard::remove_empty_dirs(&repo.root)?;
            Index::default().save(&repo.root)?;
        }
    }

    println!(
        "Saved stash@{{{stash_count}}}: {} staged, {} dirty file(s)",
        staged_entries.len(), dirty_entries.len()
    );

    Ok(())
}

pub fn stash_apply(repo: &mut Repository, index: usize) -> Result<()> {
    let stash_ref = repo.root.join(format!(".mog/refs/stash/{index}"));
    if !stash_ref.exists() {
        bail!("no stash entry stash@{{{index}}}");
    }

    let stash_hash = repo.read_ref(&format!("refs/stash/{index}"))?;
    apply_stash(repo, stash_hash)?;

    println!("Applied stash@{{{index}}} (stash entry remains, use 'mog stash drop {index}' to remove)");
    Ok(())
}

pub fn stash_pop(repo: &mut Repository) -> Result<()> {
    let stash_ref = repo.root.join(".mog/refs/stash/0");
    if !stash_ref.exists() {
        bail!("no stash entries found");
    }

    let stash_hash = repo.read_ref("refs/stash/0")?;
    apply_stash(repo, stash_hash)?;
    fs::remove_file(&stash_ref)?;
    shift_stash_refs_down(repo)?;

    println!("Restored and dropped stash@{{0}}");
    Ok(())
}

pub fn stash_drop(repo: &Repository, index: usize) -> Result<()> {
    let stash_ref = repo.root.join(format!(".mog/refs/stash/{index}"));
    if !stash_ref.exists() {
        bail!("no stash entry stash@{{{index}}}");
    }
    fs::remove_file(&stash_ref)?;
    shift_stash_refs_down_from(repo, index)?;
    println!("Dropped stash@{{{index}}}");
    Ok(())
}

pub fn stash_list(repo: &mut Repository) -> Result<()> {
    let refs_dir = repo.root.join(".mog/refs/stash");
    if !refs_dir.exists() {
        println!("No stash entries");
        return Ok(());
    }

    let mut entries = read_stash_indexes(&refs_dir)?.collect::<Vec<_>>();

    if entries.is_empty() {
        println!("No stash entries");
        return Ok(());
    }

    entries.sort_unstable_by(|a, b| b.cmp(a)); // Print em like its magit

    for n in entries {
        let hash    = repo.read_ref(&format!("refs/stash/{n}"))?;
        let object  = repo.read_object_without_touching_cache(&hash)?;
        let commit  = object.try_as_commit_id()?;
        let message = repo.commit.get_message(commit);
        println!("stash@{{{n}}}: {message}");
    }

    Ok(())
}

#[inline]
fn count_stashes(repo: &Repository) -> Result<usize> {
    let refs_dir = repo.root.join(".mog/refs/stash");
    if !refs_dir.exists() {
        return Ok(0);
    }

    let count = read_stash_indexes(&refs_dir)?.count();

    Ok(count)
}

#[inline]
fn shift_stash_refs_up(repo: &Repository) -> Result<()> {
    let refs_dir = repo.root.join(".mog/refs/stash");
    let mut indexes = read_stash_indexes(&refs_dir)?.collect::<Vec<_>>();

    // Rename highest first to avoid clobbering.
    indexes.sort_unstable_by(|a, b| b.cmp(a));
    for n in indexes {
        fs::rename(refs_dir.join(n.to_string()), refs_dir.join((n + 1).to_string()))?;
    }

    Ok(())
}

fn shift_stash_refs_down_from(repo: &Repository, from: usize) -> Result<()> {
    let refs_dir = repo.root.join(".mog/refs/stash");
    let mut indices = fs::read_dir(&refs_dir)?
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter_map(|n| n.parse::<usize>().ok())
        .filter(|&n| n > from)
        .collect::<Vec<_>>();

    indices.sort_unstable();
    for n in indices {
        fs::rename(refs_dir.join(n.to_string()), refs_dir.join((n - 1).to_string()))?;
    }
    Ok(())
}

#[inline]
fn shift_stash_refs_down(repo: &Repository) -> Result<()> {
    shift_stash_refs_down_from(repo, 0)
}

#[inline]
fn read_stash_indexes(refs_dir: impl AsRef<Path>) -> Result<impl Iterator<Item = u32>> {
    Ok(fs::read_dir(refs_dir)?
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter_map(|n| n.parse::<u32>().ok()))
}

fn apply_stash(repo: &mut Repository, stash_hash: Hash) -> Result<()> {
    let obj       = repo.read_object(&stash_hash)?;
    let commit_id = obj.try_as_commit_id()?;
    let message   = repo.commit.get_message(commit_id).to_string();
    let tree_hash = repo.commit.get_tree(commit_id);

    let dirty_tree_hash = message
        .lines()
        .find(|l| l.starts_with("dirty="))
        .and_then(|l| crate::hash::hex_to_hash(l.trim_start_matches("dirty=")).ok());

    //
    // Restore staged state into index and disk.
    //
    let staged_obj     = repo.read_object(&tree_hash)?;
    let staged_tree_id = staged_obj.try_as_tree_id()?;
    let n              = repo.tree.entry_count(staged_tree_id);
    let mut index      = Index::load(&repo.root)?;

    for j in 0..n {
        let TreeEntry { hash, name, .. } = repo.tree.get_entry(staged_tree_id, j);
        let abs = repo.root.join(name.as_ref());

        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent)?;
        }

        {
            let raw  = repo.storage.read(&hash)?;
            let data = crate::object::decode_blob_bytes(raw)?;
            fs::write(&abs, data)?;
            repo.storage.evict_pages(raw);
        }

        let meta = fs::metadata(&abs)?;
        index.add(name.as_ref(), hash, &meta);
    }

    //
    // Overlay dirty disk changes on top.
    //
    if let Some(dirty_hash) = dirty_tree_hash {
        let dirty_obj     = repo.read_object(&dirty_hash)?;
        let dirty_tree_id = dirty_obj.try_as_tree_id()?;
        let m             = repo.tree.entry_count(dirty_tree_id);
        for j in 0..m {
            let TreeEntry { hash, name, .. } = repo.tree.get_entry(dirty_tree_id, j);
            let abs  = repo.root.join(name.as_ref());
            {
                let raw  = repo.storage.read(&hash)?;
                let data = crate::object::decode_blob_bytes(raw)?;
                fs::write(&abs, data)?;
                repo.storage.evict_pages(raw);
            }
            //
            // Don't update index, dirty files should show as modified.
            //
        }
    }

    index.save(&repo.root)?;
    Ok(())
}
