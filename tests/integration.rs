use std::fs;
use std::path::Path;
use tempfile::TempDir;
use anyhow::Result;

// @Note: These are AI generated, but it's fine for a start I guess...

//
//
// Init
//
//

#[test]
fn test_init_creates_mog_dir() {
    let dir  = TempDir::new().unwrap();
    let root = dir.path();
    mog::repository::Repository::init(root).unwrap();
    assert!(root.join(".mog").exists());
    assert!(root.join(".mog/objects.bin").exists());
}

#[test]
fn test_init_twice_does_not_destroy_data() {
    let (_dir, root) = setup();
    write_file(&root, "file.rs", b"content");
    stage_all(&root);
    commit_all(&root, "first");
    // Re-init should not wipe objects.
    mog::repository::Repository::init(&root).unwrap();
    let repo = open(&root);
    // HEAD ref should still be readable.
    assert!(repo.read_head_commit().is_ok());
}

//
//
// Stage / Unstage
//
//

#[test]
fn test_stage_single_file() {
    let (_dir, root) = setup();
    write_file(&root, "hello.rs", b"fn hello() {}");
    stage_all(&root);
    let index = mog::index::Index::load(&root).unwrap();
    assert_eq!(index.count, 1 + 1);  // + default .mogged file
    assert!(index.find("hello.rs").is_some());
}

#[test]
fn test_stage_nested_files() {
    let (_dir, root) = setup();
    write_file(&root, "src/main.rs", b"fn main() {}");
    write_file(&root, "src/lib.rs",  b"pub fn foo() {}");
    write_file(&root, "README.md",   b"# Hello");
    stage_all(&root);
    let index = mog::index::Index::load(&root).unwrap();
    assert_eq!(index.count, 3 + 1);  // + default .mogged file
    assert!(index.find("src/main.rs").is_some());
    assert!(index.find("src/lib.rs").is_some());
    assert!(index.find("README.md").is_some());
}

#[test]
fn test_stage_is_idempotent() {
    let (_dir, root) = setup();
    write_file(&root, "file.rs", b"content");
    stage_all(&root);
    stage_all(&root);
    stage_all(&root);
    let index = mog::index::Index::load(&root).unwrap();
    assert_eq!(index.count, 1 + 1);  // + default .mogged file
}

#[test]
fn test_stage_updated_file() {
    let (_dir, root) = setup();
    write_file(&root, "file.rs", b"v1");
    stage_all(&root);
    let idx1 = mog::index::Index::load(&root).unwrap();
    let h1   = idx1.hashes[idx1.find("file.rs").unwrap()];

    write_file(&root, "file.rs", b"v2");
    touch_future(&root, "file.rs");
    stage_all(&root);
    let idx2 = mog::index::Index::load(&root).unwrap();
    let h2   = idx2.hashes[idx2.find("file.rs").unwrap()];

    assert_ne!(h1, h2);
    assert_eq!(idx2.count, 1 + 1);  // + default .mogged file
}

#[test]
fn test_stage_removes_deleted_files_from_index() {
    let (_dir, root) = setup();
    write_file(&root, "a.rs", b"aaa");
    write_file(&root, "b.rs", b"bbb");
    stage_all(&root);

    // Delete b.rs from disk and re-stage.
    fs::remove_file(root.join("b.rs")).unwrap();
    stage_all(&root);

    let index = mog::index::Index::load(&root).unwrap();
    assert_eq!(index.count, 1 + 1);  // + default .mogged file
    assert!(index.find("a.rs").is_some());
    assert!(index.find("b.rs").is_none());
}

#[test]
fn test_unstage_specific_file() {
    let (_dir, root) = setup();
    write_file(&root, "a.rs", b"aaa");
    write_file(&root, "b.rs", b"bbb");
    stage_all(&root);

    let mut repo = open(&root);
    mog::unstage::unstage(&mut repo, &[std::path::PathBuf::from("a.rs")]).unwrap();

    let index = mog::index::Index::load(&root).unwrap();
    assert_eq!(index.count, 1 + 1);  // + default .mogged file
    assert!(index.find("a.rs").is_none());
    assert!(index.find("b.rs").is_some());
}

#[test]
fn test_unstage_all() {
    let (_dir, root) = setup();
    write_file(&root, "a.rs", b"aaa");
    write_file(&root, "b.rs", b"bbb");
    stage_all(&root);

    let mut repo = open(&root);
    mog::unstage::unstage(&mut repo, &[]).unwrap();

    let index = mog::index::Index::load(&root).unwrap();
    assert_eq!(index.count, 0);
}

//
//
// Commit
//
//

#[test]
fn test_commit_creates_head() {
    let (_dir, root) = setup();
    write_file(&root, "file.rs", b"content");
    stage_all(&root);
    commit_all(&root, "initial");
    let repo = open(&root);
    assert!(repo.read_head_commit().is_ok());
}

#[test]
fn test_commit_chain_head_advances() {
    let (_dir, root) = setup();
    write_file(&root, "file.rs", b"v1");
    stage_all(&root);
    let h1 = commit_all(&root, "first");

    write_file(&root, "file.rs", b"v2");
    stage_all(&root);
    let h2 = commit_all(&root, "second");

    assert_ne!(h1, h2);
    let repo   = open(&root);
    let head   = repo.read_head_commit().unwrap();
    assert_eq!(head, h2);
}

#[test]
fn test_commit_stores_correct_tree() {
    let (_dir, root) = setup();
    write_file(&root, "src/main.rs", b"fn main() {}");
    write_file(&root, "README.md",   b"# Readme");
    stage_all(&root);
    commit_all(&root, "init");

    let mut repo    = open(&root);
    let head_hash   = repo.read_head_commit().unwrap();
    let obj         = repo.read_object(&head_hash).unwrap();
    let commit_id   = obj.try_as_commit_id().unwrap();
    let tree_hash   = repo.commit.get_tree(commit_id);

    let tree_obj    = repo.read_object(&tree_hash).unwrap();
    let tree_id     = tree_obj.try_as_tree_id().unwrap();

    // Root should have src/ and README.md
    assert!(repo.tree.find_entry(tree_id, "src").is_some());
    assert!(repo.tree.find_entry(tree_id, "README.md").is_some());
}

//
//
// Status
//
//

#[test]
fn test_status_clean_working_tree() -> Result<()> {
    let (_dir, root) = setup();
    write_file(&root, "file.rs", b"content");
    stage_all(&root);
    commit_all(&root, "init");

    let mut repo = open(&root);
    let buckets  = mog::status::collect_status(&mut repo)?;
    assert!(buckets.staged_new_modified.is_empty());
    assert!(buckets.staged_deleted.is_empty());
    assert!(buckets.modified.is_empty());
    assert!(buckets.deleted.is_empty());
    assert!(buckets.untracked.is_empty());
    Ok(())
}

#[test]
fn test_status_detects_untracked() -> Result<()> {
    let (_dir, root) = setup();
    write_file(&root, "file.rs", b"content");

    let mut repo = open(&root);
    let buckets  = mog::status::collect_status(&mut repo)?;
    assert!(buckets.untracked.iter().any(|p| p.as_ref() == "file.rs"));
    Ok(())
}

#[test]
fn test_status_detects_staged_new() -> Result<()> {
    let (_dir, root) = setup();
    write_file(&root, "file.rs", b"content");
    stage_all(&root);
    commit_all(&root, "base");

    write_file(&root, "new.rs", b"new");
    stage_all(&root);

    let mut repo = open(&root);
    let buckets  = mog::status::collect_status(&mut repo)?;
    assert!(buckets.staged_new_modified.iter().any(|p| p.as_ref() == "new.rs"));
    Ok(())
}

#[test]
fn test_status_detects_modified_on_disk() -> Result<()> {
    let (_dir, root) = setup();
    write_file(&root, "file.rs", b"original");
    stage_all(&root);
    commit_all(&root, "base");

    // Modify on disk but don't stage.
    write_file(&root, "file.rs", b"modified");
    touch_future(&root, "file.rs");

    let mut repo = open(&root);
    let buckets  = mog::status::collect_status(&mut repo)?;
    assert!(buckets.modified.iter().any(|p| p.as_ref() == "file.rs"));
    Ok(())
}

#[test]
fn test_status_detects_deleted_on_disk() -> Result<()> {
    let (_dir, root) = setup();
    write_file(&root, "file.rs", b"content");
    stage_all(&root);
    commit_all(&root, "base");

    fs::remove_file(root.join("file.rs")).unwrap();

    let mut repo = open(&root);
    let buckets  = mog::status::collect_status(&mut repo)?;
    assert!(buckets.deleted.iter().any(|p| p.as_ref() == "file.rs"));
    Ok(())
}

//
//
// Checkout
//
//

#[test]
fn test_checkout_restores_files() {
    let (_dir, root) = setup();
    write_file(&root, "file.rs", b"v1");
    stage_all(&root);
    commit_all(&root, "v1");

    // Create and switch to branch, modify file.
    let mut repo = open(&root);
    mog::branch::create(&mut repo, "feature", None).unwrap();
    mog::checkout::checkout(&mut repo, "feature").unwrap();

    write_file(&root, "file.rs", b"v2");
    touch_future(&root, "file.rs");
    stage_all(&root);
    commit_all(&root, "v2 on feature");

    // Switch back to main.
    let mut repo = open(&root);
    mog::checkout::checkout(&mut repo, "main").unwrap();

    assert_eq!(read_file(&root, "file.rs"), b"v1");
}

#[test]
fn test_checkout_new_branch() {
    let (_dir, root) = setup();
    write_file(&root, "file.rs", b"content");
    stage_all(&root);
    commit_all(&root, "init");

    let mut repo = open(&root);
    mog::branch::create(&mut repo, "feature", None).unwrap();
    mog::checkout::checkout(&mut repo, "feature").unwrap();

    // HEAD should now point to feature.
    let head_branch = fs::read_to_string(root.join(".mog/HEAD")).unwrap();
    assert!(head_branch.contains("feature"));
}

#[test]
fn test_checkout_creates_missing_files() {
    let (_dir, root) = setup();

    // Commit with two files on main.
    write_file(&root, "a.rs", b"aaa");
    write_file(&root, "b.rs", b"bbb");
    stage_all(&root);
    commit_all(&root, "base");

    // Feature branch: delete b.rs.
    let mut repo = open(&root);
    mog::branch::create(&mut repo, "feature", None).unwrap();
    mog::checkout::checkout(&mut repo, "feature").unwrap();
    fs::remove_file(root.join("b.rs")).unwrap();
    stage_all(&root);
    commit_all(&root, "remove b");

    assert!(!file_exists(&root, "b.rs"));

    // Back to main: b.rs should reappear.
    let mut repo = open(&root);
    mog::checkout::checkout(&mut repo, "main").unwrap();
    assert!(file_exists(&root, "b.rs"));
    assert_eq!(read_file(&root, "b.rs"), b"bbb");
}

//
//
// Branch
//
//

#[test]
fn test_branch_create_and_list() {
    let (_dir, root) = setup();
    write_file(&root, "f.rs", b"x");
    stage_all(&root);
    commit_all(&root, "init");

    let mut repo = open(&root);
    mog::branch::create(&mut repo, "feature", None).unwrap();

    let branches_path = root.join(".mog/refs/heads");
    let branches: Vec<_> = fs::read_dir(&branches_path).unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().into_string().unwrap())
        .collect();

    assert!(branches.contains(&"main".to_string()) || branches.contains(&"main".to_string()));
    assert!(branches.contains(&"feature".to_string()));
}

#[test]
fn test_branch_delete() {
    let (_dir, root) = setup();
    write_file(&root, "f.rs", b"x");
    stage_all(&root);
    commit_all(&root, "init");

    let mut repo = open(&root);
    mog::branch::create(&mut repo, "to-delete", None).unwrap();
    mog::branch::delete(&mut repo, "to-delete").unwrap();

    assert!(!root.join(".mog/refs/heads/to-delete").exists());
}

//
//
// Discard
//
//

#[test]
fn test_discard_restores_modified_file() {
    let (_dir, root) = setup();
    write_file(&root, "file.rs", b"original");
    stage_all(&root);

    write_file(&root, "file.rs", b"modified");
    assert_eq!(read_file(&root, "file.rs"), b"modified");

    let mut repo = open(&root);
    mog::discard::discard(&mut repo, &[]).unwrap();

    assert_eq!(read_file(&root, "file.rs"), b"original");
}

#[test]
fn test_discard_removes_untracked_files() {
    let (_dir, root) = setup();
    write_file(&root, "tracked.rs",   b"tracked");
    stage_all(&root);
    write_file(&root, "untracked.rs", b"untracked");

    let mut repo = open(&root);
    mog::discard::discard(&mut repo, &[]).unwrap();

    assert!(file_exists(&root, "tracked.rs"));
    assert!(!file_exists(&root, "untracked.rs"));
}

#[test]
fn test_discard_specific_path() {
    let (_dir, root) = setup();
    write_file(&root, "a.rs", b"original_a");
    write_file(&root, "b.rs", b"original_b");
    stage_all(&root);

    write_file(&root, "a.rs", b"modified_a");
    write_file(&root, "b.rs", b"modified_b");

    let mut repo = open(&root);
    mog::discard::discard(&mut repo, &[std::path::PathBuf::from("a.rs")]).unwrap();

    assert_eq!(read_file(&root, "a.rs"), b"original_a");
    assert_eq!(read_file(&root, "b.rs"), b"modified_b"); // b untouched
}

//
//
// Stash
//
//

#[test]
fn test_stash_save_and_pop() {
    let (_dir, root) = setup();
    write_file(&root, "file.rs", b"original");
    stage_all(&root);
    commit_all(&root, "base");

    // Modify on disk but DON'T stage — this is the dirty state stash should capture.
    write_file_later(&root, "file.rs", b"modified");
    // Do NOT call stage_all here.

    let mut repo = open(&root);
    mog::stash::stash(&mut repo).unwrap();
    assert_eq!(read_file(&root, "file.rs"), b"original");

    let mut repo = open(&root);
    mog::stash::stash_pop(&mut repo).unwrap();
    assert_eq!(read_file(&root, "file.rs"), b"modified");
}

#[test]
fn test_stash_save_no_changes_is_noop() {
    let (_dir, root) = setup();
    write_file(&root, "file.rs", b"content");
    stage_all(&root);
    commit_all(&root, "base");

    let mut repo = open(&root);
    mog::stash::stash(&mut repo).unwrap();

    // No stash ref should have been created.
    assert!(!root.join(".mog/refs/stash/0").exists());
}

#[test]
fn test_stash_multiple_and_pop_order() {
    let (_dir, root) = setup();
    write_file(&root, "file.rs", b"base");
    stage_all(&root);
    commit_all(&root, "base");

    // Stash 1.
    write_file(&root, "file.rs", b"change1");
    stage_all(&root);
    let mut repo = open(&root);
    mog::stash::stash(&mut repo).unwrap();

    // Stash 2.
    write_file(&root, "file.rs", b"change2");
    stage_all(&root);
    let mut repo = open(&root);
    mog::stash::stash(&mut repo).unwrap();

    assert!(root.join(".mog/refs/stash/0").exists());
    assert!(root.join(".mog/refs/stash/1").exists());

    // Pop order: most recent first.
    let mut repo = open(&root);
    mog::stash::stash_pop(&mut repo).unwrap();
    assert_eq!(read_file(&root, "file.rs"), b"change2");

    let mut repo = open(&root);
    mog::stash::stash_pop(&mut repo).unwrap();
    assert_eq!(read_file(&root, "file.rs"), b"change1");

    assert!(!root.join(".mog/refs/stash/0").exists());
}

#[test]
fn test_stash_apply_leaves_stash_intact() {
    let (_dir, root) = setup();
    write_file(&root, "file.rs", b"original");
    stage_all(&root);
    commit_all(&root, "base");

    write_file(&root, "file.rs", b"modified");
    touch_future(&root, "file.rs");
    stage_all(&root);

    let mut repo = open(&root);
    mog::stash::stash(&mut repo).unwrap();
    let mut repo = open(&root);
    mog::stash::stash_apply(&mut repo, 0).unwrap();

    // Stash ref still there.
    assert!(root.join(".mog/refs/stash/0").exists());
    assert_eq!(read_file(&root, "file.rs"), b"modified");
}

#[test]
fn test_stash_drop() {
    let (_dir, root) = setup();
    write_file(&root, "file.rs", b"content");
    stage_all(&root);
    commit_all(&root, "base");

    write_file(&root, "file.rs", b"modified");
    stage_all(&root);
    let mut repo = open(&root);
    mog::stash::stash(&mut repo).unwrap();

    assert!(root.join(".mog/refs/stash/0").exists());
    mog::stash::stash_drop(&open(&root), 0).unwrap();
    assert!(!root.join(".mog/refs/stash/0").exists());
}

//
//
// Log
//
//

#[test]
fn test_log_shows_commits_in_order() {
    let (_dir, root) = setup();
    write_file(&root, "f.rs", b"v1");
    stage_all(&root);
    commit_all(&root, "first commit");

    write_file(&root, "f.rs", b"v2");
    stage_all(&root);
    commit_all(&root, "second commit");

    let mut repo = open(&root);
    let mut buf  = String::new();
    mog::log::log(&mut repo, &mut buf).unwrap();

    let first_pos  = buf.find("first commit").unwrap();
    let second_pos = buf.find("second commit").unwrap();

    // Most recent first.
    assert!(second_pos < first_pos);
}

//
//
// Full end-to-end workflow
//
//

#[test]
fn test_full_dev_workflow() {
    let (_dir, root) = setup();

    // Initial commit.
    write_file(&root, "src/main.rs", b"fn main() {}");
    write_file(&root, "README.md",   b"# Project");
    stage_all(&root);
    commit_all(&root, "initial commit");

    // Feature branch.
    let mut repo = open(&root);
    mog::branch::create(&mut repo, "feature", None).unwrap();
    mog::checkout::checkout(&mut repo, "feature").unwrap();

    write_file(&root, "src/feature.rs", b"pub fn feature() {}");
    stage_all(&root);
    commit_all(&root, "add feature");

    // Stash mid-work change.
    write_file(&root, "src/wip.rs", b"// work in progress");
    stage_all(&root);
    touch_future(&root, "src/wip.rs");
    let mut repo = open(&root);
    mog::stash::stash(&mut repo).unwrap();

    assert!(!file_exists(&root, "src/wip.rs"));

    // Switch back to main, verify feature.rs absent.
    let mut repo = open(&root);
    mog::checkout::checkout(&mut repo, "main").unwrap();
    assert!(!file_exists(&root, "src/feature.rs"));
    assert_eq!(read_file(&root, "src/main.rs"), b"fn main() {}");

    // Back to feature, pop stash.
    let mut repo = open(&root);
    mog::checkout::checkout(&mut repo, "feature").unwrap();
    let mut repo = open(&root);
    mog::stash::stash_pop(&mut repo).unwrap();
    assert!(file_exists(&root, "src/wip.rs"));

    // Discard wip.
    let mut repo = open(&root);
    mog::discard::discard(&mut repo, &[std::path::PathBuf::from("src/wip.rs")]).unwrap();
    assert!(!file_exists(&root, "src/wip.rs"));

    // Status should be clean.
    let mut repo = open(&root);
    let buckets  = mog::status::collect_status(&mut repo).unwrap();
    assert!(buckets.staged_new_modified.is_empty());
    assert!(buckets.modified.is_empty());
    assert!(buckets.untracked.is_empty());
}


//
//
// Helpers
//
//

fn setup() -> (TempDir, std::path::PathBuf) {
    let dir  = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    mog::repository::Repository::init(&root).unwrap();
    (dir, root)
}

fn open(root: &Path) -> mog::repository::Repository {
    mog::repository::Repository::open(root).unwrap()
}

fn write_file(root: &Path, rel: &str, content: &[u8]) {
    let abs = root.join(rel);
    if let Some(parent) = abs.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&abs, content).unwrap();
    // Force mtime to change even within the same second.
    let mtime = std::time::SystemTime::now() + std::time::Duration::from_secs(1);
    filetime::set_file_mtime(&abs, filetime::FileTime::from_system_time(mtime)).unwrap();
}

fn write_file_later(root: &Path, rel: &str, content: &[u8]) {
    std::thread::sleep(std::time::Duration::from_millis(1100));
    write_file(root, rel, content);
}

fn touch_future(root: &Path, rel: &str) {
    let abs    = root.join(rel);
    let future = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
    filetime::set_file_mtime(&abs, filetime::FileTime::from_system_time(future)).unwrap();
}

#[track_caller]
fn read_file(root: &Path, rel: &str) -> Vec<u8> {
    fs::read(root.join(rel)).unwrap()
}

fn file_exists(root: &Path, rel: &str) -> bool {
    root.join(rel).exists()
}

fn stage_all(root: &Path) {
    let mut repo = open(root);
    mog::stage::stage(&mut repo, &[root.to_path_buf()]).unwrap();
}

fn commit_all(root: &Path, message: &str) -> mog::hash::Hash {
    let mut repo  = open(root);
    let index     = mog::index::Index::load(&repo.root).unwrap();
    let tree      = index.write_tree(&mut repo).unwrap();
    let parent    = repo.read_head_commit().ok();
    mog::commit::commit(&mut repo, tree, parent, "test", message).unwrap()
}
