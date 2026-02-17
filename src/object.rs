use crate::hash::Hash;

use std::ops::{Deref, DerefMut};

use anyhow::{Result, bail};
use smallvec::SmallVec;

pub const MODE_FILE: u32 = 0o100644;
pub const MODE_EXEC: u32 = 0o100755;
pub const MODE_DIR:  u32 = 0o040000;
pub const MODE_LINK: u32 = 0o120000;

pub const OBJECT_BLOB:   u8 = 0x1;
pub const OBJECT_TREE:   u8 = 0x2;
pub const OBJECT_COMMIT: u8 = 0x4;

#[derive(Debug, Clone)]
pub enum Object {
    Blob(Blob),
    Tree(Tree),
    Commit(Commit),
}

impl Object {
    #[inline]
    pub fn try_as_commit(&self) -> Result<&Commit> {
        match self {
            Self::Commit(c) => Ok(c),
            _ => bail!("not a commit!")
        }
    }

    #[inline]
    pub fn try_as_tree(&self) -> Result<&Tree> {
        match self {
            Self::Tree(t) => Ok(t),
            _ => bail!("not a tree!")
        }
    }

    #[inline]
    pub fn try_as_blob(&self) -> Result<&Blob> {
        match self {
            Self::Blob(b) => Ok(b),
            _ => bail!("not a blob!")
        }
    }

    #[inline]
    pub fn try_into_commit(self) -> Result<Commit> {
        match self {
            Self::Commit(c) => Ok(c),
            _ => bail!("not a commit!")
        }
    }

    #[inline]
    pub fn try_into_tree(self) -> Result<Tree> {
        match self {
            Self::Tree(t) => Ok(t),
            _ => bail!("not a tree!")
        }
    }

    #[inline]
    pub fn try_into_blob(self) -> Result<Blob> {
        match self {
            Self::Blob(b) => Ok(b),
            _ => bail!("not a blob!")
        }
    }

    #[inline]
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"VX01");

        match self {
            Object::Blob(blob) => {
                buf.push(OBJECT_BLOB);
                buf.extend_from_slice(&(blob.data.len() as u64).to_le_bytes());
                buf.extend_from_slice(&blob.data);
            }
            Object::Tree(tree) => {
                buf.push(OBJECT_TREE);
                tree.encode_into(&mut buf);
            }
            Object::Commit(commit) => {
                buf.push(OBJECT_COMMIT);
                commit.encode_into(&mut buf);
            }
        }

        buf
    }

    #[inline]
    pub fn decode(data: &[u8]) -> Result<Self> {
        if data.len() < 5 {
            bail!("data too short");
        }

        if &data[0..4] != b"VX01" {
            bail!("invalid magic");
        }

        match data[4] {
            0 => Ok(Object::Blob(Blob::decode(&data[5..])?)),
            1 => Ok(Object::Tree(Tree::decode(&data[5..])?)),
            2 => Ok(Object::Commit(Commit::decode(&data[5..])?)),
            _ => bail!("unknown object type"),
        }
    }

    #[inline]
    pub fn hash(&self) -> Hash {
        let encoded = self.encode();
        blake3::hash(&encoded).into()
    }
}

#[derive(Debug, Clone)]
pub struct Blob {
    pub data: Box<[u8]>,
}

impl Blob {
    #[inline]
    fn decode(data: &[u8]) -> Result<Self> {
        let len = u64::from_le_bytes(data[0..8].try_into()?) as usize;
        let data = crate::util::vec_into_boxed_slice_noshrink(data[8..8+len].to_vec());
        Ok(Blob { data })
    }
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

    fn encode_into(&self, buf: &mut Vec<u8>) {
        // Entry count
        buf.extend_from_slice(&(self.count() as u32).to_le_bytes());

        // Modes (SoA)
        for mode in &self.modes {
            buf.extend_from_slice(&mode.to_le_bytes());
        }

        // Hashes (SoA)
        for hash in &self.hashes {
            buf.extend_from_slice(hash);
        }

        // Name offsets (SoA)
        for offset in &self.name_offsets {
            buf.extend_from_slice(&offset.to_le_bytes());
        }

        // Names blob
        buf.extend_from_slice(&(self.names_blob.len() as u32).to_le_bytes());
        buf.extend_from_slice(&self.names_blob);
    }

    fn decode(data: &[u8]) -> Result<Self> {
        let mut cursor = 0;

        // Entry count
        let count = u32::from_le_bytes(data[cursor..cursor+4].try_into()?) as usize;
        cursor += 4;

        // Modes
        let mut modes = Vec::with_capacity(count);
        for _ in 0..count {
            let mode = u32::from_le_bytes(data[cursor..cursor+4].try_into()?);
            modes.push(mode);
            cursor += 4;
        }

        // Hashes
        let mut hashes = Vec::with_capacity(count);
        for _ in 0..count {
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&data[cursor..cursor+32]);
            hashes.push(hash);
            cursor += 32;
        }

        // Name offsets
        let mut name_offsets = Vec::with_capacity(count);
        for _ in 0..count {
            let offset = u32::from_le_bytes(data[cursor..cursor+4].try_into()?);
            name_offsets.push(offset);
            cursor += 4;
        }

        // Names blob
        let names_len = u32::from_le_bytes(data[cursor..cursor+4].try_into()?) as usize;
        cursor += 4;
        let names_blob = data[cursor..cursor+names_len].to_vec();

        Ok(Tree {
            modes: crate::util::vec_into_boxed_slice_noshrink(modes),
            hashes: crate::util::vec_into_boxed_slice_noshrink(hashes),
            name_offsets: crate::util::vec_into_boxed_slice_noshrink(name_offsets),
            names_blob: crate::util::vec_into_boxed_slice_noshrink(names_blob),
        })
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

#[derive(Debug, Clone)]
pub struct Commit {
    // align 8
    pub parents: SmallVec<[Hash; 1]>, // Usually only one parent!
    pub timestamp: i64,
    pub author: Box<str>,
    pub message: Box<str>,

    pub tree: Hash,
}

impl Commit {
    fn encode_into(&self, buf: &mut Vec<u8>) {
        // Tree hash
        buf.extend_from_slice(&self.tree);

        // Parent count + hashes
        buf.extend_from_slice(&(self.parents.len() as u32).to_le_bytes());
        for parent in &self.parents {
            buf.extend_from_slice(parent);
        }

        // Timestamp
        buf.extend_from_slice(&self.timestamp.to_le_bytes());

        // Author
        buf.extend_from_slice(&(self.author.len() as u32).to_le_bytes());
        buf.extend_from_slice(self.author.as_bytes());

        // Message
        buf.extend_from_slice(&(self.message.len() as u32).to_le_bytes());
        buf.extend_from_slice(self.message.as_bytes());
    }

    fn decode(data: &[u8]) -> Result<Self> {
        let mut cursor = 0;

        // Tree
        let mut tree = [0u8; 32];
        tree.copy_from_slice(&data[cursor..cursor+32]);
        cursor += 32;

        // Parents
        let parent_count = u32::from_le_bytes(data[cursor..cursor+4].try_into()?) as usize;
        cursor += 4;

        let mut parents = SmallVec::with_capacity(parent_count);
        for _ in 0..parent_count {
            let mut parent = [0u8; 32];
            parent.copy_from_slice(&data[cursor..cursor+32]);
            parents.push(parent);
            cursor += 32;
        }

        // Timestamp
        let timestamp = i64::from_le_bytes(data[cursor..cursor+8].try_into()?);
        cursor += 8;

        // Author
        let author_len = u32::from_le_bytes(data[cursor..cursor+4].try_into()?) as usize;
        cursor += 4;
        let author = String::from_utf8(data[cursor..cursor+author_len].to_vec())?.into_boxed_str();
        cursor += author_len;

        // Message
        let msg_len = u32::from_le_bytes(data[cursor..cursor+4].try_into()?) as usize;
        cursor += 4;
        let message = String::from_utf8(data[cursor..cursor+msg_len].to_vec())?.into_boxed_str();

        Ok(Commit {
            tree,
            parents,
            timestamp,
            author,
            message,
        })
    }
}
