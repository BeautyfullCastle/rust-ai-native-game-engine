//! Local, bounded, non-replacing filesystem operations for hazard bundles.
//!
//! Leaf no-follow flags, identity checks and repeated ancestry checks detect
//! ordinary replacement/change. These are path-based operations, not directory-
//! handle-relative traversal: they do not promise immunity to an adversary
//! concurrently replacing an ancestor between the checks and the operation.
//! Native rename is required solely for atomic no-replace directory publication.
use super::MAX_SCENE_BYTES;
use std::{
    collections::BTreeSet,
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};
const MAX_PATH_BYTES: usize = 4096;
const MAX_PATH_COMPONENTS: usize = 64;
const MAX_SCENE_LEAF_BYTES: usize = 180;
#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
pub(super) fn inventory_names(
    root: &Path,
    expected_count: usize,
) -> Result<BTreeSet<String>, String> {
    let mut names = BTreeSet::new();
    let mut folded = BTreeSet::new();
    for entry in fs::read_dir(root).map_err(|error| format!("read bundle inventory: {error}"))? {
        let entry = entry.map_err(|error| format!("read bundle entry: {error}"))?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
        if !regular_file(&metadata) {
            return Err("hazard bundle entries must be regular non-reparse files".into());
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "bundle filename must be Unicode")?;
        checked_leaf(&name)?;
        if !folded.insert(name.to_lowercase()) || !names.insert(name) {
            return Err("hazard bundle contains colliding filenames".into());
        }
        if names.len() > expected_count {
            return Err("hazard bundle contains too many files".into());
        }
    }
    Ok(names)
}

pub(super) fn checked_scene_path(path: &Path) -> Result<PathBuf, String> {
    let canonical = checked_existing_file(path, MAX_SCENE_BYTES, "mesh scene")?;
    let filename = canonical
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("mesh scene filename must be Unicode")?;
    if !filename.ends_with(".scene.yaml")
        || filename.len() == ".scene.yaml".len()
        || filename.len() > MAX_SCENE_LEAF_BYTES
    {
        return Err("mesh scene filename must be a bounded *.scene.yaml name".into());
    }
    Ok(canonical)
}

pub(super) fn checked_existing_file(
    path: &Path,
    limit: u64,
    context: &str,
) -> Result<PathBuf, String> {
    let text = path.to_str().ok_or("path must be Unicode")?;
    let components: Vec<_> = path.components().collect();
    if !path.is_absolute()
        || text.is_empty()
        || text.len() > MAX_PATH_BYTES
        || components.len() > MAX_PATH_COMPONENTS
        || text
            .chars()
            .any(|character| character.is_control() || character == '\0')
        || text
            .split(['\\', '/'])
            .any(|part| part == "." || part == "..")
    {
        return Err(format!(
            "{context} path must be bounded, absolute and normalized"
        ));
    }
    validate_local_prefix(&components)?;
    let filename = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("path must have a Unicode filename")?;
    checked_leaf(filename)?;
    let parent = path.parent().ok_or("path parent missing")?;
    check_parent_chain(parent)?;
    let canonical_parent = fs::canonicalize(parent)
        .map_err(|error| format!("canonicalize {context} parent: {error}"))?;
    let canonical = canonical_parent.join(filename);
    let metadata =
        fs::symlink_metadata(&canonical).map_err(|error| format!("inspect {context}: {error}"))?;
    if !regular_file(&metadata) || metadata.len() > limit {
        return Err(format!(
            "{context} must be a regular non-reparse file within its byte limit"
        ));
    }
    Ok(canonical)
}

pub(super) fn checked_output_parent(path: &Path) -> Result<PathBuf, String> {
    let text = path.to_str().ok_or("bundle destination must be Unicode")?;
    let components: Vec<_> = path.components().collect();
    if !path.is_absolute()
        || text.is_empty()
        || text.len() > MAX_PATH_BYTES
        || components.len() > MAX_PATH_COMPONENTS
        || text
            .chars()
            .any(|character| character.is_control() || character == '\0')
        || text
            .split(['\\', '/'])
            .any(|part| part == "." || part == "..")
    {
        return Err("bundle destination must be bounded, absolute and normalized".into());
    }
    validate_local_prefix(&components)?;
    let leaf = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("bundle destination must have a Unicode directory name")?;
    checked_leaf(leaf)?;
    let parent = path.parent().ok_or("bundle destination parent missing")?;
    check_parent_chain(parent)?;
    fs::canonicalize(parent)
        .map_err(|error| format!("canonicalize bundle destination parent: {error}"))
}

pub(super) fn checked_directory(path: &Path) -> Result<PathBuf, String> {
    let text = path.to_str().ok_or("bundle root must be Unicode")?;
    let components: Vec<_> = path.components().collect();
    if !path.is_absolute()
        || text.len() > MAX_PATH_BYTES
        || components.len() > MAX_PATH_COMPONENTS
        || text
            .chars()
            .any(|character| character.is_control() || character == '\0')
        || text
            .split(['\\', '/'])
            .any(|part| part == "." || part == "..")
    {
        return Err("bundle root must be a bounded absolute normalized path".into());
    }
    validate_local_prefix(&components)?;
    check_parent_chain(path)?;
    let canonical =
        fs::canonicalize(path).map_err(|error| format!("canonicalize bundle root: {error}"))?;
    let metadata = fs::symlink_metadata(&canonical).map_err(|error| error.to_string())?;
    if !regular_directory(&metadata) {
        return Err("bundle root must be a regular non-reparse directory".into());
    }
    Ok(canonical)
}

pub(super) fn validate_local_prefix(components: &[Component<'_>]) -> Result<(), String> {
    #[cfg(windows)]
    if !matches!(components.first(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)))
    {
        return Err("path must use an absolute local disk prefix".into());
    }
    #[cfg(not(windows))]
    let _ = components;
    Ok(())
}

pub(super) fn check_parent_chain(parent: &Path) -> Result<(), String> {
    let components: Vec<_> = parent.components().collect();
    let mut current = PathBuf::new();
    for component in components {
        match component {
            Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            Component::RootDir => current.push(component.as_os_str()),
            Component::Normal(part) => {
                let value = part.to_str().ok_or("path components must be Unicode")?;
                checked_leaf(value)?;
                current.push(part);
                let metadata = fs::symlink_metadata(&current)
                    .map_err(|error| format!("inspect path ancestry: {error}"))?;
                if !regular_directory(&metadata) {
                    return Err("path ancestry must contain only non-reparse directories".into());
                }
            }
            Component::CurDir | Component::ParentDir => {
                return Err("path ancestry is not normalized".into());
            }
        }
    }
    Ok(())
}

pub(super) fn checked_leaf(value: &str) -> Result<&str, String> {
    if value.is_empty()
        || value.len() > 255
        || value.ends_with(['.', ' '])
        || value.contains([':', '/', '\\', '\0', '<', '>', '|', '?', '*'])
        || value.chars().any(char::is_control)
        || reserved_device_name(value)
    {
        return Err("path contains an invalid Windows filename component".into());
    }
    Ok(value)
}

pub(super) fn reserved_device_name(value: &str) -> bool {
    let stem = value
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(['.', ' '])
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        })
}

pub(super) fn read_bounded(path: &Path, limit: u64, context: &str) -> Result<Vec<u8>, String> {
    let canonical = checked_existing_file(path, limit, context)?;
    let before = fs::symlink_metadata(&canonical).map_err(|error| error.to_string())?;
    read_observed(&canonical, before, limit, context)
}

// Shared checked-open path also permits deterministic replacement-race tests.
pub(super) fn read_observed(
    canonical: &Path,
    before: Metadata,
    limit: u64,
    context: &str,
) -> Result<Vec<u8>, String> {
    let mut file =
        open_without_following(canonical).map_err(|error| format!("open {context}: {error}"))?;
    let opened = file.metadata().map_err(|error| error.to_string())?;
    if !regular_file(&opened) || opened.len() > limit || !same_observation(&before, &opened) {
        return Err(format!("{context} changed before reading"));
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    Read::by_ref(&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read {context}: {error}"))?;
    let after = file.metadata().map_err(|error| error.to_string())?;
    let current = fs::symlink_metadata(canonical).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > limit
        || bytes.len() as u64 != opened.len()
        || !regular_file(&after)
        || !regular_file(&current)
        || !same_observation(&opened, &after)
        || !same_observation(&before, &current)
    {
        return Err(format!("{context} changed while being read"));
    }
    let parent = canonical.parent().ok_or("file parent missing")?;
    check_parent_chain(parent)?;
    if fs::canonicalize(parent).map_err(|error| error.to_string())? != parent {
        return Err(format!("{context} parent changed while being read"));
    }
    Ok(bytes)
}

pub(super) fn open_without_following(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x20000 | 0x800); // Linux O_NOFOLLOW | O_NONBLOCK; reject opened non-regular leaves before reading
    }
    options.open(path)
}

pub(super) fn regular_file(metadata: &Metadata) -> bool {
    if !metadata.is_file() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
    }
    #[cfg(not(windows))]
    {
        !metadata.file_type().is_symlink()
    }
}

pub(super) fn regular_directory(metadata: &Metadata) -> bool {
    if !metadata.is_dir() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
    }
    #[cfg(not(windows))]
    {
        !metadata.file_type().is_symlink()
    }
}

pub(super) fn same_observation(first: &Metadata, second: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if first.dev() != second.dev()
            || first.ino() != second.ino()
            || first.ctime() != second.ctime()
            || first.ctime_nsec() != second.ctime_nsec()
        {
            return false;
        }
    }
    first.len() == second.len()
        && match (first.modified(), second.modified()) {
            (Ok(first_time), Ok(second_time)) => first_time == second_time,
            _ => false,
        }
}

pub(super) fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("create bundle file: {error}"))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("write bundle file: {error}"))
}

pub(super) fn refuse_existing(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err("hazard bundle destination already exists".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("inspect hazard bundle destination: {error}")),
    }
}

pub(super) fn publish_new_directory(stage: &Path, destination: &Path) -> Result<(), String> {
    checked_directory(stage)?;
    checked_output_parent(destination)?;
    refuse_existing(destination)?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
        }
        let stage_metadata = fs::symlink_metadata(stage).map_err(|error| error.to_string())?;
        if !regular_directory(&stage_metadata) {
            return Err("bundle staging directory changed before publish".into());
        }
        refuse_existing(destination)?;
        let stage_wide: Vec<u16> = stage.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination_wide: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let result = unsafe { MoveFileExW(stage_wide.as_ptr(), destination_wide.as_ptr(), 0) };
        if result == 0 {
            return Err(format!(
                "native no-replace bundle publication failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        unsafe extern "C" {
            fn renameat2(
                old_dirfd: i32,
                old_path: *const i8,
                new_dirfd: i32,
                new_path: *const i8,
                flags: u32,
            ) -> i32;
        }
        let old_path =
            CString::new(stage.as_os_str().as_bytes()).map_err(|_| "staging path contains NUL")?;
        let new_path = CString::new(destination.as_os_str().as_bytes())
            .map_err(|_| "destination path contains NUL")?;
        let result = unsafe { renameat2(-100, old_path.as_ptr(), -100, new_path.as_ptr(), 1) };
        if result != 0 {
            return Err(format!(
                "native no-replace bundle publication failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (stage, destination);
        Err("native no-replace bundle publication is unavailable on this platform".into())
    }
}

/// Transfers ownership away from TempDir before any post-publication work.
/// On publication failure the still-owned private stage is cleaned normally.
pub(super) fn publish_stage(
    stage: tempfile::TempDir,
    destination: &Path,
) -> Result<PathBuf, String> {
    publish_new_directory(stage.path(), destination)?;
    Ok(stage.keep())
}
