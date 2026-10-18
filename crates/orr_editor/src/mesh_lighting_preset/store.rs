//! Local preset import/export. The destination directory must be trusted.
//!
//! Metadata checks detect observed replacements and changes, not every possible
//! concurrent write. Ancestor checks are path based, not an atomic filesystem
//! snapshot or a defense against an attacker controlling the directory tree.
use super::{Preset, MAX_BYTES};
use std::{
    fs::{self, Metadata, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

const MAX_PATH_BYTES: usize = 4096;
const MAX_PATH_COMPONENTS: usize = 128;
const SUFFIX: &str = ".mesh-lighting-preset.json";
#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
#[cfg(windows)]
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

struct CheckedPath {
    path: PathBuf,
    parent: PathBuf,
    ancestors: Vec<(PathBuf, Metadata)>,
}

pub(crate) fn load(path: &Path) -> Result<Preset, String> {
    let checked = checked_path(path)?;
    let bytes = read_file(&checked, || {}, || {})?;
    Preset::decode(&bytes)
}

pub(crate) fn save_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    save_new_observed(path, bytes, || {})
}

fn save_new_observed(
    path: &Path,
    bytes: &[u8],
    before_publish: impl FnOnce(),
) -> Result<(), String> {
    // Even direct callers cannot publish an oversized or malformed preset.
    Preset::decode(bytes)?;
    let checked = checked_path(path)?;
    refuse_existing(&checked.path)?;
    let mut temporary = tempfile::NamedTempFile::new_in(&checked.parent)
        .map_err(|error| format!("create mesh lighting preset staging file: {error}"))?;
    temporary
        .write_all(bytes)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|error| format!("write mesh lighting preset: {error}"))?;
    before_publish();
    recheck_ancestors(&checked)?;
    temporary
        .persist_noclobber(&checked.path)
        .map_err(|error| {
            format!(
                "publish mesh lighting preset without replacement: {}",
                error.error
            )
        })?;
    // Publication is the final fallible step. Never remove a published path
    // after an observation fails: it might now belong to another writer.
    Ok(())
}

fn refuse_existing(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err("mesh lighting preset already exists; choose a new filename".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("inspect mesh lighting preset destination: {error}")),
    }
}

fn checked_path(path: &Path) -> Result<CheckedPath, String> {
    let encoded = path.as_os_str().as_encoded_bytes();
    if !path.is_absolute()
        || encoded.len() > MAX_PATH_BYTES
        || encoded.starts_with(b"\\\\")
        || encoded.starts_with(b"//")
    {
        return Err(
            "mesh lighting preset path must be absolute, local, and within 4096 bytes".into(),
        );
    }
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("mesh lighting preset filename must be Unicode")?;
    if !filename.ends_with(SUFFIX) || filename.len() == SUFFIX.len() {
        return Err("mesh lighting preset filename must end in .mesh-lighting-preset.json".into());
    }
    validate_component(filename)?;
    let parent = path
        .parent()
        .ok_or("mesh lighting preset parent is missing")?;
    let ancestors = inspect_ancestors(parent)?;
    Ok(CheckedPath {
        path: path.to_owned(),
        parent: parent.to_owned(),
        ancestors,
    })
}

fn inspect_ancestors(parent: &Path) -> Result<Vec<(PathBuf, Metadata)>, String> {
    let mut current = PathBuf::new();
    let mut ancestors = Vec::new();
    for component in parent.components() {
        match component {
            Component::Prefix(prefix) if matches!(prefix.kind(), std::path::Prefix::Disk(_)) => {
                current.push(component);
                continue;
            }
            Component::RootDir => current.push(component),
            Component::Normal(name) => {
                validate_component(
                    name.to_str()
                        .ok_or("mesh lighting preset path must be Unicode")?,
                )?;
                current.push(name);
            }
            _ => {
                return Err(
                    "mesh lighting preset path cannot contain traversal or device prefixes".into(),
                )
            }
        }
        if ancestors.len() >= MAX_PATH_COMPONENTS {
            return Err("mesh lighting preset path has too many components".into());
        }
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| format!("inspect mesh lighting preset directory: {error}"))?;
        if !metadata.is_dir() || is_reparse(&metadata) {
            return Err(
                "mesh lighting preset ancestry must use real non-reparse directories".into(),
            );
        }
        verify_directory(&current, &metadata)?;
        ancestors.push((current.clone(), metadata));
    }
    if ancestors.is_empty() {
        return Err("mesh lighting preset parent must be an existing directory".into());
    }
    Ok(ancestors)
}

fn recheck_ancestors(checked: &CheckedPath) -> Result<(), String> {
    let current = inspect_ancestors(&checked.parent)?;
    if current.len() != checked.ancestors.len()
        || current
            .iter()
            .zip(&checked.ancestors)
            .any(|((path, now), (old_path, old))| path != old_path || !same_directory(old, now))
    {
        return Err("mesh lighting preset directory changed during access".into());
    }
    Ok(())
}

fn validate_component(component: &str) -> Result<(), String> {
    if component.is_empty()
        || component.encode_utf16().count() > 255
        || component.ends_with('.')
        || component.ends_with(' ')
        || component.chars().any(|c| {
            c.is_control() || matches!(c, ':' | '/' | '\\' | '<' | '>' | '"' | '|' | '?' | '*')
        })
    {
        return Err("mesh lighting preset path contains a non-portable component".into());
    }
    let base = component
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if matches!(
        base.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "CONIN$"
            | "CONOUT$"
            | "COM¹"
            | "COM²"
            | "COM³"
            | "LPT¹"
            | "LPT²"
            | "LPT³"
    ) || (base.len() == 4
        && (base.starts_with("COM") || base.starts_with("LPT"))
        && base.as_bytes()[3].is_ascii_digit())
    {
        return Err("mesh lighting preset path contains a Windows-reserved device name".into());
    }
    Ok(())
}

fn verify_directory(path: &Path, observed: &Metadata) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let opened = OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .and_then(|directory| directory.metadata())
            .map_err(|error| format!("open mesh lighting preset directory: {error}"))?;
        if !opened.is_dir() || is_reparse(&opened) || !same_directory(observed, &opened) {
            return Err("mesh lighting preset directory changed while opening".into());
        }
    }
    #[cfg(not(windows))]
    let _ = (path, observed);
    Ok(())
}

// The two no-op production observers permit deterministic replacement/race
// tests without relying on scheduler timing or an unbounded filesystem reader.
fn read_file(
    checked: &CheckedPath,
    before_open: impl FnOnce(),
    after_read: impl FnOnce(),
) -> Result<Vec<u8>, String> {
    let before = fs::symlink_metadata(&checked.path)
        .map_err(|error| format!("inspect mesh lighting preset: {error}"))?;
    if !regular_file(&before) || before.len() > MAX_BYTES as u64 {
        return Err(format!(
            "mesh lighting preset must be a regular file of at most {MAX_BYTES} bytes"
        ));
    }
    before_open();
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
        // Linux O_NOFOLLOW | O_NONBLOCK: a swapped FIFO must not hang open.
        options.custom_flags(0x20000 | 0x800);
    }
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    ))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x100 | 0x4); // O_NOFOLLOW | O_NONBLOCK
    }
    let mut file = options
        .open(&checked.path)
        .map_err(|error| format!("open mesh lighting preset: {error}"))?;
    let opened = file
        .metadata()
        .map_err(|error| format!("inspect opened mesh lighting preset: {error}"))?;
    if !regular_file(&opened) || !same_metadata(&before, &opened) {
        return Err("mesh lighting preset changed while opening".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read mesh lighting preset: {error}"))?;
    after_read();
    let handle_after = file
        .metadata()
        .map_err(|error| format!("recheck opened mesh lighting preset: {error}"))?;
    let path_after = fs::symlink_metadata(&checked.path)
        .map_err(|error| format!("recheck mesh lighting preset: {error}"))?;
    if bytes.len() > MAX_BYTES
        || bytes.len() as u64 != opened.len()
        || !regular_file(&handle_after)
        || !regular_file(&path_after)
        || !same_metadata(&opened, &handle_after)
        || !same_metadata(&opened, &path_after)
    {
        return Err("mesh lighting preset changed or exceeded its byte limit while reading".into());
    }
    recheck_ancestors(checked)?;
    Ok(bytes)
}

fn regular_file(metadata: &Metadata) -> bool {
    metadata.is_file() && !is_reparse(metadata)
}

fn is_reparse(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x0400 != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn same_directory(left: &Metadata, right: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        left.dev() == right.dev() && left.ino() == right.ino() && left.mode() == right.mode()
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        left.creation_time() == right.creation_time()
            && left.file_attributes() == right.file_attributes()
    }
    #[cfg(not(any(windows, unix)))]
    {
        left.is_dir() == right.is_dir() && left.created().ok() == right.created().ok()
    }
}

fn same_metadata(left: &Metadata, right: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        same_directory(left, right)
            && left.nlink() == right.nlink()
            && left.len() == right.len()
            && left.mtime() == right.mtime()
            && left.mtime_nsec() == right.mtime_nsec()
            && left.ctime() == right.ctime()
            && left.ctime_nsec() == right.ctime_nsec()
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        same_directory(left, right)
            && left.len() == right.len()
            && left.last_write_time() == right.last_write_time()
    }
    #[cfg(not(any(windows, unix)))]
    {
        left.len() == right.len()
            && left.modified().ok() == right.modified().ok()
            && left.created().ok() == right.created().ok()
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
