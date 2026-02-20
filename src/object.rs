use crate::{hash::Hash, store::{BlobId, BlobStore, CommitId, Stores, TreeId}, tree::{TreeEntry, TreePayloadOwned}, wire::{Decode, ReadCursor}};

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

/// Hash of object encoded from stores.
#[inline]
#[must_use]
pub fn hash_object(object: Object, stores: &Stores) -> Hash {
    let mut buf = Vec::new();
    stores.encode_object_into(object, &mut buf);
    blake3::hash(&buf).into()
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
pub fn encode_blob_and_hash(data: &[u8], buf: &mut Vec<u8>) -> Hash {
    encode_blob_into(data, buf);
    blake3::hash(buf).into()
}

#[inline]
pub fn encode_blob_id_and_hash(store: &BlobStore, id: BlobId, buf: &mut Vec<u8>) -> Hash {
    encode_blob_into(store.get(id), buf);
    blake3::hash(buf).into()
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
#[must_use]
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
