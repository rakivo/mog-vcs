use crate::{hash::{hash_to_hex}, object::Object, repository::Repository};

use anyhow::Result;

pub fn log(repo: &Repository, f: &mut dyn core::fmt::Write) -> Result<()> {
    let mut current = repo.read_head_commit()?;
    loop {
        let Object::Commit(commit) = repo.storage.read(&current)? else {
            continue;
        };

        writeln!(f, "commit {}", hash_to_hex(&current))?;
        writeln!(f, "Author: {}", commit.author)?;
        writeln!(f, "Date: {}", commit.timestamp)?;
        writeln!(f, "\n    {}", commit.message)?;

        if commit.parents.is_empty() {
            break;
        }

        writeln!(f);
        current = commit.parents[0];
    }

    Ok(())
}
