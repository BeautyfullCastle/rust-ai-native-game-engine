//! Bounded RAW16 input snapshots, requiring trusted stable ancestry.
//! These std path checks are not a hostile-user filesystem sandbox: replacement
//! between the checks and the open is detected by file identity from open handles. Retained
//! bytes do not certify absence of concurrent in-place modification.
use super::{Error, Recipe, Result};
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

fn checked_path(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(Error(
            "empty paths and parent traversal are not allowed".into(),
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
        match fs::symlink_metadata(&resolved) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(Error(
                    "symlink ancestry or destination is not allowed".into(),
                ))
            }
            Ok(_) => {}
            Err(e) => return Err(Error(format!("path metadata: {e}"))),
        }
    }
    Ok(resolved)
}

/// Open a regular file by handle, verify the path still names it, and retain
/// at most expected+1 bytes. Exact length is required; no pipes or stdin route.
pub fn read_raw16(input: &Path, recipe: &Recipe) -> Result<Vec<u8>> {
    let expected = recipe.validate()?;
    let path = checked_path(input)?;
    // `open_file` refuses a link, a folder or a FIFO without waiting for a writer.
    let file = orr_fsid::open_file(&path).map_err(|e| Error(format!("input open: {e}")))?;
    let opened = file
        .metadata()
        .map_err(|e| Error(format!("opened input metadata: {e}")))?;
    if opened.len() != expected as u64 {
        return Err(Error(format!(
            "input must be a regular file of exactly {expected} bytes"
        )));
    }
    if !matches!(orr_fsid::same_file(&path, &file), Ok(true)) {
        return Err(Error(
            "input changed during open; use a trusted stable source directory".into(),
        ));
    }
    let mut limited = file.take(expected as u64 + 1);
    let mut bytes = Vec::with_capacity(expected + 1);
    limited
        .read_to_end(&mut bytes)
        .map_err(|e| Error(format!("input read: {e}")))?;
    let after = limited
        .get_ref()
        .metadata()
        .map_err(|e| Error(format!("input final metadata: {e}")))?;
    if bytes.len() != expected
        || after.len() != expected as u64
        || opened.modified().ok() != after.modified().ok()
        || !matches!(orr_fsid::same_file(&path, limited.get_ref()), Ok(true))
    {
        return Err(Error(
            "input length, modification time or identity changed during read".into(),
        ));
    }
    Ok(bytes)
}
