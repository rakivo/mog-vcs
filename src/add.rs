use std::fs;
use std::borrow::Cow;
use std::path::PathBuf;

use crate::tracy;
use crate::hash::Hash;
use crate::index::Index;
use crate::repository::Repository;
use crate::object::encode_blob_into;

use anyhow::Result;
use walkdir::WalkDir;
use regex::Regex;

const ADD_BATCH_MAX_BYTES: usize = 1024 * 1024;
const ADD_MAX_FILE_BYTES:  usize = 1024 * 1024;

pub fn add(repo: &mut Repository, paths: &[PathBuf]) -> Result<()> {
    let _span = tracy::span!("add");

    let mut refused_over_limit = 0usize; // @Metric
    let mut added_successfully = 0usize; // @Metric
    let mut bytes_added_successfully = 0usize; // @Metric

    let current_dir = std::env::current_dir()?;
    let mut index   = Index::load(&repo.root)?;

    //
    //
    // Classify patterns into literal roots or regexes.
    //
    //

    let default = [PathBuf::from(".")];
    let patterns = if paths.is_empty() { &default } else { paths };

    let mut literal_roots = Vec::new();
    let mut regexes       = Vec::new(); // @Speed @Memory: We most likely shouldn't store them like that.

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
        match Regex::new(&s) {
            Ok(re) => regexes.push(re),
            Err(_) => eprintln!("Invalid regex pattern '{}', skipping", s),
        }
    }

    //
    //
    // Collect candidate files.
    //
    //

    let mut files_to_add = Vec::new();

    for entry in WalkDir::new(&repo.root)
        .into_iter()
        .filter_entry(|e| !repo.ignore.is_ignored_abs(e.path()))
    {
        let Ok(entry) = entry else { continue };

        if !entry.file_type().is_file() { continue }

        let path = entry.into_path();

        let Ok(rel) = path.strip_prefix(&repo.root) else { continue };
        let rel_norm_string = rel.to_string_lossy().replace('\\', "/");

        let matched = literal_roots.iter().any(|root| path.starts_with(root))
            || regexes.iter().any(|re| re.is_match(&rel_norm_string));

        if matched {
            files_to_add.push(path);
        }
    }

    files_to_add.sort_unstable();
    files_to_add.dedup();

    //
    //
    // Filter to dirty files within the size limit.
    //
    //

    let mut files_to_process = Vec::<FileMeta>::new();

    for path in files_to_add {
        //
        // @Cutnpaste from above
        //
        let Ok(rel) = path.strip_prefix(&repo.root) else { continue };
        let rel_norm_string = rel.to_string_lossy().replace('\\', "/");

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
            refused_over_limit += 1;
            continue;
        }

        if let Some(i) = index.find(rel_norm_string.as_ref()) {
            if !index.is_dirty(i, &metadata) {
                continue;
            }
        }

        let rel_norm = PathBuf::from(&rel_norm_string);
        files_to_process.push(FileMeta { path, rel_norm, meta: metadata.into() });
    }

    if refused_over_limit > 0 {
        eprintln!(
            "Refused to add {refused_over_limit} file(s) over 1 MiB (max {ADD_MAX_FILE_BYTES} bytes)"
        );
    }

    //
    //
    // Encode and write in size-bounded batches.
    //
    //

    let mut encoded_buf               = Vec::new();
    let mut file_infos                = Vec::<FileInfo>::new();
    let mut file_metas_batch          = Vec::<FileMeta>::new();
    let mut current_batch_bytes       = 0usize;
    let mut singular_blob_scratch_buf = Vec::new();

    for file in files_to_process {
        let FileMeta { path, meta: metadata, .. } = &file;

        let size = metadata.len() as usize;

        if current_batch_bytes + size > ADD_BATCH_MAX_BYTES {
            flush_batch(repo, &mut index, &encoded_buf, &file_infos, &file_metas_batch)?;

            //
            // Reset the batch
            //

            // @Cleanup: Move this to flush_batch?
            encoded_buf.clear();
            file_infos.clear();
            file_metas_batch.clear();
            current_batch_bytes = 0;
        }

        let data = match fs::read(path) {
            Ok(d)  => d,
            Err(e) => {
                eprintln!("read error for {}: {}", path.display(), e);
                continue;
            }
        };

        singular_blob_scratch_buf.clear();
        encode_blob_into(&data, &mut singular_blob_scratch_buf);

        let hash  = Hash::from(blake3::hash(&singular_blob_scratch_buf));
        let offset = encoded_buf.len() as _;
        let len = singular_blob_scratch_buf.len() as _;

        encoded_buf.extend_from_slice(&singular_blob_scratch_buf);
        file_infos.push(FileInfo { hash, offset, len });
        file_metas_batch.push(file);
        current_batch_bytes += size;

        added_successfully += 1;
        bytes_added_successfully += data.len();
    }

    flush_batch(repo, &mut index, &encoded_buf, &file_infos, &file_metas_batch)?;

    repo.storage.sync()?;
    index.save(&repo.root)?;

    println!("Added {added_successfully} file(s), {bytes_added_successfully} in byte(s)");

    Ok(())
}

struct FileInfo {
    hash: Hash,
    offset: u32,
    len: u32
}

struct FileMeta {
     // @Memory: Make these Box<Path>?
    path: PathBuf,
    rel_norm: PathBuf,
    meta: Box<fs::Metadata>
}

fn flush_batch(
    repo:        &mut Repository,
    index:       &mut Index,
    encoded_buf: &[u8],
    file_infos:  &[FileInfo],
    file_metas:  &[FileMeta],
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
        index.add(rel_norm.as_path(), *hash, meta);
    }

    Ok(())
}
