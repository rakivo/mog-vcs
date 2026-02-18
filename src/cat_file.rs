use anyhow::Result;
use crate::hash::hex_to_hash;
use crate::repository::Repository;
use crate::object::Object;
use crate::tree::TreeEntry;

pub fn cat_file(repo: &mut Repository, hash_str: &str) -> Result<()> {
    let hash = hex_to_hash(hash_str)?;
    let obj = repo.read_object(&hash)?;

    match obj {
        Object::Blob(id) => {
            let data = repo.blob_store.get(id);
            println!("{}", String::from_utf8_lossy(data));
        }
        Object::Tree(id) => {
            let n = repo.tree_store.entry_count(id);
            for j in 0..n {
                let TreeEntry { mode, hash: entry_hash, name } = repo.tree_store.get_entry(id, j);
                println!("0o{:06o} {} {}", mode, hex::encode(entry_hash), name);
            }
        }
        Object::Commit(id) => {
            println!("tree {}", hex::encode(repo.commit_store.get_tree(id)));
            for parent in repo.commit_store.get_parents(id) {
                println!("parent {}", hex::encode(parent));
            }
            println!(
                "author {} {}",
                repo.commit_store.get_author(id),
                repo.commit_store.get_timestamp(id)
            );
            println!("\n{}", repo.commit_store.get_message(id));
        }
    }

    Ok(())
}
