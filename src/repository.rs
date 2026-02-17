use crate::object::{Commit, Object};
use crate::storage::Storage;
use crate::hash::{Hash, hash_to_hex, hex_to_hash};
use crate::util::Xxh3HashSet;

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

pub struct Repository {
    pub root: PathBuf,
    pub storage: Storage,
}

impl Repository {
    #[inline]
    pub fn init(path: &Path) -> Result<Self> {
        let vx_dir = path.join(".vx");

        std::fs::create_dir_all(&vx_dir)?;
        std::fs::create_dir_all(vx_dir.join("objects"))?;
        std::fs::create_dir_all(vx_dir.join("refs/heads"))?;
        std::fs::create_dir_all(vx_dir.join("refs/remotes"))?;

        std::fs::write(
            vx_dir.join("HEAD"),
            b"ref: refs/heads/main\n"
        )?;

        Ok(Self {
            root: path.canonicalize()?.to_path_buf(),
            storage: Storage::new(vx_dir),
        })
    }

    #[inline]
    pub fn open(path: &Path) -> Result<Self> {
        let vx_dir = path.join(".vx");

        if !vx_dir.exists() {
            bail!("not a vx repository");
        }

        Ok(Self {
            root: path.canonicalize()?.to_path_buf(),
            storage: Storage::new(vx_dir),
        })
    }

    #[inline]
    pub fn read_ref(&self, refname: &str) -> Result<Hash> {
        let path = self.root.join(".vx").join(refname);
        let content = std::fs::read_to_string(path)?;
        hex_to_hash(content.trim())
    }

    #[inline]
    pub fn write_ref(&self, refname: &str, hash: &Hash) -> Result<()> {
        let path = self.root.join(".vx").join(refname);

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(path, format!("{}\n", hash_to_hex(hash)))?;
        Ok(())
    }

    /// Read the commit hash HEAD currently points to,
    /// whether HEAD is a branch ref or detached
    #[inline]
    pub fn read_head_commit(&self) -> Result<Hash> {
        let head = std::fs::read_to_string(self.root.join(".vx/HEAD"))?;
        let head = head.trim();

        if let Some(refpath) = head.strip_prefix("ref: ") {
            // Normal: follow the ref
            self.read_ref(refpath.trim())
        } else {
            // Detached: HEAD is the hash
            hex_to_hash(head)
        }
    }

    /// Return current branch name, or None if detached
    #[inline]
    pub fn current_branch(&self) -> Result<Option<String>> {
        let head = std::fs::read_to_string(self.root.join(".vx/HEAD"))?;
        let head = head.trim();

        if let Some(refpath) = head.strip_prefix("ref: ") {
            let branch = refpath
                .trim()
                .strip_prefix("refs/heads/")
                .map(|s| s.to_string());
            Ok(branch)
        } else {
            Ok(None) // detached
        }
    }

    // Resolve a branch name or commit hash to a Commit object
    #[inline]
    pub fn resolve_to_commit(&self, target: &str) -> Result<Commit> {
        //
        // Try as branch first
        //
        let branch_ref  = format!("refs/heads/{target}");
        let branch_path = self.root.join(".vx").join(&branch_ref);

        let hash = if branch_path.exists() {
            self.read_ref(&branch_ref)?
        } else {
            //
            // Try as raw commit hash
            //
            hex_to_hash(target)?
        };

        self.storage.read(&hash)?.try_into_commit()
    }

    // Walk commit graph from `start`, collecting all reachable commit hashes.
    // Used for merge-safety check on delete.
    #[inline]
    pub fn reachable_commits(&self, start: &Hash) -> Xxh3HashSet<Hash> {
        let mut visited = Xxh3HashSet::default();
        let mut stack   = vec![*start];

        while let Some(hash) = stack.pop() {
            if visited.contains(&hash) { continue; }
            visited.insert(hash);

            if let Ok(commit) = self.storage.read(&hash).and_then(Object::try_into_commit) {
                stack.extend(commit.parents);
            }
        }

        visited
    }

    // Walk a tree object following path components.
    // e.g. path = "src/foo/bar.rs"
    //   -> look for "src" in root tree  (Tree)
    //   -> look for "foo" in src tree   (Tree)
    //   -> look for "bar.rs" in foo tree (Blob)
    //   -> return the Blob
    pub fn walk_tree_path(&self, tree_hash: &Hash, path: &str) -> Result<(Object, Hash)> {
        let mut current = self.storage.read(tree_hash)?.try_into_tree()?;

        let components = path
            .trim_matches('/')  // strip leading/trailing slashes
            .split('/')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();

        if components.is_empty() {
            bail!("empty path");
        }

        //
        // Walk all components except the last - each must be a tree
        //
        for &component in &components[..components.len() - 1] {
            let hash = current.find_in_tree(component)
                .ok_or_else(|| anyhow::anyhow!("path not found: '{component}'"))?;

            current = self.storage.read(hash)?.try_into_tree()?;
        }

        //
        // Last component can be either blob or tree
        //
        let last = components[components.len() - 1];
        let hash = current.find_in_tree(last)
            .ok_or_else(|| anyhow::anyhow!("path not found: '{last}'"))?;

        self.storage.read(hash).map(|object| (object, *hash))
    }
}
