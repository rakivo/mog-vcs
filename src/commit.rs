use crate::repository::Repository;
use crate::object::Object;
use crate::store::{CommitId, CommitStore};
use crate::hash::{Hash, hash_to_hex};
use crate::wire::{Decode, Encode, ReadCursor, WriteCursor};

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;

pub fn commit(
    repo: &mut Repository,
    tree: Hash,
    parent: Option<Hash>,
    author: &str,
    message: &str,
) -> Result<Hash> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_secs() as i64;

    let parents = parent.into_iter().collect::<Vec<_>>();
    let commit_id = repo.commit.push(tree, &parents, timestamp, author, message);
    let hash = repo.write_object(Object::Commit(commit_id));

    let head = fs::read_to_string(repo.root.join(".mog/HEAD"))?;
    let head = head.trim();

    if let Some(refpath) = head.strip_prefix("ref: ") {
        //
        // Normal: update the branch HEAD points to
        //
        repo.write_ref(refpath.trim(), &hash)?;
    } else {
        //
        // Detached HEAD: update HEAD directly to new commit
        //
        fs::write(
            repo.root.join(".mog/HEAD"),
            format!("{}\n", hash_to_hex(&hash))
        )?;
        println!("Warning: committing in detached HEAD state");
        println!("Create a branch to keep this work: mog branch save-my-work");
    }

    println!("Created commit {}", hash_to_hex(&hash));

    //
    // Ensure commit (and any trees written along the way) are durably stored.
    //
    repo.storage.flush()?;
    Ok(hash)
}

crate::payload_triple! {
    owned CommitPayloadOwned {
        tree: Hash,
        parents: Box<[Hash]>,
        timestamp: i64,
        author: Box<str>,
        message: Box<str>,
    }
    view CommitPayloadView<'a> {
        tree: Hash,
        parents: &'a [Hash],
        timestamp: i64,
        author: &'a str,
        message: &'a str,
    }
    ref CommitPayloadRef<'a> {
        store: &'a CommitStore,
        id: CommitId,
    }
    view_from_owned(o) {
        CommitPayloadView {
            tree: o.tree,
            parents: &o.parents,
            timestamp: o.timestamp,
            author: &o.author,
            message: &o.message,
        }
    }
    view_from_ref(r) {
        CommitPayloadView {
            tree: r.store.get_tree(r.id),
            parents: r.store.get_parents(r.id),
            timestamp: r.store.get_timestamp(r.id),
            author: r.store.get_author(r.id),
            message: r.store.get_message(r.id),
        }
    }
}

impl CommitPayloadOwned {
    #[must_use]
    pub fn new(tree: Hash, parents: Box<[Hash]>, timestamp: i64, author: Box<str>, message: Box<str>) -> Self {
        Self {
            tree,
            parents,
            timestamp,
            author,
            message,
        }
    }
}

impl Decode for CommitPayloadOwned {
    fn decode(r: &mut ReadCursor<'_>) -> Result<Self> {
        let tree = r.read_hash()?;

        let parent_count = r.read_u32()? as usize;
        let mut parents = Vec::with_capacity(parent_count);
        for _ in 0..parent_count {
            parents.push(r.read_hash()?);
        }

        let timestamp = r.read_i64()?;

        let author = r.read_len_prefixed_str()?.into_owned();

        let message = r.read_len_prefixed_str()?.into_owned();

        Ok(CommitPayloadOwned::new(
            tree,
            parents.into_boxed_slice(),
            timestamp,
            author.into(),
            message.into(),
        ))
    }
}

impl<'a> CommitPayloadRef<'a> {
    #[must_use]
    pub fn new(store: &'a CommitStore, id: CommitId) -> Self {
        Self { store, id }
    }
}

impl Encode for CommitPayloadView<'_> {
    fn encode(&self, w: &mut WriteCursor<'_>) {
        w.write_hash(&self.tree);

        w.write_u32(self.parents.len() as u32);
        for p in self.parents {
            w.write_hash(p);
        }

        w.write_i64(self.timestamp);

        w.write_len_prefixed_str(self.author);

        w.write_len_prefixed_str(self.message);
    }
}
