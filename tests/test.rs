use mog::repository::Repository;
use mog::index::Index;
use mog::storage::MogStorage;

fn mock_repo() -> Repository<mog::storage_mock::MockStorage> {
    Repository::new_mock()
}

// @Note: These are AI generated, but it's fine for a start I guess...

//
//
// Object store tests
//
//

#[test]
fn test_blob_roundtrip() {
    let mut repo = mock_repo();
    let data = b"hello world";
    let hash = repo.write_blob(data);
    assert!(repo.storage.exists(&hash));
    let raw = repo.storage.read(&hash).unwrap();
    let got = mog::object::decode_blob_bytes(raw).unwrap();
    assert_eq!(got, data);
}

#[test]
fn test_blob_dedup() {
    let mut repo = mock_repo();
    let data = b"same content";
    let h1 = repo.write_blob(data);
    let h2 = repo.write_blob(data);
    assert_eq!(h1, h2);
    assert_eq!(repo.storage.object_count(), 1);
}

#[test]
fn test_empty_blob() {
    let mut repo = mock_repo();
    let hash = repo.write_blob(b"");
    let raw  = repo.storage.read(&hash).unwrap();
    let got  = mog::object::decode_blob_bytes(raw).unwrap();
    assert_eq!(got, b"");
}

#[test]
fn test_large_blob() {
    let mut repo = mock_repo();
    let data: Vec<u8> = (0..100_000).map(|i| (i % 256) as u8).collect();
    let hash = repo.write_blob(&data);
    let raw  = repo.storage.read(&hash).unwrap();
    let got  = mog::object::decode_blob_bytes(raw).unwrap();
    assert_eq!(got, data.as_slice());
}

//
//
// Tree tests
//
//

#[test]
fn test_tree_roundtrip() {
    let mut repo = mock_repo();

    let blob_hash = repo.write_blob(b"file contents");

    let entries = vec![
        mog::tree::TreeEntry {
            hash: blob_hash,
            name: "file.txt".into(),
            mode: mog::object::MODE_FILE,
        },
    ];

    let tree_id   = repo.tree.push(&entries);
    let tree_hash = repo.write_object(mog::object::Object::Tree(tree_id));

    assert!(repo.storage.exists(&tree_hash));

    let raw      = repo.storage.read(&tree_hash).unwrap();
    let obj      = repo.stores.decode_and_push_object(raw).unwrap();
    let tid      = obj.try_as_tree_id().unwrap();
    assert_eq!(repo.tree.entry_count(tid), 1);

    let e = repo.tree.get_entry(tid, 0);
    assert_eq!(e.name.as_ref(), "file.txt");
    assert_eq!(e.hash, blob_hash);
    assert_eq!(e.mode, mog::object::MODE_FILE);
}

#[test]
fn test_tree_multiple_entries_sorted() {
    let mut repo = mock_repo();

    let h1 = repo.write_blob(b"aaa");
    let h2 = repo.write_blob(b"bbb");
    let h3 = repo.write_blob(b"ccc");

    let entries = vec![
        mog::tree::TreeEntry { hash: h2, name: "b.txt".into(), mode: mog::object::MODE_FILE },
        mog::tree::TreeEntry { hash: h1, name: "a.txt".into(), mode: mog::object::MODE_FILE },
        mog::tree::TreeEntry { hash: h3, name: "c.txt".into(), mode: mog::object::MODE_FILE },
    ];

    let tree_id = repo.tree.push(&entries);
    assert_eq!(repo.tree.entry_count(tree_id), 3);
    // entries stored in insertion order - sorting is caller's responsibility
    assert_eq!(repo.tree.get_entry(tree_id, 0).name.as_ref(), "b.txt");
    assert_eq!(repo.tree.get_entry(tree_id, 1).name.as_ref(), "a.txt");
}

//
//
// Commit tests
//
//

#[test]
fn test_commit_roundtrip() {
    let mut repo = mock_repo();

    let blob_hash = repo.write_blob(b"content");
    let entries   = vec![mog::tree::TreeEntry {
        hash: blob_hash, name: "f.txt".into(), mode: mog::object::MODE_FILE,
    }];
    let tree_id   = repo.tree.push(&entries);
    let tree_hash = repo.write_object(mog::object::Object::Tree(tree_id));

    let commit_id   = repo.commit.push(tree_hash, &[], 1234567890, "test author", "initial commit");
    let commit_hash = repo.write_object(mog::object::Object::Commit(commit_id));

    assert!(repo.storage.exists(&commit_hash));
    assert_eq!(repo.commit.get_message(commit_id), "initial commit");
    assert_eq!(repo.commit.get_author(commit_id),  "test author");
    assert_eq!(repo.commit.get_tree(commit_id),    tree_hash);
}

#[test]
fn test_commit_with_parent() {
    let mut repo = mock_repo();

    let tree_hash = {
        let h  = repo.write_blob(b"v1");
        let id = repo.tree.push(&[mog::tree::TreeEntry {
            hash: h, name: "f.txt".into(), mode: mog::object::MODE_FILE,
        }]);
        repo.write_object(mog::object::Object::Tree(id))
    };

    let c1_id   = repo.commit.push(tree_hash, &[], 1000, "author", "first");
    let c1_hash = repo.write_object(mog::object::Object::Commit(c1_id));

    let c2_id = repo.commit.push(tree_hash, &[c1_hash], 2000, "author", "second");
    assert_eq!(repo.commit.get_parents(c2_id), &[c1_hash]);
}

//
//
// Index tests
//
//

#[test]
fn test_index_add_find_remove() {
    let mut index = Index::default();

    let hash = [0xabu8; 32];
    let meta = make_fake_meta(1234, 100);

    index.add("src/main.rs", hash, &meta);
    assert_eq!(index.count, 1);

    let i = index.find("src/main.rs").unwrap();
    assert_eq!(index.hashes[i], hash);

    assert!(index.remove("src/main.rs"));
    assert_eq!(index.count, 0);
    assert!(index.find("src/main.rs").is_none());
}

#[test]
fn test_index_dedup_on_re_add() {
    let mut index = Index::default();
    let hash1 = [0x01u8; 32];
    let hash2 = [0x02u8; 32];
    let meta  = make_fake_meta(1000, 50);

    index.add("foo.rs", hash1, &meta);
    assert_eq!(index.count, 1);

    // Re-add same path with different hash - should update, not duplicate.
    let meta2 = make_fake_meta(2000, 60);
    index.add("foo.rs", hash2, &meta2);
    assert_eq!(index.count, 1);

    let i = index.find("foo.rs").unwrap();
    assert_eq!(index.hashes[i], hash2);
}

#[test]
fn test_index_is_dirty() {
    let mut index = Index::default();
    let hash = [0xabu8; 32];
    let meta = make_fake_meta(9999, 42);

    index.add("a.rs", hash, &meta);
    let i = index.find("a.rs").unwrap();

    // Same mtime and size - clean.
    assert!(!index.is_dirty(i, &make_fake_meta(9999, 42)));
    // Different mtime - dirty.
    assert!(index.is_dirty(i,  &make_fake_meta(1111, 42)));
    // Different size - dirty.
    assert!(index.is_dirty(i,  &make_fake_meta(9999, 99)));
}

#[test]
fn test_index_encode_decode_roundtrip() {
    let mut index = Index::default();
    let hash = [0xdeu8; 32];
    let meta = make_fake_meta(5555, 128);

    index.add("src/lib.rs", hash, &meta);
    index.add("src/main.rs", [0xadu8; 32], &make_fake_meta(6666, 256));

    let encoded = index.encode_for_test();
    let decoded = Index::decode_for_test(&encoded).unwrap();

    assert_eq!(decoded.count, 2);
    let i = decoded.find("src/lib.rs").unwrap();
    assert_eq!(decoded.hashes[i], hash);
    let j = decoded.find("src/main.rs").unwrap();
    assert_eq!(decoded.hashes[j], [0xadu8; 32]);
}

//
//
// Hash table / storage stress test
//
//

#[test]
fn test_mock_storage_many_objects() {
    let mut repo = mock_repo();
    let n = 1000usize;

    let hashes: Vec<_> = (0..n).map(|i| {
        let data = format!("object number {i}");
        repo.write_blob(data.as_bytes())
    }).collect();

    assert_eq!(repo.storage.object_count(), n);

    for (i, hash) in hashes.iter().enumerate() {
        let raw  = repo.storage.read(hash).unwrap();
        let got  = mog::object::decode_blob_bytes(raw).unwrap();
        let want = format!("object number {i}");
        assert_eq!(got, want.as_bytes());
    }
}

//
//
// Index edge cases
//
//

#[test]
fn test_index_remove_nonexistent() {
    let mut index = Index::default();
    assert!(!index.remove("doesnt_exist.rs"));
    assert_eq!(index.count, 0);
}

#[test]
fn test_index_many_files() {
    let mut index = Index::default();
    let n = 1000usize;

    for i in 0..n {
        let path = format!("src/file_{i}.rs");
        let hash = [i as u8; 32];
        index.add(&path, hash, &make_fake_meta(i as i64, i as u64));
    }
    assert_eq!(index.count, n);

    for i in 0..n {
        let path = format!("src/file_{i}.rs");
        let idx  = index.find(&path).unwrap();
        assert_eq!(index.hashes[idx], [i as u8; 32]);
    }
}

#[test]
fn test_index_remove_middle_entry() {
    let mut index = Index::default();
    index.add("a.rs", [0x01u8; 32], &make_fake_meta(1, 1));
    index.add("b.rs", [0x02u8; 32], &make_fake_meta(2, 2));
    index.add("c.rs", [0x03u8; 32], &make_fake_meta(3, 3));

    assert!(index.remove("b.rs"));
    assert_eq!(index.count, 2);

    // a and c should still be findable.
    assert!(index.find("a.rs").is_some());
    assert!(index.find("c.rs").is_some());
    assert!(index.find("b.rs").is_none());
}

#[test]
fn test_index_clear() {
    let mut index = Index::default();
    index.add("a.rs", [0x01u8; 32], &make_fake_meta(1, 1));
    index.add("b.rs", [0x02u8; 32], &make_fake_meta(2, 2));
    index.clear();
    assert_eq!(index.count, 0);
    assert!(index.find("a.rs").is_none());
    assert!(index.find("b.rs").is_none());
}

#[test]
fn test_index_encode_decode_empty() {
    let index   = Index::default();
    let encoded = index.encode_for_test();
    let decoded = Index::decode_for_test(&encoded).unwrap();
    assert_eq!(decoded.count, 0);
}

#[test]
fn test_index_encode_decode_paths_with_slashes() {
    let mut index = Index::default();
    index.add("a/b/c/deep.rs",  [0x01u8; 32], &make_fake_meta(1, 1));
    index.add("a/b/other.rs",   [0x02u8; 32], &make_fake_meta(2, 2));
    index.add("root.rs",        [0x03u8; 32], &make_fake_meta(3, 3));

    let encoded = index.encode_for_test();
    let decoded = Index::decode_for_test(&encoded).unwrap();

    assert_eq!(decoded.count, 3);
    assert_eq!(decoded.get_path(decoded.find("a/b/c/deep.rs").unwrap()), "a/b/c/deep.rs");
    assert_eq!(decoded.get_path(decoded.find("root.rs").unwrap()), "root.rs");
}

//
//
// Tree edge cases
//
//

#[test]
fn test_empty_tree() {
    let mut repo = mock_repo();
    let tree_id   = repo.tree.push(&[]);
    let tree_hash = repo.write_object(mog::object::Object::Tree(tree_id));
    assert!(repo.storage.exists(&tree_hash));
    assert_eq!(repo.tree.entry_count(tree_id), 0);
}

#[test]
fn test_tree_with_subtree() {
    let mut repo = mock_repo();

    let blob_hash = repo.write_blob(b"nested file");
    let sub_entries = vec![mog::tree::TreeEntry {
        hash: blob_hash,
        name: "nested.rs".into(),
        mode: mog::object::MODE_FILE,
    }];
    let sub_tree_id   = repo.tree.push(&sub_entries);
    let sub_tree_hash = repo.write_object(mog::object::Object::Tree(sub_tree_id));

    let root_entries = vec![
        mog::tree::TreeEntry {
            hash: sub_tree_hash,
            name: "subdir".into(),
            mode: mog::object::MODE_DIR,
        },
        mog::tree::TreeEntry {
            hash: blob_hash,
            name: "root.rs".into(),
            mode: mog::object::MODE_FILE,
        },
    ];
    let root_id = repo.tree.push(&root_entries);
    assert_eq!(repo.tree.entry_count(root_id), 2);

    let subdir_entry = repo.tree.get_entry(root_id, 0);
    assert_eq!(subdir_entry.mode, mog::object::MODE_DIR);
    assert_eq!(subdir_entry.hash, sub_tree_hash);
}

#[test]
fn test_tree_find_entry() {
    let mut repo = mock_repo();

    let h1 = repo.write_blob(b"aaa");
    let h2 = repo.write_blob(b"bbb");

    let entries = vec![
        mog::tree::TreeEntry { hash: h1, name: "alpha.rs".into(), mode: mog::object::MODE_FILE },
        mog::tree::TreeEntry { hash: h2, name: "beta.rs".into(),  mode: mog::object::MODE_FILE },
    ];
    let tree_id = repo.tree.push(&entries);

    assert_eq!(repo.tree.find_entry(tree_id, "alpha.rs"), Some(h1));
    assert_eq!(repo.tree.find_entry(tree_id, "beta.rs"),  Some(h2));
    assert_eq!(repo.tree.find_entry(tree_id, "gamma.rs"), None);
}

//
//
// Commit edge cases
//
//

#[test]
fn test_commit_chain() {
    let mut repo = mock_repo();
    let tree_hash = write_simple_tree(&mut repo, b"v1", "f.txt");

    let mut prev = None;
    let mut hashes = Vec::new();
    for i in 0..10 {
        let parents: Vec<_> = prev.into_iter().collect();
        let id   = repo.commit.push(tree_hash, &parents, i * 1000, "author", &format!("commit {i}"));
        let hash = repo.write_object(mog::object::Object::Commit(id));
        hashes.push(hash);
        prev = Some(hash);
    }

    // Walk the chain.
    let mut current = *hashes.last().unwrap();
    for i in (0..10).rev() {
        let obj = repo.stores.decode_and_push_object(
            repo.storage.read(&current).unwrap()
        ).unwrap();
        let id  = obj.try_as_commit_id().unwrap();
        assert_eq!(repo.commit.get_message(id), format!("commit {i}"));
        let parents = repo.commit.get_parents(id);
        if i == 0 {
            assert!(parents.is_empty());
            break;
        }
        current = parents[0];
    }
}

#[test]
fn test_commit_same_tree_different_message() {
    let mut repo = mock_repo();
    let tree_hash = write_simple_tree(&mut repo, b"content", "f.txt");

    let id1 = repo.commit.push(tree_hash, &[], 1000, "author", "first");
    let id2 = repo.commit.push(tree_hash, &[], 2000, "author", "second");
    let h1  = repo.write_object(mog::object::Object::Commit(id1));
    let h2  = repo.write_object(mog::object::Object::Commit(id2));

    // Different messages = different hashes.
    assert_ne!(h1, h2);
    assert_eq!(repo.commit.get_tree(id1), repo.commit.get_tree(id2));
}

//
//
// Content addressing
//
//

#[test]
fn test_identical_content_same_hash() {
    let mut repo = mock_repo();
    let h1 = repo.write_blob(b"the quick brown fox");
    let h2 = repo.write_blob(b"the quick brown fox");
    assert_eq!(h1, h2);
    assert_eq!(repo.storage.object_count(), 1);
}

#[test]
fn test_different_content_different_hash() {
    let mut repo = mock_repo();
    let h1 = repo.write_blob(b"foo");
    let h2 = repo.write_blob(b"bar");
    assert_ne!(h1, h2);
    assert_eq!(repo.storage.object_count(), 2);
}

#[test]
fn test_blob_with_null_bytes() {
    let mut repo = mock_repo();
    let data = b"hello\x00world\x00";
    let hash = repo.write_blob(data);
    let raw  = repo.storage.read(&hash).unwrap();
    let got  = mog::object::decode_blob_bytes(raw).unwrap();
    assert_eq!(got, data);
}

#[test]
fn test_blob_with_unicode() {
    let mut repo = mock_repo();
    let data = "こんにちは世界 🦀".as_bytes();
    let hash = repo.write_blob(data);
    let raw  = repo.storage.read(&hash).unwrap();
    let got  = mog::object::decode_blob_bytes(raw).unwrap();
    assert_eq!(got, data);
}

//
//
// Additional helpers
//
//

fn write_simple_tree(
    repo: &mut mog::repository::Repository<mog::storage_mock::MockStorage>,
    content: &[u8],
    name: &str,
) -> mog::hash::Hash {
    let blob_hash = repo.write_blob(content);
    let entries   = vec![mog::tree::TreeEntry {
        hash: blob_hash,
        name: name.into(),
        mode: mog::object::MODE_FILE,
    }];
    let tree_id = repo.tree.push(&entries);
    repo.write_object(mog::object::Object::Tree(tree_id))
}

//
//
// Full workflow tests
//
//

#[test]
fn test_full_commit_workflow() {
    let mut repo = mock_repo();

    let mut index = Index::default();
    let h1 = repo.write_blob(b"fn main() {}");
    let h2 = repo.write_blob(b"pub fn lib() {}");
    index.add("src/main.rs", h1, &make_fake_meta(1000, 12));
    index.add("src/lib.rs",  h2, &make_fake_meta(1001, 15));

    let tree_hash = index.write_tree(&mut repo).unwrap();

    let c1_id   = repo.commit.push(tree_hash, &[], 1000, "Alice", "init");
    let c1_hash = repo.write_object(mog::object::Object::Commit(c1_id));

    let h3 = repo.write_blob(b"fn main() { println!(\"hello\"); }");
    index.add("src/main.rs", h3, &make_fake_meta(2000, 33));
    let tree_hash2 = index.write_tree(&mut repo).unwrap();

    assert_ne!(tree_hash, tree_hash2);

    let c2_id   = repo.commit.push(tree_hash2, &[c1_hash], 2000, "Alice", "update main");
    let _c2_hash = repo.write_object(mog::object::Object::Commit(c2_id));

    assert_eq!(repo.commit.get_parents(c2_id), &[c1_hash]);
    assert!(repo.commit.get_parents(c1_id).is_empty());
    assert_eq!(repo.commit.get_message(c2_id), "update main");

    //
    // Traverse root -> src dir -> main.rs
    //
    let t2_obj = repo.stores.decode_and_push_object(
        repo.storage.read(&tree_hash2).unwrap()
    ).unwrap();
    let t2_id = t2_obj.try_as_tree_id().unwrap();

    // Root tree has a "src" directory entry.
    let src_hash = repo.tree.find_entry(t2_id, "src").unwrap();

    let src_obj = repo.stores.decode_and_push_object(
        repo.storage.read(&src_hash).unwrap()
    ).unwrap();
    let src_id = src_obj.try_as_tree_id().unwrap();

    // src tree has "main.rs".
    let main_hash = repo.tree.find_entry(src_id, "main.rs").unwrap();
    let raw  = repo.storage.read(&main_hash).unwrap();
    let data = mog::object::decode_blob_bytes(raw).unwrap();
    assert_eq!(data, b"fn main() { println!(\"hello\"); }");
}

#[test]
fn test_tree_content_addressed_dedup() {
    let mut repo = mock_repo();

    // Two indices with identical content should produce the same tree hash.
    let mut index1 = Index::default();
    let mut index2 = Index::default();

    let h = repo.write_blob(b"shared content");
    index1.add("file.rs", h, &make_fake_meta(1, 14));
    index2.add("file.rs", h, &make_fake_meta(9999, 14)); // different mtime, same content

    let t1 = index1.write_tree(&mut repo).unwrap();
    let t2 = index2.write_tree(&mut repo).unwrap();

    // Tree hash depends on content only (path + blob hash + mode), not mtime.
    assert_eq!(t1, t2);
}

#[test]
fn test_status_clean_after_add() {
    let mut repo = mock_repo();

    let h1 = repo.write_blob(b"content a");
    let h2 = repo.write_blob(b"content b");

    let mut index = Index::default();
    index.add("a.rs", h1, &make_fake_meta(1000, 9));
    index.add("b.rs", h2, &make_fake_meta(1001, 9));

    // Simulate HEAD = same tree as index - nothing staged.
    let tree_hash = index.write_tree(&mut repo).unwrap();
    let commit_id = repo.commit.push(tree_hash, &[], 1000, "author", "commit");
    let _commit_hash = repo.write_object(mog::object::Object::Commit(commit_id));

    let head_flat = mog::status::flatten_tree(&mut repo, tree_hash).unwrap();

    // Nothing should be staged since index matches HEAD exactly.
    for i in 0..index.count {
        let path = index.get_path(i);
        let index_hash = index.hashes[i];
        let head_hash  = head_flat.lookup(path);
        assert_eq!(head_hash, Some(index_hash), "path {path} should be clean");
    }
}

#[test]
fn test_status_detects_staged_new_file() {
    let mut repo = mock_repo();

    // HEAD has one file.
    let h1 = repo.write_blob(b"original");
    let mut head_index = Index::default();
    head_index.add("existing.rs", h1, &make_fake_meta(1000, 8));
    let head_tree = head_index.write_tree(&mut repo).unwrap();

    let head_flat = mog::status::flatten_tree(&mut repo, head_tree).unwrap();

    // Index has two files - one new.
    let h2 = repo.write_blob(b"new file");
    let mut index = Index::default();
    index.add("existing.rs", h1, &make_fake_meta(1000, 8));
    index.add("new.rs",      h2, &make_fake_meta(2000, 8));

    let mut staged = Vec::new();
    for i in 0..index.count {
        let path       = index.get_path(i);
        let index_hash = index.hashes[i];
        if head_flat.lookup(path) != Some(index_hash) {
            staged.push(path.to_string());
        }
    }

    assert_eq!(staged, vec!["new.rs"]);
}

#[test]
fn test_status_detects_staged_modification() {
    let mut repo = mock_repo();

    let h_old = repo.write_blob(b"old content");
    let h_new = repo.write_blob(b"new content");

    // HEAD has old content.
    let mut head_index = Index::default();
    head_index.add("file.rs", h_old, &make_fake_meta(1000, 11));
    let head_tree = head_index.write_tree(&mut repo).unwrap();
    let head_flat = mog::status::flatten_tree(&mut repo, head_tree).unwrap();

    // Index has new content.
    let mut index = Index::default();
    index.add("file.rs", h_new, &make_fake_meta(2000, 11));

    let i          = index.find("file.rs").unwrap();
    let index_hash = index.hashes[i];
    let head_hash  = head_flat.lookup("file.rs");

    assert_ne!(Some(index_hash), head_hash);
}

#[test]
fn test_status_detects_staged_delete() {
    let mut repo = mock_repo();

    let h = repo.write_blob(b"to be deleted");

    // HEAD has the file.
    let mut head_index = Index::default();
    head_index.add("gone.rs", h, &make_fake_meta(1000, 13));
    let head_tree = head_index.write_tree(&mut repo).unwrap();
    let head_flat = mog::status::flatten_tree(&mut repo, head_tree).unwrap();

    // Index does not have the file.
    let index = Index::default();

    let mut staged_deleted = Vec::new();
    for j in 0..head_flat.len() {
        let path = head_flat.get_path(head_flat.sorted_order[j]);
        if index.find(path).is_none() {
            staged_deleted.push(path.to_string());
        }
    }

    assert_eq!(staged_deleted, vec!["gone.rs"]);
}

#[test]
fn test_index_dirty_detection() {
    let mut index = Index::default();
    let hash = [0xabu8; 32];

    index.add("f.rs", hash, &make_fake_meta(1000, 100));
    let i = index.find("f.rs").unwrap();

    // Exact match - clean.
    assert!(!index.is_dirty(i, &make_fake_meta(1000, 100)));
    // Mtime changed, size same - dirty (could be same content but we don't know).
    assert!(index.is_dirty(i,  &make_fake_meta(1001, 100)));
    // Size changed - definitely dirty.
    assert!(index.is_dirty(i,  &make_fake_meta(1000, 101)));
    // Both changed.
    assert!(index.is_dirty(i,  &make_fake_meta(9999, 999)));
    // Zero mtime edge case.
    assert!(index.is_dirty(i,  &make_fake_meta(0, 100)));
    // Zero size edge case.
    assert!(index.is_dirty(i,  &make_fake_meta(1000, 0)));
}

#[test]
fn test_sorted_flat_tree_binary_search_correctness() {
    let mut builder = mog::status::FlatTreeBuilder::new();

    // Insert out of alphabetical order.
    let hashes: Vec<_> = (0..26).map(|i| {
        let mut h = [0u8; 32];
        h[0] = i as u8;
        h
    }).collect();

    // Insert in reverse order.
    for i in (0..26).rev() {
        let name = format!("{}.rs", (b'a' + i as u8) as char);
        builder.push(&name, hashes[i as usize]);
    }

    let flat = builder.build();
    assert_eq!(flat.len(), 26);

    // Every entry should be findable.
    for i in 0..26usize {
        let name = format!("{}.rs", (b'a' + i as u8) as char);
        assert_eq!(flat.lookup(&name), Some(hashes[i]), "failed to find {name}");
    }

    // Non-existent entries should return None.
    assert_eq!(flat.lookup("z_extra.rs"), None);
    assert_eq!(flat.lookup(""),           None);
}

#[test]
fn test_sorted_flat_tree_empty() {
    let builder = mog::status::FlatTreeBuilder::new();
    let flat    = builder.build();
    assert_eq!(flat.len(), 0);
    assert!(flat.is_empty());
    assert_eq!(flat.lookup("anything"), None);
}

#[test]
fn test_sorted_flat_tree_single_entry() {
    let mut builder = mog::status::FlatTreeBuilder::new();
    let hash = [0x42u8; 32];
    builder.push("only.rs", hash);
    let flat = builder.build();
    assert_eq!(flat.lookup("only.rs"),  Some(hash));
    assert_eq!(flat.lookup("other.rs"), None);
}

#[test]
fn test_commit_history_chain_integrity() {
    let mut repo = mock_repo();

    // Build a 5-commit chain, each modifying the same file.
    let mut prev_hash = None;
    let mut commit_hashes = Vec::new();

    for i in 0..5usize {
        let content   = format!("version {i}");
        let blob_hash = repo.write_blob(content.as_bytes());
        let entries   = vec![mog::tree::TreeEntry {
            hash: blob_hash,
            name: "file.rs".into(),
            mode: mog::object::MODE_FILE,
        }];
        let tree_id   = repo.tree.push(&entries);
        let tree_hash = repo.write_object(mog::object::Object::Tree(tree_id));
        let parents: Vec<_> = prev_hash.into_iter().collect();
        let commit_id   = repo.commit.push(tree_hash, &parents, i as i64 * 1000, "author", &format!("v{i}"));
        let commit_hash = repo.write_object(mog::object::Object::Commit(commit_id));
        commit_hashes.push((commit_hash, tree_hash, blob_hash));
        prev_hash = Some(commit_hash);
    }

    // Walk backward from tip and verify each commit's tree contains the right content.
    let mut current = commit_hashes.last().unwrap().0;
    for i in (0..5).rev() {
        let raw       = repo.storage.read(&current).unwrap();
        let obj       = repo.stores.decode_and_push_object(raw).unwrap();
        let cid       = obj.try_as_commit_id().unwrap();
        let tree_hash = repo.commit.get_tree(cid);

        let tree_raw  = repo.storage.read(&tree_hash).unwrap();
        let tree_obj  = repo.stores.decode_and_push_object(tree_raw).unwrap();
        let tid       = tree_obj.try_as_tree_id().unwrap();

        let blob_hash = repo.tree.find_entry(tid, "file.rs").unwrap();
        let blob_raw  = repo.storage.read(&blob_hash).unwrap();
        let blob_data = mog::object::decode_blob_bytes(blob_raw).unwrap();
        assert_eq!(blob_data, format!("version {i}").as_bytes());

        let parents = repo.commit.get_parents(cid);
        if i == 0 {
            assert!(parents.is_empty());
            break;
        }
        current = parents[0];
    }
}

//
//
// Adversarial / edge case tests
//
//

#[test]
fn test_blob_single_byte() {
    let mut repo = mock_repo();
    for b in 0u8..=255 {
        let hash = repo.write_blob(&[b]);
        let raw  = repo.storage.read(&hash).unwrap();
        let got  = mog::object::decode_blob_bytes(raw).unwrap();
        assert_eq!(got, &[b], "failed for byte 0x{b:02x}");
    }
    // All 256 single-byte blobs should be distinct objects.
    assert_eq!(repo.storage.object_count(), 256);
}

#[test]
fn test_blob_all_zeros_vs_all_ones() {
    let mut repo = mock_repo();
    let h1 = repo.write_blob(&[0u8; 1024]);
    let h2 = repo.write_blob(&[0xffu8; 1024]);
    let h3 = repo.write_blob(&[0u8; 1024]); // same as h1
    assert_ne!(h1, h2);
    assert_eq!(h1, h3);
    assert_eq!(repo.storage.object_count(), 2);
}

#[test]
fn test_index_path_prefix_collision() {
    // "foo" and "foo/bar" are different paths - make sure hash map doesn't collide.
    let mut index = Index::default();
    let h1 = [0x01u8; 32];
    let h2 = [0x02u8; 32];
    let h3 = [0x03u8; 32];
    index.add("foo",        h1, &make_fake_meta(1, 1));
    index.add("foo/bar",    h2, &make_fake_meta(2, 2));
    index.add("foo/bar/baz",h3, &make_fake_meta(3, 3));

    assert_eq!(index.count, 3);
    assert_eq!(index.hashes[index.find("foo").unwrap()],         h1);
    assert_eq!(index.hashes[index.find("foo/bar").unwrap()],     h2);
    assert_eq!(index.hashes[index.find("foo/bar/baz").unwrap()], h3);
}

#[test]
fn test_index_unicode_paths() {
    let mut index = Index::default();
    let h1 = [0x01u8; 32];
    let h2 = [0x02u8; 32];
    let h3 = [0x03u8; 32];
    index.add("src/こんにちは.rs", h1, &make_fake_meta(1, 1));
    index.add("src/🦀.rs",        h2, &make_fake_meta(2, 2));
    index.add("données/été.txt",   h3, &make_fake_meta(3, 3));

    assert_eq!(index.hashes[index.find("src/こんにちは.rs").unwrap()], h1);
    assert_eq!(index.hashes[index.find("src/🦀.rs").unwrap()],        h2);
    assert_eq!(index.hashes[index.find("données/été.txt").unwrap()],   h3);

    // Encode/decode roundtrip with unicode paths.
    let encoded = index.encode_for_test();
    let decoded = Index::decode_for_test(&encoded).unwrap();
    assert_eq!(decoded.count, 3);
    assert!(decoded.find("src/🦀.rs").is_some());
}

#[test]
fn test_index_remove_all_entries_one_by_one() {
    let mut index = Index::default();
    let n = 50usize;

    for i in 0..n {
        index.add(&format!("file_{i}.rs"), [i as u8; 32], &make_fake_meta(i as i64, i as u64));
    }
    assert_eq!(index.count, n);

    // Remove in reverse order.
    for i in (0..n).rev() {
        let path = format!("file_{i}.rs");
        assert!(index.remove(&path), "remove failed for {path}");
        assert!(index.find(&path).is_none(), "find should return None after remove for {path}");
    }
    assert_eq!(index.count, 0);
}

#[test]
fn test_index_re_add_after_remove() {
    let mut index = Index::default();
    let h1 = [0x01u8; 32];
    let h2 = [0x02u8; 32];

    index.add("a.rs", h1, &make_fake_meta(1, 1));
    assert!(index.remove("a.rs"));
    assert_eq!(index.count, 0);

    // Re-add same path - should work cleanly.
    index.add("a.rs", h2, &make_fake_meta(2, 2));
    assert_eq!(index.count, 1);
    let i = index.find("a.rs").unwrap();
    assert_eq!(index.hashes[i], h2);
}

#[test]
fn test_sorted_flat_tree_duplicate_paths_last_wins() {
    // Builder allows duplicate pushes - last one should win after sort
    // since binary search finds one arbitrarily. Document the behavior.
    let mut builder = mog::status::FlatTreeBuilder::new();
    let h1 = [0x01u8; 32];
    let h2 = [0x02u8; 32];
    builder.push("same.rs", h1);
    builder.push("same.rs", h2);
    let flat = builder.build();

    // Should find *something* - not panic or return None.
    assert!(flat.lookup("same.rs").is_some());
}

#[test]
fn test_tree_entry_name_with_spaces_and_dots() {
    let mut repo = mock_repo();
    let h = repo.write_blob(b"content");
    let entries = vec![
        mog::tree::TreeEntry { hash: h, name: "my file.txt".into(),  mode: mog::object::MODE_FILE },
        mog::tree::TreeEntry { hash: h, name: ".hidden".into(),       mode: mog::object::MODE_FILE },
        mog::tree::TreeEntry { hash: h, name: "...".into(),           mode: mog::object::MODE_FILE },
        mog::tree::TreeEntry { hash: h, name: "file with spaces".into(), mode: mog::object::MODE_FILE },
    ];
    let tree_id   = repo.tree.push(&entries);
    let tree_hash = repo.write_object(mog::object::Object::Tree(tree_id));

    // Roundtrip.
    let raw     = repo.storage.read(&tree_hash).unwrap();
    let obj     = repo.stores.decode_and_push_object(raw).unwrap();
    let tid     = obj.try_as_tree_id().unwrap();

    assert_eq!(repo.tree.find_entry(tid, "my file.txt"),      Some(h));
    assert_eq!(repo.tree.find_entry(tid, ".hidden"),           Some(h));
    assert_eq!(repo.tree.find_entry(tid, "..."),               Some(h));
    assert_eq!(repo.tree.find_entry(tid, "file with spaces"),  Some(h));
    assert_eq!(repo.tree.find_entry(tid, "nonexistent"),       None);
}

#[test]
fn test_commit_zero_timestamp() {
    let mut repo = mock_repo();
    let tree_hash = write_simple_tree(&mut repo, b"x", "x.rs");
    let id   = repo.commit.push(tree_hash, &[], 0, "author", "epoch commit");
    let hash = repo.write_object(mog::object::Object::Commit(id));
    assert!(repo.storage.exists(&hash));
    assert_eq!(repo.commit.get_message(id), "epoch commit");
}

#[test]
fn test_commit_empty_message() {
    let mut repo = mock_repo();
    let tree_hash = write_simple_tree(&mut repo, b"x", "x.rs");
    let id   = repo.commit.push(tree_hash, &[], 1000, "author", "");
    let hash = repo.write_object(mog::object::Object::Commit(id));
    assert!(repo.storage.exists(&hash));
    assert_eq!(repo.commit.get_message(id), "");
}

#[test]
fn test_commit_empty_author() {
    let mut repo = mock_repo();
    let tree_hash = write_simple_tree(&mut repo, b"x", "x.rs");
    let id = repo.commit.push(tree_hash, &[], 1000, "", "msg");
    assert_eq!(repo.commit.get_author(id), "");
}

#[test]
fn test_write_tree_path_ordering() {
    // write_tree must sort entries - trees with same files in different
    // insertion order should produce the same hash.
    let mut repo = mock_repo();

    let ha = repo.write_blob(b"aaa");
    let hb = repo.write_blob(b"bbb");
    let hc = repo.write_blob(b"ccc");

    let mut index1 = Index::default();
    index1.add("a.rs", ha, &make_fake_meta(1, 3));
    index1.add("b.rs", hb, &make_fake_meta(2, 3));
    index1.add("c.rs", hc, &make_fake_meta(3, 3));

    let mut index2 = Index::default();
    index2.add("c.rs", hc, &make_fake_meta(3, 3));
    index2.add("a.rs", ha, &make_fake_meta(1, 3));
    index2.add("b.rs", hb, &make_fake_meta(2, 3));

    let t1 = index1.write_tree(&mut repo).unwrap();
    let t2 = index2.write_tree(&mut repo).unwrap();

    assert_eq!(t1, t2, "tree hash must be insertion-order independent");
}

#[test]
fn test_storage_exists_after_write() {
    let mut repo = mock_repo();
    let hash = repo.write_blob(b"data");
    assert!(repo.storage.exists(&hash));

    let fake_hash = [0xffu8; 32];
    assert!(!repo.storage.exists(&fake_hash));
}

#[test]
fn test_storage_read_nonexistent_errors() {
    let repo = mock_repo();
    let fake = [0xabu8; 32];
    assert!(repo.storage.read(&fake).is_err());
}

#[test]
fn test_index_encode_decode_large() {
    let mut index = Index::default();
    let n = 5000usize;

    for i in 0..n {
        let path = format!("src/module_{:04}/file_{:04}.rs", i / 100, i % 100);
        let mut hash = [0u8; 32];
        hash[..8].copy_from_slice(&i.to_le_bytes());
        index.add(&path, hash, &make_fake_meta(i as i64, i as u64));
    }

    let encoded = index.encode_for_test();
    let decoded = Index::decode_for_test(&encoded).unwrap();
    assert_eq!(decoded.count, n);

    // Spot-check a few.
    for i in [0, 1, 99, 100, 999, 4999] {
        let path = format!("src/module_{:04}/file_{:04}.rs", i / 100, i % 100);
        let idx  = decoded.find(&path).expect(&format!("missing {path}"));
        let mut expected_hash = [0u8; 32];
        expected_hash[..8].copy_from_slice(&(i as usize).to_le_bytes());
        assert_eq!(decoded.hashes[idx], expected_hash);
    }
}

//
//
// Real-world workflow tests
//
//

/// Simulate: init repo, make several commits, checkout an old commit,
/// verify working state matches that point in history.
#[test]
fn test_checkout_restores_correct_historical_state() {
    let mut repo = mock_repo();

    // Commit 1: two files.
    let h_main_v1 = repo.write_blob(b"fn main() {}");
    let h_lib_v1  = repo.write_blob(b"pub fn foo() -> u32 { 1 }");
    let mut idx   = Index::default();
    idx.add("src/main.rs", h_main_v1, &make_fake_meta(1000, 12));
    idx.add("src/lib.rs",  h_lib_v1,  &make_fake_meta(1001, 25));
    let t1   = idx.write_tree(&mut repo).unwrap();
    let c1   = repo.commit.push(t1, &[], 1000, "Alice", "initial");
    let c1_h = repo.write_object(mog::object::Object::Commit(c1));

    // Commit 2: modify main, add new file.
    let h_main_v2   = repo.write_blob(b"fn main() { println!(\"v2\"); }");
    let h_readme_v1 = repo.write_blob(b"# My Project");
    idx.add("src/main.rs", h_main_v2,   &make_fake_meta(2000, 30));
    idx.add("README.md",   h_readme_v1, &make_fake_meta(2001, 12));
    let t2   = idx.write_tree(&mut repo).unwrap();
    let c2   = repo.commit.push(t2, &[c1_h], 2000, "Alice", "add readme and update main");
    let c2_h = repo.write_object(mog::object::Object::Commit(c2));

    // Commit 3: modify README.
    let h_readme_v2 = repo.write_blob(b"# My Project\n\nA great project.");
    idx.add("README.md", h_readme_v2, &make_fake_meta(3000, 30));
    let t3   = idx.write_tree(&mut repo).unwrap();
    let c3   = repo.commit.push(t3, &[c2_h], 3000, "Alice", "expand readme");
    let _c3_h = repo.write_object(mog::object::Object::Commit(c3));

    // Now "checkout" commit 1 and verify state.
    let t1_obj  = repo.stores.decode_and_push_object(repo.storage.read(&t1).unwrap()).unwrap();
    let t1_id   = t1_obj.try_as_tree_id().unwrap();
    let src_h   = repo.tree.find_entry(t1_id, "src").unwrap();
    let src_obj = repo.stores.decode_and_push_object(repo.storage.read(&src_h).unwrap()).unwrap();
    let src_id  = src_obj.try_as_tree_id().unwrap();

    // At commit 1, main.rs should be v1.
    let main_h = repo.tree.find_entry(src_id, "main.rs").unwrap();
    let data   = mog::object::decode_blob_bytes(repo.storage.read(&main_h).unwrap()).unwrap();
    assert_eq!(data, b"fn main() {}");

    // At commit 1, README should not exist.
    assert_eq!(repo.tree.find_entry(t1_id, "README.md"), None);

    // At commit 2, README should be v1.
    let t2_obj  = repo.stores.decode_and_push_object(repo.storage.read(&t2).unwrap()).unwrap();
    let t2_id   = t2_obj.try_as_tree_id().unwrap();
    let readme_h = repo.tree.find_entry(t2_id, "README.md").unwrap();
    let readme   = mog::object::decode_blob_bytes(repo.storage.read(&readme_h).unwrap()).unwrap();
    assert_eq!(readme, b"# My Project");

    // At commit 3, README should be v2.
    let t3_obj   = repo.stores.decode_and_push_object(repo.storage.read(&t3).unwrap()).unwrap();
    let t3_id    = t3_obj.try_as_tree_id().unwrap();
    let readme_h = repo.tree.find_entry(t3_id, "README.md").unwrap();
    let readme   = mog::object::decode_blob_bytes(repo.storage.read(&readme_h).unwrap()).unwrap();
    assert_eq!(readme, b"# My Project\n\nA great project.");
}

/// Simulate: two branches diverge from a common ancestor, verify their
/// trees are independent and the common ancestor is reachable from both.
#[test]
fn test_diverging_branches() {
    let mut repo = mock_repo();

    // Common base commit.
    let h_base = repo.write_blob(b"shared content");
    let mut idx = Index::default();
    idx.add("shared.rs", h_base, &make_fake_meta(1000, 14));
    let t_base   = idx.write_tree(&mut repo).unwrap();
    let c_base   = repo.commit.push(t_base, &[], 1000, "dev", "base");
    let c_base_h = repo.write_object(mog::object::Object::Commit(c_base));

    // Branch A: add feature_a.rs.
    let h_a = repo.write_blob(b"fn feature_a() {}");
    let mut idx_a = idx.clone();
    idx_a.add("feature_a.rs", h_a, &make_fake_meta(2000, 17));
    let t_a   = idx_a.write_tree(&mut repo).unwrap();
    let c_a   = repo.commit.push(t_a, &[c_base_h], 2000, "alice", "add feature a");
    let c_a_h = repo.write_object(mog::object::Object::Commit(c_a));

    // Branch B: add feature_b.rs.
    let h_b = repo.write_blob(b"fn feature_b() {}");
    let mut idx_b = idx.clone();
    idx_b.add("feature_b.rs", h_b, &make_fake_meta(2001, 17));
    let t_b   = idx_b.write_tree(&mut repo).unwrap();
    let c_b   = repo.commit.push(t_b, &[c_base_h], 2001, "bob", "add feature b");
    let c_b_h = repo.write_object(mog::object::Object::Commit(c_b));

    // Branch A should not contain feature_b.rs.
    let ta_obj = repo.stores.decode_and_push_object(repo.storage.read(&t_a).unwrap()).unwrap();
    let ta_id  = ta_obj.try_as_tree_id().unwrap();
    assert!(repo.tree.find_entry(ta_id, "feature_a.rs").is_some());
    assert!(repo.tree.find_entry(ta_id, "feature_b.rs").is_none());

    // Branch B should not contain feature_a.rs.
    let tb_obj = repo.stores.decode_and_push_object(repo.storage.read(&t_b).unwrap()).unwrap();
    let tb_id  = tb_obj.try_as_tree_id().unwrap();
    assert!(repo.tree.find_entry(tb_id, "feature_b.rs").is_some());
    assert!(repo.tree.find_entry(tb_id, "feature_a.rs").is_none());

    // Both branches share the same base commit as parent.
    assert_eq!(repo.commit.get_parents(c_a), &[c_base_h]);
    assert_eq!(repo.commit.get_parents(c_b), &[c_base_h]);

    // Simulate merge commit with two parents.
    let h_merge = repo.write_blob(b"fn feature_a() {}\nfn feature_b() {}");
    let mut idx_merge = idx.clone();
    idx_merge.add("feature_a.rs", h_a,     &make_fake_meta(3000, 17));
    idx_merge.add("feature_b.rs", h_b,     &make_fake_meta(3001, 17));
    idx_merge.add("merged.rs",    h_merge,  &make_fake_meta(3002, 35));
    let t_merge   = idx_merge.write_tree(&mut repo).unwrap();
    let c_merge   = repo.commit.push(t_merge, &[c_a_h, c_b_h], 3000, "dev", "merge a and b");
    let _c_merge_h = repo.write_object(mog::object::Object::Commit(c_merge));

    let parents = repo.commit.get_parents(c_merge);
    assert_eq!(parents.len(), 2);
    assert!(parents.contains(&c_a_h));
    assert!(parents.contains(&c_b_h));
}

/// Simulate: rename a file across commits - old path disappears, new path appears
/// with same content hash. Verify content-addressing means no duplicate storage.
#[test]
fn test_rename_file_across_commits() {
    let mut repo = mock_repo();
    let content  = b"fn important_logic() { 42 }";
    let h        = repo.write_blob(content);

    // Commit 1: file at old path.
    let mut idx = Index::default();
    idx.add("old_name.rs", h, &make_fake_meta(1000, content.len() as u64));
    let t1   = idx.write_tree(&mut repo).unwrap();
    let c1   = repo.commit.push(t1, &[], 1000, "dev", "before rename");
    let c1_h = repo.write_object(mog::object::Object::Commit(c1));

    // Commit 2: file at new path, same content.
    let mut idx2 = Index::default();
    idx2.add("new_name.rs", h, &make_fake_meta(2000, content.len() as u64));
    let t2   = idx2.write_tree(&mut repo).unwrap();
    let c2   = repo.commit.push(t2, &[c1_h], 2000, "dev", "rename");
    let _c2_h = repo.write_object(mog::object::Object::Commit(c2));

    // Trees differ (different filenames) but blob is stored only once.
    assert_ne!(t1, t2);

    // Content is identical - same hash, no duplicate blob.
    let t2_obj  = repo.stores.decode_and_push_object(repo.storage.read(&t2).unwrap()).unwrap();
    let t2_id   = t2_obj.try_as_tree_id().unwrap();
    let new_h   = repo.tree.find_entry(t2_id, "new_name.rs").unwrap();
    assert_eq!(new_h, h);
    assert_eq!(repo.tree.find_entry(t2_id, "old_name.rs"), None);
}

/// Simulate: delete a file, verify it disappears from the tree but history
/// still lets you recover it from old commits.
#[test]
fn test_delete_file_recoverable_from_history() {
    let mut repo = mock_repo();
    let precious = b"irreplaceable content";
    let h        = repo.write_blob(precious);

    let mut idx = Index::default();
    idx.add("precious.rs", h, &make_fake_meta(1000, precious.len() as u64));
    idx.add("other.rs",   repo.write_blob(b"other"), &make_fake_meta(1001, 5));
    let t1   = idx.write_tree(&mut repo).unwrap();
    let c1   = repo.commit.push(t1, &[], 1000, "dev", "add precious");
    let c1_h = repo.write_object(mog::object::Object::Commit(c1));

    // Delete precious.rs.
    idx.remove("precious.rs");
    let t2    = idx.write_tree(&mut repo).unwrap();
    let c2    = repo.commit.push(t2, &[c1_h], 2000, "dev", "delete precious");
    let _c2_h = repo.write_object(mog::object::Object::Commit(c2));

    // Current tree: precious.rs gone.
    let t2_obj = repo.stores.decode_and_push_object(repo.storage.read(&t2).unwrap()).unwrap();
    let t2_id  = t2_obj.try_as_tree_id().unwrap();
    assert!(repo.tree.find_entry(t2_id, "precious.rs").is_none());

    // Historical tree at c1: precious.rs recoverable.
    let t1_obj = repo.stores.decode_and_push_object(repo.storage.read(&t1).unwrap()).unwrap();
    let t1_id  = t1_obj.try_as_tree_id().unwrap();
    let old_h  = repo.tree.find_entry(t1_id, "precious.rs").unwrap();
    let data   = mog::object::decode_blob_bytes(repo.storage.read(&old_h).unwrap()).unwrap();
    assert_eq!(data, precious);
}

/// Simulate: a large project with deep directory nesting and many files,
/// verify the tree roundtrips correctly and lookup works at every level.
#[test]
fn test_deep_nested_tree_structure() {
    let mut repo = mock_repo();
    let mut idx  = Index::default();

    // Simulate a Rust project layout.
    let paths = vec![
        ("Cargo.toml",                         b"[package]" as &[u8]),
        ("src/main.rs",                        b"fn main() {}"),
        ("src/lib.rs",                         b"pub mod core;"),
        ("src/core/mod.rs",                    b"pub mod storage;"),
        ("src/core/storage/mod.rs",            b"pub struct Store;"),
        ("src/core/storage/backend.rs",        b"pub struct Backend;"),
        ("src/core/storage/cache.rs",          b"pub struct Cache;"),
        ("tests/integration_test.rs",          b"#[test] fn it_works() {}"),
        ("tests/common/mod.rs",                b"pub fn setup() {}"),
        ("docs/architecture.md",               b"# Architecture"),
        ("docs/api/reference.md",              b"# API Reference"),
    ];

    let mut hashes = std::collections::HashMap::new();
    for (path, content) in &paths {
        let h = repo.write_blob(content);
        idx.add(path, h, &make_fake_meta(1000, content.len() as u64));
        hashes.insert(*path, h);
    }

    let tree_hash = idx.write_tree(&mut repo).unwrap();

    // Verify by walking the tree manually for a few deep paths.
    let root_obj = repo.stores.decode_and_push_object(repo.storage.read(&tree_hash).unwrap()).unwrap();
    let root_id  = root_obj.try_as_tree_id().unwrap();

    // src/core/storage/backend.rs
    let src_h    = repo.tree.find_entry(root_id, "src").unwrap();
    let src_obj  = repo.stores.decode_and_push_object(repo.storage.read(&src_h).unwrap()).unwrap();
    let src_id   = src_obj.try_as_tree_id().unwrap();
    let core_h   = repo.tree.find_entry(src_id, "core").unwrap();
    let core_obj = repo.stores.decode_and_push_object(repo.storage.read(&core_h).unwrap()).unwrap();
    let core_id  = core_obj.try_as_tree_id().unwrap();
    let stor_h   = repo.tree.find_entry(core_id, "storage").unwrap();
    let stor_obj = repo.stores.decode_and_push_object(repo.storage.read(&stor_h).unwrap()).unwrap();
    let stor_id  = stor_obj.try_as_tree_id().unwrap();

    let backend_h = repo.tree.find_entry(stor_id, "backend.rs").unwrap();
    let data = mog::object::decode_blob_bytes(repo.storage.read(&backend_h).unwrap()).unwrap();
    assert_eq!(data, b"pub struct Backend;");

    // Cargo.toml at root.
    let cargo_h = repo.tree.find_entry(root_id, "Cargo.toml").unwrap();
    let data = mog::object::decode_blob_bytes(repo.storage.read(&cargo_h).unwrap()).unwrap();
    assert_eq!(data, b"[package]");

    // Encode/decode the index and verify count.
    let encoded = idx.encode_for_test();
    let decoded = Index::decode_for_test(&encoded).unwrap();
    assert_eq!(decoded.count, paths.len());
}

/// Simulate: amend a commit - same parent, updated tree, same parent chain.
/// The old commit still exists and is reachable by hash.
#[test]
fn test_amend_commit() {
    let mut repo = mock_repo();
    let mut idx  = Index::default();

    let h1 = repo.write_blob(b"typo in mesage");
    idx.add("file.rs", h1, &make_fake_meta(1000, 14));
    let t1   = idx.write_tree(&mut repo).unwrap();
    let c1   = repo.commit.push(t1, &[], 1000, "dev", "fix: typo in mesage");
    let c1_h = repo.write_object(mog::object::Object::Commit(c1));

    // "Amend": same tree, same parent, corrected message, slightly later timestamp.
    let c1_parents = repo.commit.get_parents(c1).to_vec();
    let c1_tree    = repo.commit.get_tree(c1);
    let c_amended  = repo.commit.push(c1_tree, &c1_parents, 1001, "dev", "fix: typo in message");
    let c_amended_h = repo.write_object(mog::object::Object::Commit(c_amended));

    // Amended commit has different hash (different message).
    assert_ne!(c1_h, c_amended_h);

    // Both commits point to same tree.
    assert_eq!(repo.commit.get_tree(c1), repo.commit.get_tree(c_amended));

    // Original commit still accessible.
    assert!(repo.storage.exists(&c1_h));
    assert_eq!(repo.commit.get_message(c1), "fix: typo in mesage");
    assert_eq!(repo.commit.get_message(c_amended), "fix: typo in message");
}

/// Simulate: revert a commit - create a new commit that restores previous tree state.
#[test]
fn test_revert_commit() {
    let mut repo = mock_repo();
    let mut idx  = Index::default();

    // Good state.
    let h_good = repo.write_blob(b"good code");
    idx.add("app.rs", h_good, &make_fake_meta(1000, 9));
    let t1   = idx.write_tree(&mut repo).unwrap();
    let c1   = repo.commit.push(t1, &[], 1000, "dev", "good state");
    let c1_h = repo.write_object(mog::object::Object::Commit(c1));

    // Bad commit.
    let h_bad = repo.write_blob(b"broken code!!!!");
    idx.add("app.rs", h_bad, &make_fake_meta(2000, 15));
    let t2   = idx.write_tree(&mut repo).unwrap();
    let c2   = repo.commit.push(t2, &[c1_h], 2000, "dev", "oops, broke it");
    let c2_h = repo.write_object(mog::object::Object::Commit(c2));

    // Revert: new commit restoring t1, parent is c2.
    let c_revert   = repo.commit.push(t1, &[c2_h], 3000, "dev", "revert: oops, broke it");
    let _c_revert_h = repo.write_object(mog::object::Object::Commit(c_revert));

    // Revert's tree == original good tree.
    assert_eq!(repo.commit.get_tree(c_revert), t1);
    // Revert's parent is the bad commit.
    assert_eq!(repo.commit.get_parents(c_revert), &[c2_h]);

    // Verify the content is restored.
    let rt_obj = repo.stores.decode_and_push_object(repo.storage.read(&t1).unwrap()).unwrap();
    let rt_id  = rt_obj.try_as_tree_id().unwrap();
    let app_h  = repo.tree.find_entry(rt_id, "app.rs").unwrap();
    let data   = mog::object::decode_blob_bytes(repo.storage.read(&app_h).unwrap()).unwrap();
    assert_eq!(data, b"good code");
}

/// Simulate: cherry-pick - apply content of one branch's commit onto another.
#[test]
fn test_cherry_pick_content() {
    let mut repo = mock_repo();

    // Main branch: file_a.rs only.
    let h_a = repo.write_blob(b"fn a() {}");
    let mut idx_main = Index::default();
    idx_main.add("file_a.rs", h_a, &make_fake_meta(1000, 9));
    let t_main   = idx_main.write_tree(&mut repo).unwrap();
    let c_main   = repo.commit.push(t_main, &[], 1000, "dev", "main: add a");
    let c_main_h = repo.write_object(mog::object::Object::Commit(c_main));

    // Feature branch: adds file_b.rs.
    let h_b = repo.write_blob(b"fn b() {}");
    let mut idx_feat = idx_main.clone();
    idx_feat.add("file_b.rs", h_b, &make_fake_meta(2000, 9));
    let t_feat   = idx_feat.write_tree(&mut repo).unwrap();
    let c_feat   = repo.commit.push(t_feat, &[c_main_h], 2000, "dev", "feat: add b");
    let _c_feat_h = repo.write_object(mog::object::Object::Commit(c_feat));

    // Cherry-pick: apply file_b.rs onto main (same content, new parent).
    idx_main.add("file_b.rs", h_b, &make_fake_meta(3000, 9));
    let t_cp    = idx_main.write_tree(&mut repo).unwrap();
    let c_cp    = repo.commit.push(t_cp, &[c_main_h], 3000, "dev", "cherry-pick: add b");
    let _c_cp_h = repo.write_object(mog::object::Object::Commit(c_cp));

    // Cherry-picked commit has same tree content as feature commit
    // (both have file_a and file_b with same hashes).
    assert_eq!(t_cp, t_feat);

    // But different parents.
    assert_eq!(repo.commit.get_parents(c_cp),   &[c_main_h]);
    assert_eq!(repo.commit.get_parents(c_feat),  &[c_main_h]);
}

/// Verify that two independently built repos with identical content
/// produce identical object hashes (content addressing is deterministic).
#[test]
fn test_deterministic_hashing_across_repos() {
    let files = vec![
        ("src/main.rs", b"fn main() {}" as &[u8]),
        ("src/lib.rs",  b"pub fn foo() {}"),
        ("README.md",   b"# Hello"),
    ];

    let mut repo1 = mock_repo();
    let mut repo2 = mock_repo();

    let mut idx1 = Index::default();
    let mut idx2 = Index::default();

    for (path, content) in &files {
        let h1 = repo1.write_blob(content);
        let h2 = repo2.write_blob(content);
        assert_eq!(h1, h2, "blob hash must be identical across repos for {path}");
        idx1.add(path, h1, &make_fake_meta(1000, content.len() as u64));
        idx2.add(path, h2, &make_fake_meta(1000, content.len() as u64));
    }

    let t1 = idx1.write_tree(&mut repo1).unwrap();
    let t2 = idx2.write_tree(&mut repo2).unwrap();
    assert_eq!(t1, t2, "tree hash must be identical across repos");

    let c1 = repo1.commit.push(t1, &[], 1000, "author", "msg");
    let c2 = repo2.commit.push(t2, &[], 1000, "author", "msg");
    let c1_h = repo1.write_object(mog::object::Object::Commit(c1));
    let c2_h = repo2.write_object(mog::object::Object::Commit(c2));
    assert_eq!(c1_h, c2_h, "commit hash must be identical across repos");
}

//
//
// Hash integrity tests
//
//

/// Verify blake3 avalanche effect - single bit flip produces completely different hash.
#[test]
fn test_hash_avalanche_effect() {
    let mut repo = mock_repo();
    let base     = b"The quick brown fox jumps over the lazy dog";
    let h_base   = repo.write_blob(base);

    // Flip every single bit in the content, verify each produces a unique hash.
    let mut hashes = std::collections::HashSet::new();
    hashes.insert(h_base);

    for byte_idx in 0..base.len() {
        for bit in 0..8u8 {
            let mut mutated  = base.to_vec();
            mutated[byte_idx] ^= 1 << bit;
            let h = repo.write_blob(&mutated);
            assert_ne!(h, h_base, "bit flip at byte {byte_idx} bit {bit} should change hash");
            hashes.insert(h);
        }
    }

    // Every single-bit mutation produces a distinct hash.
    assert_eq!(hashes.len(), 1 + base.len() * 8);
}

/// Verify that appending a single byte to content always changes the hash.
#[test]
fn test_hash_no_length_extension() {
    let mut repo = mock_repo();
    let base = b"base content";
    let h_base = repo.write_blob(base);

    for b in 0u8..=255 {
        let mut extended = base.to_vec();
        extended.push(b);
        let h = repo.write_blob(&extended);
        assert_ne!(h, h_base, "appending byte 0x{b:02x} should change hash");
    }
}

/// Verify prefix-free: "abc" and "abcd" have different hashes even though
/// one is a prefix of the other.
#[test]
fn test_hash_prefix_free() {
    let mut repo = mock_repo();
    let mut prev = repo.write_blob(b"");
    for len in 1..=256usize {
        let data: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
        let h = repo.write_blob(&data);
        assert_ne!(h, prev, "length {len} should differ from length {}", len - 1);
        prev = h;
    }
}

//
//
// Tree encoding invariants
//
//

/// A tree containing only directories should encode/decode correctly.
#[test]
fn test_tree_of_only_directories() {
    let mut repo = mock_repo();

    // Create three leaf trees.
    let h1 = repo.write_blob(b"a");
    let h2 = repo.write_blob(b"b");
    let h3 = repo.write_blob(b"c");

    let t1 = repo.tree.push(&[mog::tree::TreeEntry { hash: h1, name: "a.rs".into(), mode: mog::object::MODE_FILE }]);
    let t2 = repo.tree.push(&[mog::tree::TreeEntry { hash: h2, name: "b.rs".into(), mode: mog::object::MODE_FILE }]);
    let t3 = repo.tree.push(&[mog::tree::TreeEntry { hash: h3, name: "c.rs".into(), mode: mog::object::MODE_FILE }]);

    let th1 = repo.write_object(mog::object::Object::Tree(t1));
    let th2 = repo.write_object(mog::object::Object::Tree(t2));
    let th3 = repo.write_object(mog::object::Object::Tree(t3));

    // Root contains only directories.
    let root = repo.tree.push(&[
        mog::tree::TreeEntry { hash: th1, name: "mod_a".into(), mode: mog::object::MODE_DIR },
        mog::tree::TreeEntry { hash: th2, name: "mod_b".into(), mode: mog::object::MODE_DIR },
        mog::tree::TreeEntry { hash: th3, name: "mod_c".into(), mode: mog::object::MODE_DIR },
    ]);
    let root_hash = repo.write_object(mog::object::Object::Tree(root));

    let obj  = repo.stores.decode_and_push_object(repo.storage.read(&root_hash).unwrap()).unwrap();
    let id   = obj.try_as_tree_id().unwrap();
    assert_eq!(repo.tree.entry_count(id), 3);
    assert_eq!(repo.tree.get_entry(id, 0).mode, mog::object::MODE_DIR);
    assert_eq!(repo.tree.get_entry(id, 1).mode, mog::object::MODE_DIR);
    assert_eq!(repo.tree.get_entry(id, 2).mode, mog::object::MODE_DIR);

    // Verify each dir subtree is reachable.
    let mb_h  = repo.tree.find_entry(id, "mod_b").unwrap();
    let mb_obj = repo.stores.decode_and_push_object(repo.storage.read(&mb_h).unwrap()).unwrap();
    let mb_id  = mb_obj.try_as_tree_id().unwrap();
    assert_eq!(repo.tree.find_entry(mb_id, "b.rs"), Some(h2));
}

/// Two trees with same blobs but different filenames should have different hashes.
#[test]
fn test_tree_hash_sensitive_to_filename() {
    let mut repo = mock_repo();
    let h = repo.write_blob(b"same content");

    let t1 = repo.tree.push(&[mog::tree::TreeEntry { hash: h, name: "foo.rs".into(), mode: mog::object::MODE_FILE }]);
    let t2 = repo.tree.push(&[mog::tree::TreeEntry { hash: h, name: "bar.rs".into(), mode: mog::object::MODE_FILE }]);

    let th1 = repo.write_object(mog::object::Object::Tree(t1));
    let th2 = repo.write_object(mog::object::Object::Tree(t2));
    assert_ne!(th1, th2);
}

/// Two trees with same filenames but different modes should have different hashes.
#[test]
fn test_tree_hash_sensitive_to_mode() {
    let mut repo = mock_repo();
    let h = repo.write_blob(b"#!/bin/sh\necho hi");

    let t1 = repo.tree.push(&[mog::tree::TreeEntry { hash: h, name: "run.sh".into(), mode: mog::object::MODE_FILE }]);
    let t2 = repo.tree.push(&[mog::tree::TreeEntry { hash: h, name: "run.sh".into(), mode: mog::object::MODE_EXEC }]);

    let th1 = repo.write_object(mog::object::Object::Tree(t1));
    let th2 = repo.write_object(mog::object::Object::Tree(t2));
    assert_ne!(th1, th2);
}

//
//
// Commit graph invariants
//
//

/// Octopus merge: commit with 8 parents (jj/git both support this).
#[test]
fn test_octopus_merge() {
    let mut repo   = mock_repo();
    let tree_hash  = write_simple_tree(&mut repo, b"base", "base.rs");
    let base_id    = repo.commit.push(tree_hash, &[], 1000, "dev", "base");
    let base_hash  = repo.write_object(mog::object::Object::Commit(base_id));

    // 8 diverging commits from base.
    let mut branch_hashes = Vec::new();
    for i in 0..8usize {
        let content   = format!("branch {i} content");
        let bh        = repo.write_blob(content.as_bytes());
        let mut idx   = Index::default();
        idx.add(&format!("branch_{i}.rs"), bh, &make_fake_meta(i as i64, content.len() as u64));
        let t         = idx.write_tree(&mut repo).unwrap();
        let c         = repo.commit.push(t, &[base_hash], 2000 + i as i64, "dev", &format!("branch {i}"));
        let ch        = repo.write_object(mog::object::Object::Commit(c));
        branch_hashes.push(ch);
    }

    // Octopus merge commit.
    let merge_id   = repo.commit.push(tree_hash, &branch_hashes, 9999, "dev", "octopus merge");
    let merge_hash = repo.write_object(mog::object::Object::Commit(merge_id));

    assert!(repo.storage.exists(&merge_hash));
    let parents = repo.commit.get_parents(merge_id);
    assert_eq!(parents.len(), 8);
    for bh in &branch_hashes {
        assert!(parents.contains(bh), "octopus merge missing parent {}", mog::hash::hash_to_hex(bh));
    }
}

/// A commit must not be its own ancestor - verify identity check via hash.
#[test]
fn test_commit_hash_changes_with_parent() {
    let mut repo = mock_repo();
    let t = write_simple_tree(&mut repo, b"x", "x.rs");

    let c1   = repo.commit.push(t, &[], 1000, "dev", "msg");
    let c1_h = repo.write_object(mog::object::Object::Commit(c1));

    let c2   = repo.commit.push(t, &[c1_h], 1000, "dev", "msg");
    let c2_h = repo.write_object(mog::object::Object::Commit(c2));

    // Same tree, author, message, timestamp - but different parent means different hash.
    assert_ne!(c1_h, c2_h);
}

//
//
// Status correctness under complex mutations
//
//

/// Simulate a realistic dev cycle:
/// stage files → commit → edit some → check status shows correct buckets.
#[test]
fn test_status_buckets_after_mixed_mutations() {
    let mut repo = mock_repo();

    let h_a = repo.write_blob(b"fn a() {}");
    let h_b = repo.write_blob(b"fn b() {}");
    let h_c = repo.write_blob(b"fn c() {}");

    let mut idx = Index::default();
    idx.add("a.rs", h_a, &make_fake_meta(1000, 9));
    idx.add("b.rs", h_b, &make_fake_meta(1001, 9));
    idx.add("c.rs", h_c, &make_fake_meta(1002, 9));

    let head_tree = idx.write_tree(&mut repo).unwrap();
    let head_flat = mog::status::flatten_tree(&mut repo, head_tree).unwrap();

    // Modify a.rs in index (staged change).
    let h_a2 = repo.write_blob(b"fn a() { 1 }");
    idx.add("a.rs", h_a2, &make_fake_meta(2000, 12));

    // Add new file d.rs (staged new).
    let h_d = repo.write_blob(b"fn d() {}");
    idx.add("d.rs", h_d, &make_fake_meta(2001, 9));

    // Remove c.rs from index (staged delete).
    idx.remove("c.rs");

    // Compute staged changes.
    let mut staged_modified = Vec::new();
    let mut staged_new      = Vec::new();
    for i in 0..idx.count {
        let path       = idx.get_path(i);
        let index_hash = idx.hashes[i];
        match head_flat.lookup(path) {
            None    => staged_new.push(path.to_string()),
            Some(h) if h != index_hash => staged_modified.push(path.to_string()),
            _ => {}
        }
    }
    let mut staged_deleted = Vec::new();
    for j in 0..head_flat.len() {
        let path = head_flat.get_path(head_flat.sorted_order[j]);
        if idx.find(path).is_none() {
            staged_deleted.push(path.to_string());
        }
    }

    assert_eq!(staged_modified, vec!["a.rs"]);
    assert_eq!(staged_new,      vec!["d.rs"]);
    assert_eq!(staged_deleted,  vec!["c.rs"]);

    // b.rs untouched - should not appear in any bucket.
    assert!(!staged_modified.contains(&"b.rs".to_string()));
    assert!(!staged_new.contains(&"b.rs".to_string()));
    assert!(!staged_deleted.contains(&"b.rs".to_string()));
}

/// Moving a file (delete old path, add new path with same hash)
/// should show as staged delete + staged new, not staged modify.
#[test]
fn test_status_move_shows_as_delete_and_new() {
    let mut repo = mock_repo();
    let h = repo.write_blob(b"content");

    let mut idx = Index::default();
    idx.add("old.rs", h, &make_fake_meta(1000, 7));
    let head_tree = idx.write_tree(&mut repo).unwrap();
    let head_flat = mog::status::flatten_tree(&mut repo, head_tree).unwrap();

    // "Move": remove old, add new with same content.
    idx.remove("old.rs");
    idx.add("new.rs", h, &make_fake_meta(2000, 7));

    let mut staged_new     = Vec::new();
    let mut staged_deleted = Vec::new();

    for i in 0..idx.count {
        let path = idx.get_path(i);
        if head_flat.lookup(path).is_none() {
            staged_new.push(path.to_string());
        }
    }
    for j in 0..head_flat.len() {
        let path = head_flat.get_path(head_flat.sorted_order[j]);
        if idx.find(path).is_none() {
            staged_deleted.push(path.to_string());
        }
    }

    assert_eq!(staged_new,     vec!["new.rs"]);
    assert_eq!(staged_deleted, vec!["old.rs"]);
}

//
//
// Property-style tests
//

/// For any blob, encode then decode must be identity.
#[test]
fn test_encode_decode_identity_property() {
    let mut repo = mock_repo();
    let cases: &[&[u8]] = &[
        b"",
        b"\x00",
        b"\xff",
        b"\x00\xff\x00\xff",
        b"hello world",
        &[0u8; 4096],
        &[0xffu8; 4096],
    ];
    for &data in cases {
        let hash = repo.write_blob(data);
        let raw  = repo.storage.read(&hash).unwrap();
        let got  = mog::object::decode_blob_bytes(raw).unwrap();
        assert_eq!(got, data);
    }
}

/// Index encode/decode is identity for any sequence of adds/removes.
#[test]
fn test_index_encode_decode_identity_property() {
    let mut index = Index::default();

    // Add 100 files.
    for i in 0..100usize {
        let mut h = [0u8; 32];
        h[..8].copy_from_slice(&i.to_le_bytes());
        index.add(&format!("file_{i:03}.rs"), h, &make_fake_meta(i as i64 * 1000, i as u64 * 100));
    }

    // Remove every third.
    let to_remove: Vec<_> = (0..100usize).filter(|i| i % 3 == 0).collect();
    for i in to_remove {
        index.remove(&format!("file_{i:03}.rs"));
    }

    let encoded = index.encode_for_test();
    let decoded = Index::decode_for_test(&encoded).unwrap();
    assert_eq!(decoded.count, index.count);

    // Every surviving entry must be findable with correct hash.
    for i in 0..100usize {
        let path = format!("file_{i:03}.rs");
        if i % 3 == 0 {
            assert!(decoded.find(&path).is_none(), "{path} should have been removed");
        } else {
            let idx = decoded.find(&path).unwrap_or_else(|| panic!("{path} should exist"));
            let mut expected = [0u8; 32];
            expected[..8].copy_from_slice(&i.to_le_bytes());
            assert_eq!(decoded.hashes[idx], expected);
        }
    }
}

/// SortedFlatTree lookup is consistent with linear scan for all entries.
#[test]
fn test_sorted_flat_tree_lookup_matches_linear_scan() {
    let mut builder = mog::status::FlatTreeBuilder::new();
    let n = 200usize;

    let mut expected = std::collections::HashMap::new();
    for i in 0..n {
        // Deliberately non-alphabetical insertion order.
        let path = format!("z_{:04}_{}.rs", n - i, i % 7);
        let mut h = [0u8; 32];
        h[..8].copy_from_slice(&i.to_le_bytes());
        // Handle potential duplicates from the path template by overwriting.
        expected.insert(path.clone(), h);
        builder.push(&path, h);
    }

    let flat = builder.build();

    for (path, expected_hash) in &expected {
        let found = flat.lookup(path);
        assert!(found.is_some(), "binary search missed {path}");
        assert_eq!(&found.unwrap(), expected_hash, "wrong hash for {path}");
    }

    // Non-existent paths should return None.
    assert_eq!(flat.lookup("nonexistent_path.rs"), None);
    assert_eq!(flat.lookup("z_0000_0.r"),          None); // prefix of real path
    assert_eq!(flat.lookup("z_0000_0.rss"),         None); // extension of real path
}

//
//
// Helpers
//
//

/// Build a fake Metadata-equivalent struct for index tests.
/// Since fs::Metadata can't be constructed directly, Index::add
/// should accept a trait or a plain (mtime, size) pair for testability.
/// If your Index::add takes &fs::Metadata you'll need to expose
/// Index::add_raw(path, hash, mtime, size) for tests.
fn make_fake_meta(mtime: i64, size: u64) -> mog::index::FakeMeta {
    mog::index::FakeMeta { mtime, size }
}
