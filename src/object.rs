use crate::{hash::Hash, store::{BlobId, CommitId, TreeId}};

use anyhow::{bail, Result};

pub const MODE_FILE: u32 = 0o100_644;
pub const MODE_EXEC: u32 = 0o100_755;
pub const MODE_DIR:  u32 = 0o040_000;
#[allow(unused)]
pub const MODE_LINK: u32 = 0o120_000;

/// Copyable. Data lives in stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Object {
    Blob(BlobId),
    Tree(TreeId),
    Commit(CommitId),
}

impl Object {
    #[inline]
    pub fn try_as_commit_id(self) -> Result<CommitId> {
        match self {
            Self::Commit(c) => Ok(c),
            _ => bail!("not a commit"),
        }
    }

    #[inline]
    pub fn try_as_tree_id(self) -> Result<TreeId> {
        match self {
            Self::Tree(t) => Ok(t),
            _ => bail!("not a tree"),
        }
    }

    #[inline]
    #[allow(unused)]
    pub fn try_as_blob_id(self) -> Result<BlobId> {
        match self {
            Self::Blob(b) => Ok(b),
            _ => bail!("not a blob"),
        }
    }
}

/// Object type tag. Single place for on-disk tag bytes.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectTag {
    Blob = 0x1,
    Tree = 0x2,
    Commit = 0x4,
}

impl ObjectTag {
    #[inline]
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x1 => Some(Self::Blob),
            0x2 => Some(Self::Tree),
            0x4 => Some(Self::Commit),
            _ => None,
        }
    }

    #[inline]
    pub fn as_byte(self) -> u8 {
        self as u8
    }
}

/// Encode raw bytes as on-disk blob (VX01 + type + len + data).
#[inline]
pub fn encode_blob_into(data: &[u8], buf: &mut Vec<u8>) {
    buf.clear();
    buf.reserve(4 + 1 + 8 + data.len());
    buf.extend_from_slice(b"VX01");
    buf.push(ObjectTag::Blob.as_byte());
    buf.extend_from_slice(&(data.len() as u64).to_le_bytes());
    buf.extend_from_slice(data);
}

#[derive(Debug, Clone)]
pub struct Tree {
    pub modes:        Box<[u32]>,
    pub hashes:       Box<[Hash]>,
    pub name_offsets: Box<[u32]>,
    pub names_blob:   Box<[u8]>,
}

pub struct TreeIterator<'tree> {
    pub tree: &'tree Tree,
    pub index: usize
}

#[derive(Debug)]
pub struct TreeEntryRef<'tree> {
    // align 8
    pub hash: &'tree Hash,
    pub name: &'tree str,

    pub mode: u32,
}

impl<'tree> Iterator for TreeIterator<'tree> {
    type Item = TreeEntryRef<'tree>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.tree.count() {
            return None;
        }

        let e = TreeEntryRef {
            mode: self.tree.modes[self.index],
            hash: &self.tree.hashes[self.index],
            name: self.tree.get_name(self.index)
        };

        self.index += 1;

        Some(e)
    }
}

impl<'tree> IntoIterator for &'tree Tree {
    type Item = TreeEntryRef<'tree>;
    type IntoIter = TreeIterator<'tree>;

    fn into_iter(self) -> Self::IntoIter {
        TreeIterator { tree: self, index: 0 }
    }
}

impl Tree {
    #[inline]
    pub fn iter(&self) -> TreeIterator<'_> {
        TreeIterator { tree: self, index: 0 }
    }

    #[inline]
    pub fn count(&self) -> usize {
        self.modes.len()
    }

    // Find a named entry in a tree, returning its hash
    #[inline]
    pub fn find_in_tree<'a>(&'a self, name: &str) -> Option<&'a Hash> {
        self.into_iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.hash)
    }

    #[inline]
    pub fn get_name(&self, index: usize) -> &str {
        let start = self.name_offsets[index] as usize;
        let end = if index + 1 < self.count() {
            self.name_offsets[index + 1] as usize
        } else {
            self.names_blob.len()
        };

        std::str::from_utf8(&self.names_blob[start..end])
            .expect("invalid utf8 in tree name")
    }
}
