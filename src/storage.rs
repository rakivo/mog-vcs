use crate::hash::{Hash, hash_to_hex};
use crate::object::Object;

use std::path::{Path, PathBuf};
use std::fs;

use anyhow::Result;

pub struct Storage {
    root: PathBuf,
}

impl Storage {
    #[inline]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    #[inline]
    fn object_path(&self, hash: &Hash) -> PathBuf {
        let hex = hash_to_hex(hash);
        self.root
            .join("objects")
            .join(&hex[..2])
            .join(&hex[2..])
    }

    #[inline]
    pub fn write(&self, obj: &Object) -> Result<Hash> {
        let hash = obj.hash();
        let path = self.object_path(&hash);

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let encoded = obj.encode();
        fs::write(path, encoded)?;

        Ok(hash)
    }

    #[inline]
    pub fn read(&self, hash: &Hash) -> Result<Object> {
        let path = self.object_path(hash);
        let data = fs::read(path)?;
        Object::decode(&data)
    }

    #[inline]
    pub fn exists(&self, hash: &Hash) -> bool {
        self.object_path(hash).exists()
    }
}
