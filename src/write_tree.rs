use crate::repository::Repository;
use crate::object::{MODE_FILE, MODE_EXEC, MODE_DIR};
use crate::object::Object;
use crate::tree::TreeEntry;
use crate::hash::Hash;
use crate::util::is_executable;

use std::path::Path;
use std::fs::{self, DirEntry};

use anyhow::Result;

#[inline]
pub fn write_tree(repo: &mut Repository, dir: impl AsRef<Path>) -> Result<Hash> {
    let hash = write_tree_impl(repo, dir.as_ref())?;
    repo.storage.flush()?;
    Ok(hash)
}

fn write_tree_impl(repo: &mut Repository, root: &Path) -> Result<Hash> {
    struct Frame {
        dir:     Box<Path>,
        entries: Vec<DirEntry>,
        built:   Vec<TreeEntry>,
    }

    let mut stack = vec![Frame {
        dir:     root.to_path_buf().into(),
        entries: sorted_dir_entries(root)?,
        built:   Vec::new(),
    }];

    loop {
        //
        // Process entries from the top frame one at a time.
        // When we hit a subdirectory, push a new frame and continue.
        // When a frame is exhausted, pop it and add its tree hash to the parent.
        //
        let next_entry = stack.last_mut().unwrap().entries.pop();

        if let Some(entry) = next_entry {
            let path = entry.path();
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            if let Ok(rel) = path.strip_prefix(&repo.root) {
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                if repo.ignore.is_ignored_rel(&rel_str) {
                    continue;
                }
            }

            if name_str == ".mog" {
                continue;
            }

            let metadata = entry.metadata()?;

            if metadata.is_dir() {
                stack.push(Frame {
                    entries: sorted_dir_entries(&path)?,
                    dir:     path.into(),
                    built:   Vec::new(),
                });

                continue;
            }

            //
            // Blob: read, hash, write object without pushing into blob store.
            //
            let data = fs::read(&path)?;
            let hash = repo.write_blob(&data);
            let mode = if is_executable(&metadata) { MODE_EXEC } else { MODE_FILE };

            stack.last_mut().unwrap().built.push(TreeEntry {
                hash,
                name: name_str.into(),
                mode,
            });
        } else {
            //
            // This frame is done build its tree object and pop.
            //
            let frame = stack.pop().unwrap();
            let tree_id = repo.tree.push(&frame.built);
            let hash = repo.write_object(Object::Tree(tree_id));

            if stack.is_empty() {
                return Ok(hash);
            }

            let parent_name = frame.dir.file_name().unwrap().to_string_lossy();
            stack.last_mut().unwrap().built.push(TreeEntry {
                hash,
                name: parent_name.into(),
                mode: MODE_DIR,
            });
        }
    }
}

#[inline]
fn sorted_dir_entries(dir: &Path) -> Result<Vec<DirEntry>> {
    let mut entries = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();

    // Reverse sort so `.pop()` yields entries in forward order.
    entries.sort_by_key(|b| std::cmp::Reverse(b.file_name()));
    Ok(entries)
}
