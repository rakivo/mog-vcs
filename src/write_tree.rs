use std::path::Path;
use std::fs;
use anyhow::Result;
use crate::repository::Repository;
use crate::object::{Object, Blob, MODE_FILE, MODE_EXEC, MODE_DIR};
use crate::tree_builder::TreeBuilder;
use crate::hash::Hash;

pub fn write_tree(repo: &Repository, dir: &Path) -> Result<Hash> {
    write_tree_recursive(repo, dir)
}

fn write_tree_recursive(repo: &Repository, dir: &Path) -> Result<Hash> {
    let mut builder = TreeBuilder::new();

    let mut entries = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .collect::<Vec<_>>();

    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip .vx directory
        if name == ".vx" {
            continue;
        }

        let metadata = entry.metadata()?;

        if metadata.is_dir() {
            // Recursively write subtree
            let hash = write_tree_recursive(repo, &path)?;
            builder.add(MODE_DIR, hash, &name);
        } else {
            // Write blob
            let data = fs::read(&path)?;
            let blob = Blob { data: crate::util::vec_into_boxed_slice_noshrink(data) };
            let hash = repo.storage.write(&Object::Blob(blob))?;

            let mode = if is_executable(&metadata) {
                MODE_EXEC
            } else {
                MODE_FILE
            };

            builder.add(mode, hash, &name);
        }
    }

    let tree = builder.build();
    repo.storage.write(&Object::Tree(tree))
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
