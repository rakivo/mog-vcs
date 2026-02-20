// Remove: unstage paths from index (opposite of add), with optional regex patterns.

use std::path::PathBuf;

use crate::tracy;
use crate::repository::Repository;
use crate::index::Index;

use anyhow::Result;

pub fn remove(repo: &mut Repository, patterns: &[PathBuf]) -> Result<()> {
    let _span = tracy::span!("remove::remove");

    let current_dir = std::env::current_dir()?;
    let mut index   = Index::load(&repo.root)?;

    //
    //
    // Classify patterns into literal roots or a combined regex.
    //
    //

    let default = [PathBuf::from(".")];
    let patterns = if patterns.is_empty() { &default } else { patterns };
    let (literal_roots, combined_re) = crate::add::classify_patterns(patterns, &current_dir);

    //
    //
    // Collect paths to unstage.
    //
    //

    let mut paths_to_unstage = crate::add::walk_matching(
        &repo.root,
        &repo.ignore,
        &literal_roots,
        combined_re.as_ref()
    ).into_iter().map(|(_path, rel)| rel).collect::<Vec<_>>();

    //
    //
    // Regex-based removals: also match directly against index paths (handles
    // files that no longer exist on disk but are still staged).
    //
    //

    if let Some(re) = &combined_re {
        let _span = tracy::span!("remove::regex_over_index");
        for i in 0..index.count {
            let path_str = index.get_path(i);
            if re.is_match(path_str) {
                paths_to_unstage.push(path_str.into());
            }
        }
    }

    paths_to_unstage.sort_unstable();
    paths_to_unstage.dedup();

    let mut removed_count = 0usize;
    for rel_string in &paths_to_unstage {
        if index.remove(rel_string) {
            removed_count += 1;
        }
    }

    if removed_count > 0 {
        index.save(&repo.root)?;
        println!("Removed {removed_count} path(s) from index");
    } else {
        println!("No matching paths in index");
    }

    Ok(())
}
