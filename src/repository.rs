use std::path::{Path, PathBuf};
use std::fs;
use anyhow::{Result, bail};
use crate::storage::Storage;
use crate::hash::{Hash, hash_to_hex, hex_to_hash};

pub struct Repository {
    pub root: PathBuf,
    pub storage: Storage,
}

impl Repository {
    pub fn init(path: &Path) -> Result<Self> {
        let vx_dir = path.join(".vx");
        
        // Create directory structure
        fs::create_dir_all(&vx_dir)?;
        fs::create_dir_all(vx_dir.join("objects"))?;
        fs::create_dir_all(vx_dir.join("refs/heads"))?;
        fs::create_dir_all(vx_dir.join("refs/remotes"))?;
        
        // Create HEAD
        fs::write(
            vx_dir.join("HEAD"),
            b"ref: refs/heads/main\n"
        )?;
        
        Ok(Self {
            root: path.to_path_buf(),
            storage: Storage::new(vx_dir),
        })
    }
    
    pub fn open(path: &Path) -> Result<Self> {
        let vx_dir = path.join(".vx");
        
        if !vx_dir.exists() {
            bail!("not a vx repository");
        }
        
        Ok(Self {
            root: path.to_path_buf(),
            storage: Storage::new(vx_dir),
        })
    }
    
    pub fn read_ref(&self, refname: &str) -> Result<Hash> {
        let path = self.root.join(".vx").join(refname);
        let content = fs::read_to_string(path)?;
        hex_to_hash(content.trim())
    }
    
    pub fn write_ref(&self, refname: &str, hash: &Hash) -> Result<()> {
        let path = self.root.join(".vx").join(refname);
        
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        fs::write(path, format!("{}\n", hash_to_hex(hash)))?;
        Ok(())
    }
}
