//! Shared private offline authoring I/O. Input/output directories must remain trusted
//! and unchanged. Static path checks are not race-proof openat/no-follow access.
//! Linux publication is atomic no-replace, not parent-fsynced crash durability.
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};
pub(super) const MAX_RGBA_BYTES: usize = 16 * 1024 * 1024;

pub(super) fn validate_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'))
}

// Canonical SemVer 2.0.0, including u64 core fields as used by orr_package.
// Keep this example independent of the optional image-reimport feature/dependency.
pub(super) fn validate_version(value: &str) -> bool {
    if value.is_empty() || value.len() > 256 {
        return false;
    }
    let (without_build, build) = value
        .split_once('+')
        .map_or((value, None), |(a, b)| (a, Some(b)));
    let (core, pre) = without_build
        .split_once('-')
        .map_or((without_build, None), |(a, b)| (a, Some(b)));
    let numbers: Vec<_> = core.split('.').collect();
    if numbers.len() != 3
        || !numbers
            .iter()
            .all(|s| numeric(s) && s.parse::<u64>().is_ok())
    {
        return false;
    }
    let identifiers = |s: &str, prerelease: bool| {
        s.split('.').all(|part| {
            !part.is_empty()
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && (!prerelease || !part.bytes().all(|b| b.is_ascii_digit()) || numeric(part))
        })
    };
    pre.is_none_or(|s| identifiers(s, true)) && build.is_none_or(|s| identifiers(s, false))
}

fn numeric(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0'))
}

// Static checks only: no race-proof no-follow/openat guarantee. All input and
// output directories must remain trusted and unchanged throughout the command.
pub(super) fn checked_path(path: &Path, allow_missing_leaf: bool) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("empty path".into());
    }
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    let parts: Vec<_> = absolute.components().collect();
    let mut checked = PathBuf::new();
    for (index, part) in parts.iter().enumerate() {
        if matches!(part, Component::ParentDir) {
            return Err("parent traversal is not allowed".into());
        }
        checked.push(part.as_os_str());
        match fs::symlink_metadata(&checked) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Err("symlink paths are not allowed".into());
                }
                if index + 1 < parts.len() && !meta.is_dir() {
                    return Err("path ancestor is not a directory".into());
                }
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && allow_missing_leaf
                    && index + 1 == parts.len() => {}
            Err(error) => return Err(format!("{}: {error}", checked.display())),
        }
    }
    Ok(checked)
}

pub(super) fn read_regular(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let path = checked_path(path, false)?;
    let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > limit as u64 {
        return Err("input must be a bounded regular file".into());
    }
    let mut file = File::open(&path).map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > limit as u64 {
        return Err("input changed or exceeds byte budget".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("input exceeds byte budget".into());
    }
    Ok(bytes)
}

pub(super) fn relative_input(base: &Path, value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains('\\')
        || value.contains(':')
        || path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("recipe inputs must be relative paths without traversal".into());
    }
    checked_path(&base.join(path), false)
}

pub(super) fn rgba_size(width: u32, height: u32) -> Result<usize, String> {
    if width == 0 || height == 0 || width > 2048 || height > 2048 {
        return Err("image or atlas dimensions must be 1..=2048".into());
    }
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .filter(|n| *n <= MAX_RGBA_BYTES)
        .ok_or_else(|| "RGBA byte budget exceeded".into())
}

pub(super) fn json_bytes(value: &impl Serialize) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub(super) fn publish(out: &Path, files: &[(&str, &[u8])]) -> Result<(), String> {
    let parent = out.parent().ok_or("output parent missing")?;
    let stage = tempfile::Builder::new()
        .prefix(".orr-png-sequence-")
        .tempdir_in(parent)
        .map_err(|e| e.to_string())?;
    for (name, bytes) in files {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(stage.path().join(name))
            .map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
    }
    if checked_path(out, true)? != out {
        return Err("output parent changed".into());
    }
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        stage.path(),
        rustix::fs::CWD,
        out,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(|e| format!("atomic no-replace publication failed: {e}"))?;
    // Stage now has no path. TempDir cleanup cannot target the published destination.
    Ok(())
}
