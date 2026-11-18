//! Bounded PNG snapshots from regular files with trusted, quiescent ancestry.
//!
//! Rejects symlinks at each observed path component. The file itself is opened
//! without following a link or waiting on a FIFO, and its identity comes from the
//! open handle. This is not a lock: in-place edits may evade observations.
//! The caller must exclude concurrent mutation of the source and its ancestry.
use super::{Error, Result, MAX_PNG_BYTES, SIGNATURE};
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

fn checked_path(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty()
        || path.as_os_str().len() > 4096
        || path.components().any(|c| matches!(c, Component::ParentDir))
    {
        return Err(Error(
            "PNG16 input path must be nonempty, at most 4096 bytes, without parent traversal"
                .into(),
        ));
    }
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|e| Error(format!("current directory: {e}")))?
            .join(path)
    };
    let mut resolved = PathBuf::new();
    for part in absolute.components() {
        if matches!(part, Component::CurDir) {
            continue;
        }
        resolved.push(part.as_os_str());
        if matches!(part, Component::Prefix(_)) {
            continue;
        }
        let metadata = fs::symlink_metadata(&resolved)
            .map_err(|e| Error(format!("PNG16 path metadata: {e}")))?;
        if metadata.file_type().is_symlink() {
            return Err(Error(
                "PNG16 symlink ancestry or destination is not allowed".into(),
            ));
        }
    }
    Ok(resolved)
}

/// Retain a bounded PNG snapshot without a generic arbitrary-byte read API.
/// This checks the PNG signature; `import_png16` validates the complete format.
pub fn read_png16(input: &Path) -> Result<Vec<u8>> {
    let path = checked_path(input)?;
    // `open_file` refuses a link, a folder or a FIFO without waiting for a writer.
    let file = orr_fsid::open_file(&path).map_err(|e| Error(format!("PNG16 input open: {e}")))?;
    let opened = file
        .metadata()
        .map_err(|e| Error(format!("PNG16 opened metadata: {e}")))?;
    if opened.len() < SIGNATURE.len() as u64 || opened.len() > MAX_PNG_BYTES as u64 {
        return Err(Error(format!(
            "PNG16 input must be a regular file of 8..={MAX_PNG_BYTES} bytes"
        )));
    }
    let expected = opened.len() as usize;
    if !matches!(orr_fsid::same_file(&checked_path(&path)?, &file), Ok(true)) {
        return Err(Error(
            "PNG16 input changed during open; use a trusted stable source directory".into(),
        ));
    }
    let mut limited = file.take(expected as u64 + 1);
    let mut bytes = Vec::with_capacity(expected + 1);
    limited
        .read_to_end(&mut bytes)
        .map_err(|e| Error(format!("PNG16 input read: {e}")))?;
    let after = limited
        .get_ref()
        .metadata()
        .map_err(|e| Error(format!("PNG16 final metadata: {e}")))?;
    let still_named = matches!(
        orr_fsid::same_file(&checked_path(&path)?, limited.get_ref()),
        Ok(true)
    );
    if bytes.len() != expected
        || after.len() != expected as u64
        || !still_named
        || opened.modified().ok() != after.modified().ok()
        || !bytes.starts_with(SIGNATURE)
    {
        return Err(Error(
            "PNG16 input changed during read or lacks a PNG signature".into(),
        ));
    }
    Ok(bytes)
}
