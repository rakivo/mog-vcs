use crate::hash::hash_to_hex;
use crate::repository::Repository;

use anyhow::Result;

pub fn log(repo: &mut Repository, f: &mut dyn core::fmt::Write) -> Result<()> {
    let Ok(mut current) = repo.read_head_commit() else {
        writeln!(f, "[looks like no commits yet brudda]")?;
        return Ok(());
    };

    loop {
        let obj = repo.read_object(&current)?;
        let Ok(commit_id) = obj.try_as_commit_id() else {
            continue;
        };

        writeln!(f, "commit {}", hash_to_hex(&current))?;
        writeln!(f, "Author: {}", repo.commit_store.get_author(commit_id))?;
        writeln!(f, "Date: {}", repo.commit_store.get_timestamp(commit_id))?;
        writeln!(f, "\n    {}", repo.commit_store.get_message(commit_id))?;
        writeln!(f)?;

        let parents = repo.commit_store.get_parents(commit_id);
        if parents.is_empty() {
            break;
        }
        current = parents[0];
    }

    Ok(())
}
