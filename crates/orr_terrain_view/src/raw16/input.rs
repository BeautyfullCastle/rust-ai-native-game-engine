//! Bounded RAW16 input snapshots, requiring trusted stable ancestry.
//! These std path checks are not a hostile-user filesystem sandbox: replacement
//! between metadata and open can still block on a substituted FIFO. Retained
//! bytes do not certify absence of concurrent in-place modification.
use super::{Error, Recipe, Result};
use std::{
    fs::{self, File, Metadata},
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

fn same_file(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        // std exposes no portable stable file ID. These are consistency checks,
        // not proof of identity; trusted non-mutating input ancestry is required.
        a.len() == b.len() && a.modified().ok() == b.modified().ok()
    }
}

/// Verify a regular file before open, verify the opened handle again, and retain
/// at most expected+1 bytes. Exact length is required; no pipes or stdin route.
pub fn read_raw16(input: &Path, recipe: &Recipe) -> Result<Vec<u8>> {
    let expected = recipe.validate()?;
    let path = checked_path(input)?;
    let before = fs::symlink_metadata(&path).map_err(|e| Error(format!("input metadata: {e}")))?;
    if !before.file_type().is_file() || before.len() != expected as u64 {
        return Err(Error(format!(
            "input must be a regular file of exactly {expected} bytes"
        )));
    }
    let file = File::open(&path).map_err(|e| Error(format!("input open: {e}")))?;
    let opened = file
        .metadata()
        .map_err(|e| Error(format!("opened input metadata: {e}")))?;
    let current = fs::symlink_metadata(&path).map_err(|e| Error(format!("input recheck: {e}")))?;
    if !opened.is_file()
        || !current.file_type().is_file()
        || opened.len() != expected as u64
        || !same_file(&before, &opened)
        || !same_file(&opened, &current)
    {
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
    {
        return Err(Error(
            "input length or modification time changed during read".into(),
        ));
    }
    Ok(bytes)
}
