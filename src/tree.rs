use crate::hash::Hash;
use crate::store::{TreeId, TreeStore};
use crate::util::str_from_utf8_data_shouldve_been_valid_or_we_got_hacked;
use crate::wire::{Decode, Encode, ReadCursor, WriteCursor};

use anyhow::Result;

#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub mode: u32,
    pub hash: Hash,
    pub name: Box<str>,
}

#[derive(Debug)]
pub struct TreeEntryRef<'a> {
    // align 8
    pub hash: Hash,
    pub name: &'a str,

    pub mode: u32,
}

impl TreeEntryRef<'_> {
    #[inline]
    pub fn into_entry(self) -> TreeEntry {
        TreeEntry {
            name: self.name.into(),
            hash: self.hash,
            mode: self.mode
        }
    }
}

impl Into<TreeEntry> for TreeEntryRef<'_> {
    fn into(self) -> TreeEntry {
        self.into_entry()
    }
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
    #[must_use]
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
            let name = str_from_utf8_data_shouldve_been_valid_or_we_got_hacked(&names_blob[start..end]);
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
    #[must_use]
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
