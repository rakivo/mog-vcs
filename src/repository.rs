use crate::cache::EncodedCache;
use crate::ignore::Ignore;
use crate::storage::Storage;
use crate::object::Object;
use crate::store::{decode_object_into_stores, encode_object_into, object_hash, CommitId, Stores};
use crate::hash::{Hash, hash_to_hex, hex_to_hash};
use crate::util::Xxh3HashSet;

use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

pub struct Repository {
    pub root: PathBuf,
    pub storage: Storage,
    pub ignore: Ignore,
    pub object_cache: EncodedCache,
    pub stores: Stores
}

impl Deref for Repository {
    type Target = Stores;
    fn deref(&self) -> &Self::Target { &self.stores }
}

impl DerefMut for Repository {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.stores }
}

impl Repository {
    #[inline]
    pub fn init(path: &Path) -> Result<Self> {
        let mog_dir = path.join(".mog");

        std::fs::create_dir_all(&mog_dir)?;
        std::fs::create_dir_all(mog_dir.join("objects"))?;
        std::fs::create_dir_all(mog_dir.join("refs/heads"))?;
        std::fs::create_dir_all(mog_dir.join("refs/remotes"))?;

        std::fs::write(
            mog_dir.join("HEAD"),
            b"ref: refs/heads/main\n"
        )?;

        let root = path.canonicalize()?;
        let mogged = root.join(".mogged");
        if !mogged.exists() {
            std::fs::write(
                &mogged,
                "# .mogged: ignore rules (repo-root-relative)\n\
# Lines ending with / ignore a directory prefix.\n\
# * and ? are supported.\n\
\n\
.mog/\n\
.git/\n\
target/\n\
.idea/\n\
*.swp\n\
*.tmp\n"
            )?;
        }

        Ok(Self {
            ignore: Ignore::load(&root)?,
            root,
            storage: Storage::new(&mog_dir)?,
            object_cache: EncodedCache::default(),
            stores: Stores::default(),
        })
    }

    #[inline]
    pub fn open(path: &Path) -> Result<Self> {
        let mog_dir = path.join(".mog");

        if !mog_dir.exists() {
            bail!("not a mog repository");
        }

        let root = path.canonicalize()?;
        Ok(Self {
            ignore: Ignore::load(&root)?,
            root,
            storage: Storage::new(&mog_dir)?,
            object_cache: EncodedCache::default(),
            stores: Stores::default()
        })
    }

    /// Read object by hash; decode into stores and return Object(id). Uses 1MB encoded-bytes cache.
    #[inline]
    pub fn read_object(&mut self, hash: &Hash) -> Result<Object> {
        if let Some(cached) = self.object_cache.get(hash) {
            return decode_object_into_stores(
                cached,
                &mut self.stores,
            );
        }
        let data = self.storage.read(hash)?;
        let obj = decode_object_into_stores(
            &data,
            &mut self.stores
        )?;
        self.object_cache.insert(*hash, data);
        Ok(obj)
    }

    /// Encode from stores, hash, push to storage. Returns hash.
    #[inline]
    pub fn write_object(&mut self, object: Object) -> Hash {
        let hash = object_hash(object, &self.stores);

        let mut buf = Vec::new();
        encode_object_into(object, &self.stores, &mut buf);

        self.storage.write(hash, buf);

        hash
    }

    #[inline]
    pub fn read_ref(&self, refname: &str) -> Result<Hash> {
        let path = self.root.join(".mog").join(refname);
        let content = std::fs::read_to_string(path)?;
        hex_to_hash(content.trim())
    }

    #[inline]
    pub fn write_ref(&self, refname: &str, hash: &Hash) -> Result<()> {
        let path = self.root.join(".mog").join(refname);

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
        let head = std::fs::read_to_string(self.root.join(".mog/HEAD"))?;
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
        let head = std::fs::read_to_string(self.root.join(".mog/HEAD"))?;
        let head = head.trim();

        if let Some(refpath) = head.strip_prefix("ref: ") {
            let branch = refpath
                .trim()
                .strip_prefix("refs/heads/")
                .map(ToString::to_string);
            Ok(branch)
        } else {
            Ok(None) // detached
        }
    }

    /// Resolve branch or hex to (`commit_hash`, `CommitId`).
    pub fn resolve_to_commit(&mut self, target: &str) -> Result<(Hash, CommitId)> {
        let branch_ref = format!("refs/heads/{target}");
        let branch_path = self.root.join(".mog").join(&branch_ref);

        let hash = if branch_path.exists() {
            self.read_ref(&branch_ref)?
        } else {
            hex_to_hash(target)?
        };

        let obj = self.read_object(&hash)?;
        let commit_id = obj.try_as_commit_id()?;
        Ok((hash, commit_id))
    }

    /// Walk commit graph from start, collecting reachable hashes.
    pub fn reachable_commits(&mut self, start: &Hash) -> Xxh3HashSet<Hash> {
        let mut visited = Xxh3HashSet::default();
        let mut stack = vec![*start];

        while let Some(hash) = stack.pop() {
            if visited.contains(&hash) {
                continue;
            }
            visited.insert(hash);

            if let Ok(obj) = self.read_object(&hash) {
                if let Ok(id) = obj.try_as_commit_id() {
                    stack.extend(self.commit.get_parents(id));
                }
            }
        }

        visited
    }

    /// Walk tree at `tree_hash` following path; return (Object, `entry_hash`).
    pub fn walk_tree_path(&mut self, tree_hash: &Hash, path: &str) -> Result<(Object, Hash)> {
        let obj = self.read_object(tree_hash)?;
        let mut current_id = obj.try_as_tree_id()?;

        let components: Vec<&str> = path
            .trim_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();

        if components.is_empty() {
            bail!("empty path");
        }

        for &component in &components[..components.len() - 1] {
            let hash = self.tree
                .find_entry(current_id, component)
                .ok_or_else(|| anyhow::anyhow!("path not found: '{component}'"))?;
            let obj = self.read_object(&hash)?;
            current_id = obj.try_as_tree_id()?;
        }

        let last = components[components.len() - 1];
        let hash = self.tree
            .find_entry(current_id, last)
            .ok_or_else(|| anyhow::anyhow!("path not found: '{last}'"))?;

        let obj = self.read_object(&hash)?;
        Ok((obj, hash))
    }
}
