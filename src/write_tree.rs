use std::path::Path;
use std::fs::{self, DirEntry};
use anyhow::Result;
use crate::repository::Repository;
use crate::object::{MODE_FILE, MODE_EXEC, MODE_DIR};
use crate::object::Object;
use crate::tree::TreeEntry;
use crate::hash::Hash;

pub fn write_tree(repo: &mut Repository, dir: &Path) -> Result<Hash> {
    let hash = write_tree_recursive(repo, dir)?;
    repo.storage.flush()?;
    Ok(hash)
}

fn write_tree_recursive(repo: &mut Repository, dir: &Path) -> Result<Hash> {
    let mut tree_entries_buffer = Vec::new();

    let mut entries = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();

    entries.sort_by_key(DirEntry::file_name);

    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();

        // Ignore rules (repo-root-relative). Tracked files should use `vx add` instead.
        if let Ok(rel) = path.strip_prefix(&repo.root) {
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            if repo.ignore.is_ignored_rel(&rel_str) {
                continue;
            }
        }

        if name == ".vx" {
            continue;
        }

        let metadata = entry.metadata()?;

        if metadata.is_dir() {
            let hash = write_tree_recursive(repo, &path)?;
            tree_entries_buffer.push(TreeEntry {
                hash,
                name: name.into(), // @Clone
                mode: MODE_DIR,
            });

            continue;
        }

        let data = fs::read(&path)?;
        let blob_id = repo.blob_store.push(&data);
        let hash = repo.write_object(Object::Blob(blob_id));

        let mode = if is_executable(&metadata) {
            MODE_EXEC
        } else {
            MODE_FILE
        };

        tree_entries_buffer.push(TreeEntry {
            hash,
            name: name.into(), // @Clone
            mode,
        });
    }

    let tree_id = repo.tree_store.extend(&tree_entries_buffer);
    let hash = repo.write_object(Object::Tree(tree_id));
    Ok(hash)
}

#[cfg(unix)]
fn is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &fs::Metadata) -> bool {
    false
}
