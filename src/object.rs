use crate::{hash::Hash, store::{BlobId, CommitId, TreeId}, tree::{TreeEntry, TreeEntryRef, TreePayloadOwned}, util::str_from_utf8_data_shouldve_been_valid_or_we_got_hacked, wire::{Decode, ReadCursor}};

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
    #[must_use]
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x1 => Some(Self::Blob),
            0x2 => Some(Self::Tree),
            0x4 => Some(Self::Commit),
            _ => None,
        }
    }

    #[inline]
    #[must_use]
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

#[inline]
pub fn decode_blob_bytes(data: &[u8]) -> Result<&[u8]> {
    if data.len() < 5 { bail!("data too short"); }

    if &data[0..4] != b"VX01" { bail!("invalid magic"); }
    if data[4] != ObjectTag::Blob as u8 { bail!("not a blob"); }

    let mut r = ReadCursor::new(&data[5..]);
    let len = r.read_u64()? as usize;

    r.read_bytes(len)
}

/// Encode raw bytes as on-disk blob (VX01 + type + len + data).
#[inline]
pub fn hash_blob(data: &[u8]) -> Hash {
    let mut hasher = blake3::Hasher::new();

    hasher.update(b"VX01");
    hasher.update(&[ObjectTag::Blob.as_byte()]);
    hasher.update(&(data.len() as u64).to_le_bytes());
    hasher.update(data);

    hasher.finalize().into()
}

#[inline]
pub fn decode_tree_entries(data: &[u8]) -> Result<Box<[TreeEntry]>> {
    if data.len() < 5 { bail!("data too short"); }

    if &data[0..4] != b"VX01" { bail!("invalid magic"); }
    if data[4] != ObjectTag::Tree as u8 { bail!("not a tree"); }

    let mut r = ReadCursor::new(&data[5..]);
    let p = TreePayloadOwned::decode(&mut r)?;
    Ok(p.entries)
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

impl<'tree> Iterator for TreeIterator<'tree> {
    type Item = TreeEntryRef<'tree>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.tree.count() {
            return None;
        }

        let e = TreeEntryRef {
            mode: self.tree.modes[self.index],
            hash: self.tree.hashes[self.index],
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
    #[must_use]
    pub fn iter(&self) -> TreeIterator<'_> {
        TreeIterator { tree: self, index: 0 }
    }

    #[inline]
    #[must_use]
    pub fn count(&self) -> usize {
        self.modes.len()
    }

    // Find a named entry in a tree, returning its hash
    #[inline]
    #[must_use]
    pub fn find_in_tree<'a>(&'a self, name: &str) -> Option<Hash> {
        self.into_iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.hash)
    }

    #[inline]
    #[must_use]
    pub fn get_name(&self, index: usize) -> &str {
        let start = self.name_offsets[index] as usize;
        let end = if index + 1 < self.count() {
            self.name_offsets[index + 1] as usize
        } else {
            self.names_blob.len()
        };

        str_from_utf8_data_shouldve_been_valid_or_we_got_hacked(&self.names_blob[start..end])
    }
}
