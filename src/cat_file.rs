use anyhow::Result;
use crate::hash::hex_to_hash;
use crate::repository::Repository;
use crate::object::Object;
use crate::tree::TreeEntryRef;

pub fn cat_file(repo: &mut Repository, hash_str: &str) -> Result<()> {
    let hash = hex_to_hash(hash_str)?;
    let obj = repo.read_object(&hash)?;

    match obj {
        Object::Blob(id) => {
            let data = repo.blob.get(id);
            println!("{}", String::from_utf8_lossy(data));
        }
        Object::Tree(id) => {
            let n = repo.tree.entry_count(id);
            for j in 0..n {
                let TreeEntryRef { mode, hash: entry_hash, name } = repo.tree.get_entry_ref(id, j);
                println!("0o{:06o} {} {}", mode, hex::encode(entry_hash), name);
            }
        }
        Object::Commit(id) => {
            println!("tree {}", hex::encode(repo.commit.get_tree(id)));
            for parent in repo.commit.get_parents(id) {
                println!("parent {}", hex::encode(parent));
            }
            println!(
                "author {} {}",
                repo.commit.get_author(id),
                repo.commit.get_timestamp(id)
            );
            println!("\n{}", repo.commit.get_message(id));
        }
    }

    Ok(())
}
