use crate::hash::Hash;
use crate::store::{TreeId, TreeStore};
use crate::wire::{Decode, Encode, ReadCursor, WriteCursor};

use anyhow::Result;

#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub mode: u32,
    pub hash: Hash,
    pub name: Box<str>,
}

crate::payload_triple! {
    owned TreePayloadOwned {
        entries: Box<[TreeEntry]>,
    }
    view TreePayloadView<'a> {
        entries: &'a [TreeEntry],
    }
    ref TreePayloadRef<'a> {
        store: &'a TreeStore,
        id: TreeId,
    }
    view_from_owned(o) {
        TreePayloadView { entries: &o.entries }
    }
    view_from_ref(r) {
        let n = r.store.entry_count(r.id);
        let mut entries = Vec::with_capacity(n);
        for j in 0..n {
            entries.push(r.store.get_entry(r.id, j));
        }
        TreePayloadView { entries: Box::leak(entries.into_boxed_slice()) }
    }
}

impl TreePayloadOwned {
    pub fn new(entries: Box<[TreeEntry]>) -> Self {
        Self { entries }
    }
}

impl Decode for TreePayloadOwned {
    fn decode(r: &mut ReadCursor<'_>) -> Result<Self> {
        let count = r.read_u32()? as usize;
        let mut modes = Vec::with_capacity(count);
        for _ in 0..count {
            modes.push(r.read_u32()?);
        }
        let mut hashes = Vec::with_capacity(count);
        for _ in 0..count {
            hashes.push(r.read_hash()?);
        }
        let mut name_offsets = Vec::with_capacity(count + 1);
        for _ in 0..count {
            name_offsets.push(r.read_u32()? as usize);
        }
        let names_len = r.read_u32()? as usize;
        let names_blob = r.read_bytes(names_len)?;
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let start = name_offsets[i];
            let end = if i + 1 < count { name_offsets[i + 1] } else { names_len };
            let name = std::str::from_utf8(&names_blob[start..end])?;
            entries.push(TreeEntry {
                mode: modes[i],
                hash: hashes[i],
                name: name.into(),
            });
        }
        Ok(TreePayloadOwned::new(entries.into_boxed_slice()))
    }
}

impl<'a> TreePayloadRef<'a> {
    pub fn new(store: &'a TreeStore, id: TreeId) -> Self {
        Self { store, id }
    }
}

impl Encode for TreePayloadView<'_> {
    fn encode(&self, w: &mut WriteCursor<'_>) {
        let n = self.entries.len();
        w.write_u32(n as u32);
        for entry in self.entries {
            w.write_u32(entry.mode);
        }
        for entry in self.entries {
            w.write_hash(&entry.hash);
        }
        let mut name_start = 0u32;
        for entry in self.entries {
            w.write_u32(name_start);
            name_start += entry.name.len() as u32;
        }
        w.write_u32(name_start);
        for entry in self.entries {
            w.write_slice(entry.name.as_bytes());
        }
    }
}
