use crate::object::Tree;
use crate::hash::Hash;

pub struct TreeBuilder {
    modes:        Vec<u32>,
    hashes:       Vec<Hash>,
    names_blob:   Vec<u8>,
    name_offsets: Vec<u32>,
}

impl TreeBuilder {
    pub fn new() -> Self {
        Self {
            modes: Vec::new(),
            hashes: Vec::new(),
            names_blob: Vec::new(),
            name_offsets: Vec::new(),
        }
    }

    pub fn add(&mut self, mode: u32, hash: Hash, name: &str) {
        self.modes.push(mode);
        self.hashes.push(hash);
        self.name_offsets.push(self.names_blob.len() as u32);
        self.names_blob.extend_from_slice(name.as_bytes());
    }

    pub fn build(self) -> Tree {
        Tree {
            modes: crate::util::vec_into_boxed_slice_noshrink(self.modes),
            hashes: crate::util::vec_into_boxed_slice_noshrink(self.hashes),
            name_offsets: crate::util::vec_into_boxed_slice_noshrink(self.name_offsets),
            names_blob: crate::util::vec_into_boxed_slice_noshrink(self.names_blob),
        }
    }
}
