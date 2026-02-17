use std::path::Path;
use std::fs;
use anyhow::Result;
use crate::object::{Object, Blob};
use crate::repository::Repository;
use crate::hash::hash_to_hex;

pub fn hash_object(repo: &Repository, path: &Path, write: bool) -> Result<()> {
    let data = fs::read(path)?;
    let blob = Blob { data };
    let obj = Object::Blob(blob);
    
    let hash = if write {
        repo.storage.write(&obj)?
    } else {
        obj.hash()
    };
    
    println!("{}", hash_to_hex(&hash));
    Ok(())
}
