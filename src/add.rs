use std::fs;
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::ignore::Ignore;
use crate::tracy;
use crate::hash::Hash;
use crate::index::Index;
use crate::repository::Repository;
use crate::object::encode_blob_into;

use anyhow::Result;
use rayon::prelude::*;
use walkdir::WalkDir;
use regex::Regex;

const ADD_BATCH_MAX_BYTES: usize = 1024 * 1024;
const ADD_MAX_FILE_BYTES:  usize = 1024 * 1024;

pub fn add(repo: &mut Repository, paths: &[PathBuf]) -> Result<()> {
    let _span = tracy::span!("add");

    let added_successfully       = AtomicUsize::new(0); // @Metric
    let bytes_added_successfully = AtomicUsize::new(0); // @Metric
    let refused_over_limit       = AtomicUsize::new(0); // @Metric

    let current_dir = std::env::current_dir()?;
    let mut index   = Index::load(&repo.root)?;

    //
    //
    // Classify patterns into literal roots or regexes.
    //
    //

    let default = [PathBuf::from(".")];
    let patterns = if paths.is_empty() { &default } else { paths };
    let (literal_roots, combined_re) = classify_patterns(patterns, &current_dir);

    //
    //
    // Collect candidate files.
    //
    //

    let files_to_add = walk_matching(&repo.root, &repo.ignore, &literal_roots, combined_re.as_ref());

    //
    //
    // Filter to dirty files within the size limit.
    //
    //

    let mut files_to_process = Vec::<FileMeta>::new();

    for (path, rel_norm_string) in files_to_add {
        if repo.ignore.is_ignored_rel(&rel_norm_string) {
            continue;
        }

        let metadata = match fs::metadata(&path) {
            Ok(m)  => m,
            Err(e) => {
                eprintln!("metadata error for {}: {}", path.display(), e);
                continue;
            }
        };

        if metadata.len() > ADD_MAX_FILE_BYTES as u64 {
            refused_over_limit.fetch_add(1, Ordering::Relaxed);
            continue;
        }

        if let Some(i) = index.find(rel_norm_string.as_ref()) {
            if !index.is_dirty(i, &metadata) {
                continue;
            }
        }

        files_to_process.push(FileMeta {
            path: path.into(),
            rel_norm: PathBuf::from(rel_norm_string.into_string()).into(),
            meta: metadata,
        });
    }

    if refused_over_limit.load(Ordering::Relaxed) > 0 {
        eprintln!(
            "Refused to add {refused_over_limit} file(s) over 1 MiB (max {ADD_MAX_FILE_BYTES} bytes)",
            refused_over_limit = refused_over_limit.load(Ordering::Relaxed)
        );
    }

    //
    //
    // Split into size-bounded batches, then read/encode/hash in parallel within each.
    //
    //

    let mut batches: Vec<Vec<&FileMeta>> = vec![Vec::new()];
    let mut current_batch_bytes = 0usize;

    for file in &files_to_process {
        let size = file.meta.len() as usize;
        if current_batch_bytes + size > ADD_BATCH_MAX_BYTES && !batches.last().unwrap().is_empty() {
            batches.push(Vec::new());
            current_batch_bytes = 0;
        }

        batches.last_mut().unwrap().push(file);
        current_batch_bytes += size;
    }

    for batch in batches {
        //
        // Read, encode, and hash in parallel.
        //
        let processed = batch.into_par_iter().filter_map(|file| {
            let data = match fs::read(&file.path) {
                Ok(d)  => d,
                Err(e) => {
                    eprintln!("read error for {}: {}", file.path.display(), e);
                    return None;
                }
            };

            let mut encoded = Vec::new();
            encode_blob_into(&data, &mut encoded);
            let hash = {
                let _span = tracy::span!("add::hash");
                Hash::from(blake3::hash(&encoded))
            };

            added_successfully.fetch_add(1, Ordering::Relaxed);
            bytes_added_successfully.fetch_add(data.len(), Ordering::Relaxed);

            Some(ProcessedFile {
                file_meta: file,
                encoded: crate::util::vec_into_boxed_slice_noshrink(encoded),
                hash,
            })
        }).collect::<Vec<_>>();

        //
        // Build encoded_buf and flush.
        //
        let mut encoded_buf = Vec::new();
        let mut file_infos  = Vec::<FileInfo>::new();
        let mut file_metas  = Vec::<&FileMeta>::new();

        for ProcessedFile { file_meta, encoded, hash } in processed {
            let offset = encoded_buf.len() as u32;
            let len    = encoded.len() as u32;
            encoded_buf.extend_from_slice(&encoded);
            file_infos.push(FileInfo { hash, offset, len });
            file_metas.push(file_meta);
        }

        flush_batch(repo, &mut index, &encoded_buf, &file_infos, &file_metas)?;
    }

    repo.storage.sync()?;
    index.save(&repo.root)?;

    println!(
        "Added {added_successfully} file(s), {bytes_added_successfully} in byte(s)",
        added_successfully = added_successfully.load(Ordering::Relaxed),
        bytes_added_successfully = bytes_added_successfully.load(Ordering::Relaxed),
    );

    Ok(())
}

//
//
// Shared pattern matching helpers. (add and remove share some functions)
//
//

pub fn classify_patterns(
    patterns:      &[PathBuf],
    current_dir:   &Path,
) -> (Vec<PathBuf>, Option<Regex>) {
    let mut literal_roots  = Vec::new();
    let mut regex_patterns = Vec::new();

    for p in patterns {
        let candidate = if p.is_absolute() {
            Cow::Borrowed(p)
        } else {
            Cow::Owned(current_dir.join(p))
        };

        if candidate.exists() {
            //
            // Canonicalize once here so we don't repeat it per-file in the walk.
            //
            match candidate.canonicalize() {
                Ok(canon) => literal_roots.push(canon),
                Err(e)    => eprintln!("Cannot canonicalize '{}': {}", candidate.display(), e),
            }
            continue;
        }

        let s = p.to_string_lossy();
        if Regex::new(&s).is_ok() {
            regex_patterns.push(format!("(?:{s})"));
        } else {
            eprintln!("Invalid regex pattern '{s}', skipping");
        }
    }

    let combined_re = if regex_patterns.is_empty() {
        None
    } else {
        match Regex::new(&regex_patterns.join("|")) {
            Ok(re) => Some(re),
            Err(e) => { eprintln!("Failed to combine regex patterns: {e}"); None }
        }
    };

    (literal_roots, combined_re)
}

/// Walk repo, returning (abs_path, rel_norm_string) for every non-ignored file
/// that matches literal_roots or combined_re.
pub fn walk_matching(
    repo_root:    &Path,
    ignore:       &Ignore,
    literal_roots: &[PathBuf],
    combined_re:   Option<&Regex>,
) -> Vec<(Box<Path>, Box<str>)> {
    let mut files = Vec::new();

    for entry in WalkDir::new(repo_root)
        .into_iter()
        .filter_entry(|e| !ignore.is_ignored_abs(e.path()))
    {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() { continue }

        let path = entry.into_path().into_boxed_path();
        let Ok(rel) = path.strip_prefix(repo_root) else { continue };
        let rel_norm = rel.to_string_lossy().replace('\\', "/").into_boxed_str();

        let matched = literal_roots.iter().any(|root| path.starts_with(root))
            || combined_re.is_some_and(|re| re.is_match(&rel_norm));

        if matched {
            files.push((path, rel_norm));
        }
    }

    files.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    files.dedup_by(|a, b| a.0 == b.0);
    files
}

struct FileInfo {
    hash: Hash,
    offset: u32,
    len: u32
}

struct FileMeta {
    path:     Box<Path>,
    rel_norm: Box<Path>,
    meta:     fs::Metadata,
}

struct ProcessedFile<'a> {
    file_meta: &'a FileMeta,
    encoded: Box<[u8]>,
    hash:    Hash,
}

fn flush_batch(
    repo:        &mut Repository,
    index:       &mut Index,
    encoded_buf: &[u8],
    file_infos:  &[FileInfo],
    file_metas:  &[&FileMeta],
) -> Result<()> {
    if file_metas.is_empty() {
        return Ok(());
    }

    let _span = tracy::span!("add::flush");

    let hash_and_data_iter = file_infos.iter().map(|FileInfo { hash, offset, len }| {
        (*hash, &encoded_buf[*offset as usize..*offset as usize + *len as usize])
    });
    repo.storage.write_batch(hash_and_data_iter)?;

    for (FileMeta { rel_norm, meta, .. }, FileInfo { hash, .. }) in file_metas.iter().zip(file_infos.iter()) {
        index.add(rel_norm.to_str().unwrap(), *hash, meta);
    }

    Ok(())
}
