use crate::hash::Hash;
use crate::commit::{CommitPayloadOwned, CommitPayloadRef};
use crate::object::{Object, ObjectTag};
use crate::tree::{TreeEntry, TreeEntryRef, TreePayloadOwned, TreePayloadRef};
use crate::util::str_from_utf8_data_shouldve_been_valid_or_we_got_hacked;
use crate::wire::{Decode, Encode, ReadCursor, WriteCursor};

use anyhow::{Result, bail};
use cranelift_entity::{entity_impl, EntityRef};

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
pub struct Stores {
    pub blob: BlobStore,
    pub tree: TreeStore,
    pub commit: CommitStore,
}

impl Stores {
    #[inline]
    pub fn decode_and_push_object(&mut self, data: &[u8]) -> Result<Object> {
        // @Cleanup

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
                let id = self.blob.push(bytes);
                Ok(Object::Blob(id))
            }
            Some(ObjectTag::Tree) => {
                let p = TreePayloadOwned::decode(&mut r)?;
                let id = self.tree.push(&p.entries);
                Ok(Object::Tree(id))
            }
            Some(ObjectTag::Commit) => {
                let p = CommitPayloadOwned::decode(&mut r)?;
                let id = self.commit.push_payload_owned(&p);
                Ok(Object::Commit(id))
            }
            None => bail!("unknown object type"),
        }
    }

    #[inline]
    pub fn encode_object_into(&self, object: Object, into: &mut Vec<u8>) {
        // @Cleanup

        into.clear();
        into.extend_from_slice(b"VX01");
        match object {
            Object::Blob(id) => {
                into.push(ObjectTag::Blob.as_byte());
                let mut w = WriteCursor::new(into);
                let data = self.blob.get(id);
                w.write_u64(data.len() as u64);
                w.write_slice(data);
            }
            Object::Tree(id) => {
                into.push(ObjectTag::Tree.as_byte());
                TreePayloadRef::new(&self.tree, id).view().encode(&mut WriteCursor::new(into));
            }
            Object::Commit(id) => {
                into.push(ObjectTag::Commit.as_byte());
                CommitPayloadRef::new(&self.commit, id).view().encode(&mut WriteCursor::new(into));
            }
        }
    }
}

#[derive(Default)]
pub struct BlobStore {
    pub len: Vec<u32>,
    pub start: Vec<u32>,
    pub data: Vec<u8>,
}

impl BlobStore {
    #[inline]
    pub fn push(&mut self, bytes: &[u8]) -> BlobId {
        let id = BlobId::new(self.len.len());
        self.start.push(self.data.len() as u32);
        self.len.push(bytes.len() as u32);
        self.data.extend_from_slice(bytes);
        id
    }

    #[inline]
    #[must_use]
    pub fn get(&self, id: BlobId) -> &[u8] {
        let i = id.index();
        let start = self.start[i] as usize;
        let len = self.len[i] as usize;
        &self.data[start..start + len]
    }
}

#[derive(Default)]
pub struct TreeStore {
    pub entry_start: Vec<u32>, // Into names_blob
    pub entry_len: Vec<u32>,

    pub modes: Vec<u32>,

    pub hashes: Vec<Hash>,

    pub name_start: Vec<u32>, // Into names_blob
    pub name_len: Vec<u32>,

    pub names_blob: Vec<u8>,
}

impl TreeStore {
    #[inline]
    pub fn push(&mut self, entries: &[TreeEntry]) -> TreeId {
        let id = TreeId::new(self.entry_start.len());

        self.entry_start.push(self.modes.len() as u32);
        self.entry_len.push(entries.len() as u32);

        for TreeEntry { mode, hash, name } in entries {
            self.modes.push(*mode);

            self.hashes.push(*hash);

            self.name_start.push(self.names_blob.len() as u32);
            self.name_len.push(name.len() as u32);

            self.names_blob.extend_from_slice(name.as_bytes());
        }

        id
    }

    #[inline]
    #[must_use]
    pub fn entry_count(&self, id: TreeId) -> usize {
        self.entry_len[id.index()] as usize
    }

    #[inline]
    #[must_use]
    pub fn get_entry_ref(&self, id: TreeId, j: usize) -> TreeEntryRef<'_> {
        let idx = self.entry_start[id.index()] as usize + j;
        let mode = self.modes[idx];
        let hash = self.hashes[idx];
        let start = self.name_start[idx] as usize;
        let end = start + self.name_len[idx] as usize;
        let name = str_from_utf8_data_shouldve_been_valid_or_we_got_hacked(&self.names_blob[start..end]);
        TreeEntryRef { hash, mode, name }
    }

    #[inline]
    #[must_use]
    pub fn get_entry(&self, id: TreeId, j: usize) -> TreeEntry {
        self.get_entry_ref(id, j).into()
    }

    #[must_use]
    #[inline]
    pub fn find_entry(&self, id: TreeId, name: &str) -> Option<Hash> {
        let n = self.entry_count(id);
        for j in 0..n {
            let TreeEntryRef { hash, name: entry_name, .. } = self.get_entry_ref(id, j);
            if entry_name == name {
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

    pub author_start: Vec<u32>, // Into strings
    pub author_len: Vec<u32>,

    pub message_start: Vec<u32>, // Into strings
    pub message_len: Vec<u32>,

    pub strings: Vec<u8>,
}

impl CommitStore {
    #[inline]
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

    #[inline]
    pub fn push_payload_owned(&mut self, p: &CommitPayloadOwned) -> CommitId {
        self.push(p.tree, &p.parents, p.timestamp, &p.author, &p.message)
    }

    #[inline]
    #[must_use]
    pub fn get_tree(&self, id: CommitId) -> Hash {
        self.tree[id.index()]
    }

    #[inline]
    #[must_use]
    pub fn get_parents(&self, id: CommitId) -> &[Hash] {
        let i = id.index();
        let start = self.parent_start[i] as usize;
        let count = self.parent_count[i] as usize;
        &self.parents[start..start + count]
    }

    #[inline]
    #[must_use]
    pub fn get_timestamp(&self, id: CommitId) -> i64 {
        self.timestamp[id.index()]
    }

    #[inline]
    #[must_use]
    pub fn get_author(&self, id: CommitId) -> &str {
        let i = id.index();
        let start = self.author_start[i] as usize;
        let len = self.author_len[i] as usize;
        str_from_utf8_data_shouldve_been_valid_or_we_got_hacked(&self.strings[start..start + len])
    }

    #[inline]
    #[must_use]
    pub fn get_message(&self, id: CommitId) -> &str {
        let i = id.index();
        let start = self.message_start[i] as usize;
        let len = self.message_len[i] as usize;
        str_from_utf8_data_shouldve_been_valid_or_we_got_hacked(&self.strings[start..start + len])
    }
}
