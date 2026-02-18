// Remove: unstage paths from index (opposite of add), with optional regex patterns.

use std::path::PathBuf;

use crate::tracy;
use crate::repository::Repository;
use crate::index::Index;

use anyhow::Result;
use regex::Regex;
use walkdir::WalkDir;

pub fn remove(repo: &mut Repository, patterns: &[PathBuf]) -> Result<()> {
    let _span = tracy::span!("remove::remove");

    let current_dir = std::env::current_dir()?;
    let mut index = Index::load(&repo.root)?;

    let default = vec![PathBuf::from(".")];
    let patterns = if patterns.is_empty() { &default } else { patterns };

    let mut literal_roots: Vec<PathBuf> = Vec::new();
    let mut regexes: Vec<Regex> = Vec::new();

    // Partition into existing paths (literals) and regex patterns.
    for p in patterns {
        let candidate = if p.is_absolute() {
            p.clone()
        } else {
            current_dir.join(p)
        };
        if candidate.exists() {
            literal_roots.push(candidate);
        } else {
            let s = p.to_string_lossy();
            match Regex::new(&s) {
                Ok(re) => regexes.push(re),
                Err(_) => {
                    eprintln!("Invalid regex pattern '{}', skipping", s);
                }
            }
        }
    }

    let mut paths_to_unstage = Vec::new();

    {
        let _span = tracy::span!("remove::collect_literal_paths");
        for full in literal_roots {
            let full = full.canonicalize()?;
            if full.is_file() {
                if let Ok(rel) = full.strip_prefix(&repo.root) {
                    paths_to_unstage.push(rel.to_string_lossy().replace('\\', "/"));
                }
            } else if full.is_dir() {
                for entry in WalkDir::new(&full)
                    .into_iter()
                    .filter_entry(|e| !repo.ignore.is_ignored_abs(e.path()))
                    .filter_map(Result::ok)
                {
                    if entry.file_type().is_file() {
                        if let Ok(rel) = entry.path().strip_prefix(&repo.root) {
                            paths_to_unstage.push(rel.to_string_lossy().replace('\\', "/"));
                        }
                    }
                }
            }
        }
    }

    // Regex-based removals: operate over index paths.
    if !regexes.is_empty() {
        let _span = tracy::span!("remove::regex_over_index");

        for i in 0..index.count {
            let path_str = index.get_path(i);
            if regexes.iter().any(|re| re.is_match(path_str)) {
                paths_to_unstage.push(path_str.to_string());
            }
        }
    }

    // Deduplicate.
    paths_to_unstage.sort();
    paths_to_unstage.dedup();

    let mut removed_count = 0usize;
    for rel_str in paths_to_unstage {
        if index.remove(PathBuf::from(&rel_str).as_path()) {
            removed_count += 1;
        }
    }

    if removed_count > 0 {
        index.save(&repo.root)?;
        println!("Removed {} path(s) from index", removed_count);
    } else {
        println!("No matching paths in index");
    }

    Ok(())
}
