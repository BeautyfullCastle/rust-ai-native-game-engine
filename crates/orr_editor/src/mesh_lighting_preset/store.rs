//! Local preset import/export. The destination directory must be trusted.
//!
//! Metadata checks detect observed replacements and changes, not every possible
//! concurrent write. Ancestor checks are path based, not an atomic filesystem
//! snapshot or a defense against an attacker controlling the directory tree.
use super::{Preset, MAX_BYTES};
use orr_fsid::PinnedDirectory;
use std::{
    fs::{self, Metadata},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

const MAX_PATH_BYTES: usize = 4096;
const MAX_PATH_COMPONENTS: usize = 128;
const SUFFIX: &str = ".mesh-lighting-preset.json";

struct CheckedPath {
    path: PathBuf,
    parent: PathBuf,
    ancestors: Vec<PinnedDirectory>,
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

fn inspect_ancestors(parent: &Path) -> Result<Vec<PinnedDirectory>, String> {
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
        // One open checks the type, refuses a link and pins the identity.
        let directory = PinnedDirectory::open(&current)
            .map_err(|error| format!("open mesh lighting preset directory: {error}"))?;
        ancestors.push(directory);
    }
    if ancestors.is_empty() {
        return Err("mesh lighting preset parent must be an existing directory".into());
    }
    Ok(ancestors)
}

fn recheck_ancestors(checked: &CheckedPath) -> Result<(), String> {
    for directory in &checked.ancestors {
        directory.verify_unchanged().map_err(|error| {
            format!("mesh lighting preset directory changed during access: {error}")
        })?;
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

// The two no-op production observers permit deterministic replacement/race
// tests without relying on scheduler timing or an unbounded filesystem reader.
fn read_file(
    checked: &CheckedPath,
    before_open: impl FnOnce(),
    after_read: impl FnOnce(),
) -> Result<Vec<u8>, String> {
    // The first handle pins the identity of the file that was inspected. A link, a
    // folder or a FIFO at the path is refused by `open_file`.
    let inspected = orr_fsid::open_file(&checked.path)
        .map_err(|error| format!("inspect mesh lighting preset: {error}"))?;
    let before = inspected
        .metadata()
        .map_err(|error| format!("inspect mesh lighting preset: {error}"))?;
    if before.len() > MAX_BYTES as u64 {
        return Err(format!(
            "mesh lighting preset must be a regular file of at most {MAX_BYTES} bytes"
        ));
    }
    before_open();
    let mut file = orr_fsid::open_file(&checked.path)
        .map_err(|error| format!("mesh lighting preset changed while opening: {error}"))?;
    let opened = file
        .metadata()
        .map_err(|error| format!("inspect opened mesh lighting preset: {error}"))?;
    let same_file = orr_fsid::file_id(&file)
        .and_then(|opened_id| orr_fsid::file_id(&inspected).map(|first| opened_id == first))
        .map_err(|error| format!("inspect opened mesh lighting preset: {error}"))?;
    if !same_file || !same_stamp(&before, &opened) {
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
    let still_named = orr_fsid::same_file(&checked.path, &file)
        .map_err(|error| format!("recheck mesh lighting preset: {error}"))?;
    if bytes.len() > MAX_BYTES
        || bytes.len() as u64 != opened.len()
        || !handle_after.is_file()
        || !still_named
        || !same_stamp(&opened, &handle_after)
    {
        return Err("mesh lighting preset changed or exceeded its byte limit while reading".into());
    }
    recheck_ancestors(checked)?;
    Ok(bytes)
}

/// Size, times and attributes of one file. Identity is compared by handle, not here.
fn same_stamp(left: &Metadata, right: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        left.mode() == right.mode()
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
        left.file_attributes() == right.file_attributes()
            && left.len() == right.len()
            && left.last_write_time() == right.last_write_time()
    }
    #[cfg(not(any(windows, unix)))]
    {
        left.len() == right.len() && left.modified().ok() == right.modified().ok()
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
