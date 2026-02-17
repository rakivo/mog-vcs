use crate::{hash::{hash_to_hex}, object::Object, repository::Repository};

use anyhow::Result;

pub fn log(repo: &Repository, start_ref: &str, f: &mut dyn core::fmt::Write) -> Result<()> {
    let mut current = repo.read_ref(start_ref)?;
    loop {
        let Object::Commit(commit) = repo.storage.read(&current)? else {
            continue;
        };

        writeln!(f, "commit {}", hash_to_hex(&current))?;
        writeln!(f, "Author: {}", commit.author)?;
        writeln!(f, "Date: {}", commit.timestamp)?;
        writeln!(f, "\n    {}\n", commit.message)?;

        if commit.parents.is_empty() {
            break;
        }

        current = commit.parents[0];
    }

    Ok(())
}
