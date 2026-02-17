use std::collections::HashSet;

use xxhash_rust::xxh3::Xxh3DefaultBuilder;

pub type Xxh3HashSet<K> = HashSet<K, Xxh3DefaultBuilder>;

pub fn make_xxh3_hashset<K>() -> HashSet<K, Xxh3DefaultBuilder> {
    HashSet::with_hasher(Xxh3DefaultBuilder::new())
}

/// `std::vec::Vec::into_boxed_slice` takes CPU cycles to shrink
/// itself to the `.len`, this function does not shrink and saves
/// us some CPU cycles
#[inline]
#[must_use]
pub fn vec_into_boxed_slice_noshrink<T>(mut v: Vec<T>) -> Box<[T]> {
    let len = v.len();
    let ptr = v.as_mut_ptr();

    core::mem::forget(v);

    unsafe { Box::from_raw(core::ptr::slice_from_raw_parts_mut(ptr, len)) }
}
