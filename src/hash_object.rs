use std::path::Path;
use std::fs;
use anyhow::Result;
use crate::repository::Repository;
use crate::object::Object;
use crate::store::blob_encode_and_hash;
use crate::hash::hash_to_hex;

pub fn hash_object(repo: &mut Repository, path: &Path, write: bool) -> Result<()> {
    let data = fs::read(path)?;
    let blob_id = repo.blob_store.push(&data);

    let hash = if write {
        let hash = repo.write_object(Object::Blob(blob_id));
        // Persist immediately when writing objects via this command.
        repo.storage.flush()?;
        hash
    } else {
        let mut buf = Vec::new();
        blob_encode_and_hash(&repo.blob_store, blob_id, &mut buf)
    };

    println!("{}", hash_to_hex(&hash));
    Ok(())
}
