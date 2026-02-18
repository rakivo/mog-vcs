use crate::commit::{CommitPayloadOwned, CommitPayloadRef};
// SoA stores. ID = index into flat arrays.
use crate::hash::Hash;
use crate::object::{encode_blob_into, Object, ObjectTag};
use crate::tree::{TreeEntry, TreePayloadOwned, TreePayloadRef};
use crate::wire::{Decode, Encode, ReadCursor, WriteCursor};
use cranelift_entity::{entity_impl, EntityRef};
use anyhow::{Result, bail};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlobId(u32);
entity_impl!(BlobId, "blob");

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TreeId(u32);
entity_impl!(TreeId, "tree");

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommitId(u32);
entity_impl!(CommitId, "commit");

#[derive(Default)]
pub struct BlobStore {
    pub lengths: Vec<u32>,
    pub offsets: Vec<u32>,
    pub data: Vec<u8>,
}

impl BlobStore {
    #[inline]
    pub fn push(&mut self, bytes: &[u8]) -> BlobId {
        let id = BlobId::new(self.lengths.len());
        self.offsets.push(self.data.len() as u32);
        self.lengths.push(bytes.len() as u32);
        self.data.extend_from_slice(bytes);
        id
    }

    #[inline]
    pub fn get(&self, id: BlobId) -> &[u8] {
        let i = id.index();
        let start = self.offsets[i] as usize;
        let len = self.lengths[i] as usize;
        &self.data[start..start + len]
    }
}

#[derive(Default)]
pub struct TreeStore {
    pub entry_start: Vec<u32>,
    pub entry_end: Vec<u32>,
    pub modes: Vec<u32>,
    pub hashes: Vec<Hash>,
    pub name_offsets: Vec<u32>,
    pub name_end: Vec<u32>,
    pub names_blob: Vec<u8>,
}

impl TreeStore  {
    pub fn extend(&mut self, entries: &[TreeEntry]) -> TreeId {
        let id = TreeId::new(self.entry_start.len());
        let start = self.modes.len() as u32;
        self.entry_start.push(start);
        for TreeEntry { mode, hash, name } in entries {
            self.modes.push(*mode);
            self.hashes.push(*hash);
            self.name_offsets.push(self.names_blob.len() as u32);
            self.names_blob.extend_from_slice(name.as_bytes());
        }
        self.name_end.push(self.names_blob.len() as u32);
        self.entry_end.push(self.modes.len() as u32);
        id
    }

    #[inline]
    pub fn entry_count(&self, id: TreeId) -> usize {
        let i = id.index();
        (self.entry_end[i] - self.entry_start[i]) as usize
    }

    #[inline]
    pub fn get_entry(&self, id: TreeId, j: usize) -> TreeEntry {
        let i = id.index();
        let base = self.entry_start[i] as usize;
        let idx = base + j;
        let mode = self.modes[idx];
        let hash = self.hashes[idx];
        let start = self.name_offsets[idx] as usize;
        let end = if idx + 1 < self.entry_end[i] as usize {
            self.name_offsets[idx + 1] as usize
        } else {
            self.name_end[i] as usize
        };
        let name = std::str::from_utf8(&self.names_blob[start..end]).expect("utf8");
        TreeEntry { hash, mode, name: name.into() }
    }

    pub fn find_entry(&self, id: TreeId, name: &str) -> Option<Hash> {
        let n = self.entry_count(id);
        for j in 0..n {
            let TreeEntry { hash, name: entry_name, .. } = self.get_entry(id, j);
            if entry_name.as_ref() == name {
                return Some(hash);
            }
        }
        None
    }
}

#[derive(Default)]
pub struct CommitStore {
    pub tree: Vec<Hash>,
    pub parent_count: Vec<u32>,
    pub parent_start: Vec<u32>,
    pub parents: Vec<Hash>,
    pub timestamp: Vec<i64>,
    pub author_start: Vec<u32>,
    pub author_len: Vec<u32>,
    pub message_start: Vec<u32>,
    pub message_len: Vec<u32>,
    pub strings: Vec<u8>,
}

impl CommitStore {
    pub fn push(&mut self, tree: Hash, parents: &[Hash], timestamp: i64, author: &str, message: &str) -> CommitId {
        let id = CommitId::new(self.tree.len());
        self.tree.push(tree);
        self.parent_count.push(parents.len() as u32);
        self.parent_start.push(self.parents.len() as u32);
        self.parents.extend_from_slice(parents);
        self.timestamp.push(timestamp);
        self.author_start.push(self.strings.len() as u32);
        self.strings.extend_from_slice(author.as_bytes());
        self.author_len.push(author.len() as u32);
        self.message_start.push(self.strings.len() as u32);
        self.strings.extend_from_slice(message.as_bytes());
        self.message_len.push(message.len() as u32);
        id
    }

    pub fn push_payload_owned(&mut self, p: &CommitPayloadOwned) -> CommitId {
        self.push(p.tree, &p.parents, p.timestamp, &p.author, &p.message)
    }

    #[inline]
    pub fn get_tree(&self, id: CommitId) -> Hash {
        self.tree[id.index()]
    }

    #[inline]
    pub fn get_parents(&self, id: CommitId) -> &[Hash] {
        let i = id.index();
        let start = self.parent_start[i] as usize;
        let count = self.parent_count[i] as usize;
        &self.parents[start..start + count]
    }

    #[inline]
    pub fn get_timestamp(&self, id: CommitId) -> i64 {
        self.timestamp[id.index()]
    }

    #[inline]
    pub fn get_author(&self, id: CommitId) -> &str {
        let i = id.index();
        let start = self.author_start[i] as usize;
        let len = self.author_len[i] as usize;
        std::str::from_utf8(&self.strings[start..start + len]).expect("utf8")
    }

    #[inline]
    pub fn get_message(&self, id: CommitId) -> &str {
        let i = id.index();
        let start = self.message_start[i] as usize;
        let len = self.message_len[i] as usize;
        std::str::from_utf8(&self.strings[start..start + len]).expect("utf8")
    }
}

#[inline]
pub fn blob_encode_and_hash(store: &BlobStore, id: BlobId, buf: &mut Vec<u8>) -> Hash {
    encode_blob_into(store.get(id), buf);
    blake3::hash(buf).into()
}

/// Encode Object (id) from stores into buf. Same on-disk format as before.
pub fn encode_object_into(
    obj: Object,
    blob: &BlobStore,
    tree: &TreeStore,
    commit: &CommitStore,
    buf: &mut Vec<u8>,
) {
    buf.clear();
    buf.extend_from_slice(b"VX01");
    match obj {
        Object::Blob(id) => {
            buf.push(ObjectTag::Blob.as_byte());
            let mut w = WriteCursor::new(buf);
            let data = blob.get(id);
            w.write_u64(data.len() as u64);
            w.write_slice(data);
        }
        Object::Tree(id) => {
            buf.push(ObjectTag::Tree.as_byte());
            TreePayloadRef::new(tree, id).view().encode(&mut WriteCursor::new(buf));
        }
        Object::Commit(id) => {
            buf.push(ObjectTag::Commit.as_byte());
            CommitPayloadRef::new(commit, id).view().encode(&mut WriteCursor::new(buf));
        }
    }
}

/// Decode object bytes into stores; return Object(id).
pub fn decode_into_stores(
    data: &[u8],
    blob: &mut BlobStore,
    tree: &mut TreeStore,
    commit: &mut CommitStore,
) -> Result<Object> {
    if data.len() < 5 {
        bail!("data too short");
    }
    if &data[0..4] != b"VX01" {
        bail!("invalid magic");
    }
    let tag = data[4];

    let mut r = ReadCursor::new(&data[5..]);
    match ObjectTag::from_byte(tag) {
        Some(ObjectTag::Blob) => {
            let len = r.read_u64()? as usize;
            let bytes = r.read_bytes(len)?;
            let id = blob.push(bytes);
            Ok(Object::Blob(id))
        }
        Some(ObjectTag::Tree) => {
            let p = TreePayloadOwned::decode(&mut r)?;
            let id = tree.extend(&p.entries);
            Ok(Object::Tree(id))
        }
        Some(ObjectTag::Commit) => {
            let p = CommitPayloadOwned::decode(&mut r)?;
            let id = commit.push_payload_owned(&p);
            Ok(Object::Commit(id))
        }
        None => bail!("unknown object type"),
    }
}

/// Hash of object encoded from stores.
pub fn object_hash(obj: Object, blob: &BlobStore, tree: &TreeStore, commit: &CommitStore) -> Hash {
    let mut buf = Vec::new();
    encode_object_into(obj, blob, tree, commit, &mut buf);
    blake3::hash(&buf).into()
}
