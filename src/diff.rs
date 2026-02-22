use crate::index::Index;
use crate::hash::hash_bytes;
use crate::repository::Repository;
use crate::status::SortedFlatTree;

use std::io::Write as _;
use std::io::BufWriter;

use anyhow::Result;
use imara_diff::{Algorithm, BasicLineDiffPrinter, Diff, InternedInput, UnifiedDiffConfig};

pub fn diff(repo: &mut Repository) -> Result<()> {
    let index = Index::load(&repo.root)?;

    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    for entry in &index {
        if repo.ignore.is_ignored_rel(entry.path) {
            continue;
        }

        //
        // Read from disk
        //

        let Ok(on_disk_data) = std::fs::read(repo.root.join(entry.path)) else {
            continue;
        };
        if hash_bytes(&on_disk_data) == *entry.hash {
            continue; // Unchanged!
        }
        let Ok(after) = std::str::from_utf8(&on_disk_data) else {
            writeln!(out, "Binary files differ: {}", entry.path)?;
            continue;
        };

        //
        // Read from index
        //

        let Ok(before) = repo.read_blob_bytes_without_touching_cache(&entry.hash) else {
            writeln!(out, "Binary files differ: {}", entry.path)?;
            continue;
        };
        let Ok(before) = std::str::from_utf8(&before) else {
            continue;
        };

        print_diff(before, after, entry.path, &mut out)?;
    }

    Ok(())
}

pub fn diff_staged(repo: &mut Repository) -> Result<()> {
    let index = Index::load(&repo.root)?;

    let head_flat = match repo.read_head_commit() {
        Ok(head_hash) => {
            let obj = repo.read_object(&head_hash)?;
            let commit_id = obj.try_as_commit_id()?;
            let tree_hash = repo.commit.get_tree(commit_id);
            crate::status::flatten_tree(repo, tree_hash)?
        }
        Err(_) => SortedFlatTree::default(), // No commits yet, empty tree!
    };

    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    for entry in &index {
        match head_flat.lookup(entry.path) {
            Some(head_hash) => {
                if head_hash == *entry.hash {
                    continue; // Unchanged!
                }

                //
                // Read from HEAD
                //

                let Ok(before) = repo.read_blob_bytes_without_touching_cache(&head_hash) else {
                    continue;
                };
                let Ok(before) = std::str::from_utf8(&before) else {
                    writeln!(&mut out, "Binary files differ: {}", entry.path)?;
                    continue;
                };

                //
                // Read from index
                //

                let Ok(after) = repo.read_blob_bytes_without_touching_cache(&entry.hash) else {
                    continue;
                };
                let Ok(after) = std::str::from_utf8(&after) else {
                    writeln!(&mut out, "Binary files differ: {}", entry.path)?;
                    continue;
                };

                print_diff(before, after, entry.path, &mut out)?;
            }
            None => {
                //
                // New file added in index, didn't exist in HEAD
                //

                let Ok(after) = repo.read_blob_bytes_without_touching_cache(&entry.hash) else {
                    continue;
                };
                let Ok(after) = std::str::from_utf8(&after) else {
                    writeln!(&mut out, "Binary files differ: {}", entry.path)?;
                    continue;
                };

                print_diff("", after, entry.path, &mut out)?;
            }
        }
    }

    Ok(())
}

fn print_diff(before: &str, after: &str, path: &str, out: &mut BufWriter<impl std::io::Write>) -> Result<()> {
    let input = InternedInput::new(before, &after);
    let mut diff = Diff::compute(Algorithm::Histogram, &input);
    diff.postprocess_lines(&input);

    if diff.hunks().next().is_none() {
        return Ok(()); // Empty diff!
    }

    let printer = BasicLineDiffPrinter(&input.interner);
    let unified_diff = diff.unified_diff(
        &printer,
        UnifiedDiffConfig::default(),
        &input,
    );

    writeln!(out, "--- a/{path}")?;
    writeln!(out, "+++ b/{path}")?;
    writeln!(out, "{unified_diff}")?;

    Ok(())
}
