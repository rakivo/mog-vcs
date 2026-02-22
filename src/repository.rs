use crate::cache::ObjectCache;
use crate::ignore::Ignore;
use crate::storage::{MogStorage, Storage};
use crate::object::{encode_blob_and_hash, hash_object, Object};
use crate::storage_mock::MockStorage;
use crate::store::{CommitId, Stores};
use crate::hash::{Hash, hash_to_hex, hex_to_hash};
use crate::tree::TreeEntry;
use crate::util::Xxh3HashSet;

use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

pub struct Repository<S: MogStorage = Storage> {
    pub root: Box<Path>,
    pub storage: S,
    pub ignore: Ignore,
    pub object_cache: ObjectCache,
    pub stores: Stores
}

impl<S: MogStorage> Deref for Repository<S> {
    type Target = Stores;
    #[inline]
    fn deref(&self) -> &Self::Target { &self.stores }
}

impl<S: MogStorage> DerefMut for Repository<S> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.stores }
}

impl Repository<Storage> {
    #[inline]
    pub fn init(path: &Path) -> Result<Self> {
        let mog_dir = path.join(".mog");

        std::fs::create_dir_all(&mog_dir)?;
        std::fs::create_dir_all(mog_dir.join("refs/heads"))?;
        std::fs::create_dir_all(mog_dir.join("refs/remotes"))?;

        std::fs::write(
            mog_dir.join("HEAD"),
            b"ref: refs/heads/main\n"
        )?;

        let root = path.canonicalize()?.into_boxed_path();
        let mogged = root.join(".mogged");
        if !mogged.exists() {
            std::fs::write(
                &mogged,
                "\
# .mogged: ignore rules (repo-root-relative)\n\
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
            object_cache: ObjectCache::default(),
            stores: Stores::default(),
        })
    }

    #[inline]
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let mog_dir = path.join(".mog");

        if !mog_dir.exists() {
            bail!("not a mog repository");
        }

        let root = path.canonicalize()?.into_boxed_path();
        Ok(Self {
            ignore: Ignore::load(&root)?,
            root,
            storage: Storage::new(&mog_dir)?,
            object_cache: ObjectCache::default(),
            stores: Stores::default()
        })
    }
}

impl Repository<MockStorage> {
    #[inline]
    #[must_use]
    pub fn new_mock() -> Self {
        Self {
            root:         PathBuf::from("/mock").into(),
            storage:      MockStorage::new(),
            ignore:       Ignore::empty(),
            object_cache: ObjectCache::default(),
            stores:       Stores::default(),
        }
    }
}

impl<S: MogStorage> Repository<S> {
    #[inline]
    pub fn read_object(&mut self, hash: &Hash) -> Result<Object> {
        if let Some(cached) = self.object_cache.get(hash) {
            return self.stores.decode_and_push_object(cached);
        }
        let data = self.storage.read(hash)?;
        let object = self.stores.decode_and_push_object(data)?;
        self.object_cache.insert(*hash, data.to_vec()); // @Clone
        Ok(object)
    }

    #[inline]
    pub fn read_object_without_touching_cache(&mut self, hash: &Hash) -> Result<Object> {
        let data = self.storage.read(hash)?;
        let object = self.stores.decode_and_push_object(data)?; // @Incomplete: Don't push to stores
        Ok(object)
    }

    #[inline]
    pub fn read_tree_entries_without_touching_cache(&mut self, hash: &Hash) -> Result<Box<[TreeEntry]>> {
        let data = self.storage.read(hash)?;
        crate::object::decode_tree_entries(data)
    }

    #[inline]
    pub fn read_blob_bytes_without_touching_cache(&mut self, hash: &Hash) -> Result<&[u8]> {
        let data = self.storage.read(hash)?;
        crate::object::decode_blob_bytes(data)
    }

    #[inline]
    pub fn with_blob_bytes_without_touching_cache_and_evict_the_pages<T, E: Into<anyhow::Error>>(
        &mut self,
        hash: &Hash,
        callback: impl FnOnce(&Self, &[u8]) -> std::result::Result<T, E>
    ) -> Result<T> {
        let raw = self.storage.read(hash)?;
        let data = crate::object::decode_blob_bytes(raw)?;
        let result = callback(self, data);

        Storage::evict_pages(raw);

        result.map_err(|e| e.into())
    }

    #[inline]
    pub fn read_blob_bytes_without_touching_stores(&mut self, hash: &Hash) -> Result<&[u8]> {
        if !self.object_cache.contains(hash) {
            let data = self.storage.read(hash)?;
            self.object_cache.insert(*hash, data.to_vec());
        }
        let cached = self.object_cache.get(hash).unwrap();
        crate::object::decode_blob_bytes(cached)
    }

    /// Encode from stores, hash, push to storage. Returns hash.
    #[inline]
    pub fn write_object(&mut self, object: Object) -> Hash {
        let hash = hash_object(object, &self.stores);

        let mut buf = Vec::new();
        self.encode_object_into(object, &mut buf);

        self.storage.write(hash, buf);

        hash
    }

    #[inline]
    pub fn write_blob(&mut self, data: &[u8]) -> Hash {
        let mut buf = Vec::new();
        let hash = encode_blob_and_hash(data, &mut buf);
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
            let hash_str = std::fs::read_to_string(
                self.root.join(".mog").join(refpath)
            )?.trim().to_string();
            return hex_to_hash(&hash_str);
        }

        hex_to_hash(head)
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
    #[inline]
    pub fn resolve_to_commit(&mut self, target: &str) -> Result<(Hash, CommitId)> {
        let branch_ref = format!("refs/heads/{target}");
        let branch_path = self.root.join(".mog").join(&branch_ref);

        let hash = if branch_path.exists() {
            self.read_ref(&branch_ref)?
        } else {
            hex_to_hash(target)?
        };

        let object = self.read_object(&hash)?;
        let commit_id = object.try_as_commit_id()?;
        Ok((hash, commit_id))
    }

    /// Walk commit graph from start, collecting reachable hashes.
    #[inline]
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
        let object = self.read_object(tree_hash)?;
        let mut current_id = object.try_as_tree_id()?;

        let components = path
            .trim_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();

        if components.is_empty() {
            bail!("empty path");
        }

        for &component in &components[..components.len() - 1] {
            let hash = self.tree
                .find_entry(current_id, component)
                .ok_or_else(|| anyhow::anyhow!("path not found: '{component}'"))?;

            let object = self.read_object(&hash)?;
            current_id = object.try_as_tree_id()?;
        }

        let last = components[components.len() - 1];
        let hash = self.tree
            .find_entry(current_id, last)
            .ok_or_else(|| anyhow::anyhow!("path not found: '{last}'"))?;

        let object = self.read_object(&hash)?;
        Ok((object, hash))
    }
}
