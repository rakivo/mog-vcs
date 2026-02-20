use anyhow::Result;
use crate::hash::hex_to_hash;
use crate::repository::Repository;
use crate::object::Object;
use crate::tree::TreeEntryRef;

pub fn cat_file(repo: &mut Repository, hash_str: &str, f: &mut dyn core::fmt::Write) -> Result<()> {
    let hash = hex_to_hash(hash_str)?;
    let object = repo.read_object(&hash)?;

    match object {
        Object::Blob(id) => {
            let data = repo.blob.get(id);
            writeln!(f, "{}", String::from_utf8_lossy(data))?;
        }
        Object::Tree(id) => {
            let n = repo.tree.entry_count(id);
            for j in 0..n {
                let TreeEntryRef { mode, hash: entry_hash, name } = repo.tree.get_entry_ref(id, j);
                writeln!(f, "0o{:06o} {} {}", mode, hex::encode(entry_hash), name)?;
            }
        }
        Object::Commit(id) => {
            writeln!(f, "tree {}", hex::encode(repo.commit.get_tree(id)))?;
            for parent in repo.commit.get_parents(id) {
                writeln!(f, "parent {}", hex::encode(parent))?;
            }
            writeln!(f,
                "author {} {}",
                repo.commit.get_author(id),
                repo.commit.get_timestamp(id)
            )?;
            writeln!(f, "\n{}", repo.commit.get_message(id))?;
        }
    }

    Ok(())
}
