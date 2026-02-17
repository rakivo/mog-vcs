use anyhow::Result;
use crate::hash::hex_to_hash;
use crate::repository::Repository;
use crate::object::Object;

pub fn cat_file(repo: &Repository, hash_str: &str) -> Result<()> {
    let hash = hex_to_hash(hash_str)?;
    let obj = repo.storage.read(&hash)?;

    match obj {
        Object::Blob(blob) => {
            println!("{}", String::from_utf8_lossy(&blob.data));
        }
        Object::Tree(tree) => {
            for i in 0..tree.count {
                println!(
                    "0o{:06o} {} {}",
                    tree.modes[i],
                    hex::encode(tree.hashes[i]),
                    tree.get_name(i)
                );
            }
        }
        Object::Commit(commit) => {
            println!("tree {}", hex::encode(commit.tree));
            for parent in &commit.parents {
                println!("parent {}", hex::encode(parent));
            }
            println!("author {} {}", commit.author, commit.timestamp);
            println!("\n{}", commit.message);
        }
    }

    Ok(())
}
