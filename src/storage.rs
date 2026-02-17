use std::path::{Path, PathBuf};
use std::fs;
use anyhow::Result;
use crate::hash::{Hash, hash_to_hex};
use crate::object::Object;

pub struct Storage {
    root: PathBuf,
}

impl Storage {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
    
    fn object_path(&self, hash: &Hash) -> PathBuf {
        let hex = hash_to_hex(hash);
        self.root
            .join("objects")
            .join(&hex[..2])
            .join(&hex[2..])
    }
    
    pub fn write(&self, obj: &Object) -> Result<Hash> {
        let hash = obj.hash();
        let path = self.object_path(&hash);
        
        // Create parent directory
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        // Write encoded object
        let encoded = obj.encode();
        fs::write(path, encoded)?;
        
        Ok(hash)
    }
    
    pub fn read(&self, hash: &Hash) -> Result<Object> {
        let path = self.object_path(hash);
        let data = fs::read(path)?;
        Object::decode(&data)
    }
    
    pub fn exists(&self, hash: &Hash) -> bool {
        self.object_path(hash).exists()
    }
}
