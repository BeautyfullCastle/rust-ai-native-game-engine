use crate::room_wall_stamp::Template;
use sha2::{Digest, Sha256};
#[cfg(target_os = "linux")]
use std::os::unix::fs::MetadataExt;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;
use std::{
    fs::{self, Metadata, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

const MAX_PATH_BYTES: usize = 4096;
const MAX_NAME_BYTES: usize = 120;
const MAX_BYTES: usize = 64 * 1024;
const SUFFIX: &str = ".room-stamp.json";
#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
#[cfg(windows)]
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

// Linux x86_64 only: the module is gated to this ABI in lib.rs.
#[cfg(target_os = "linux")]
const LINUX_NOFOLLOW_NONBLOCK: i32 = 0x20000 | 0x800;

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn validate_expected(expected: Option<&str>) -> Result<(), String> {
    if expected.is_some_and(|value| {
        value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }) {
        return Err("expected Room wall stamp digest must be lowercase SHA-256".into());
    }
    Ok(())
}

struct CheckedPath {
    path: PathBuf,
    parent: PathBuf,
}

pub(crate) fn load(path: &Path) -> Result<Option<(Template, String)>, String> {
    let checked = checked_path(path)?;
    let Some(bytes) = read_optional(&checked)? else {
        return Ok(None);
    };
    let template = Template::decode(&bytes)?;
    Ok(Some((template, digest(&bytes))))
}

pub(crate) fn save(
    path: &Path,
    template: &Template,
    expected: Option<&str>,
) -> Result<String, String> {
    save_with_before_publish(path, template, expected, || {})
}

// Per-call observer allows deterministic publication races without global test state.
fn save_with_before_publish(
    path: &Path,
    template: &Template,
    expected: Option<&str>,
    before_publish: impl FnOnce(),
) -> Result<String, String> {
    let checked = checked_path(path)?;
    validate_expected(expected)?;
    let current = read_optional(&checked)?;
    match (current.as_deref().map(digest), expected) {
        (None, None) => {}
        (Some(current), Some(expected)) if current == expected => {}
        (None, Some(_)) => return Err("Room wall stamp was removed by another editor".into()),
        (Some(_), None) => {
            return Err("Room wall stamp already exists; load it before replacing".into())
        }
        (Some(_), Some(_)) => {
            return Err("Room wall stamp changed; reload it before replacing".into())
        }
    }
    let bytes = template.encode()?;
    if bytes.len() > MAX_BYTES {
        return Err(format!("Room wall stamp exceeds {MAX_BYTES} bytes"));
    }
    let mut temporary = tempfile::NamedTempFile::new_in(&checked.parent)
        .map_err(|error| format!("create Room wall stamp staging file: {error}"))?;
    temporary
        .write_all(&bytes)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|error| format!("write Room wall stamp: {error}"))?;
    recheck_parent(&checked.parent)?;
    let final_current = read_optional(&checked)?;
    if final_current.as_deref().map(digest).as_deref() != expected {
        return Err("Room wall stamp changed before save; reload before replacing".into());
    }
    before_publish();
    if expected.is_some() {
        temporary
            .persist(&checked.path)
            .map_err(|error| format!("replace Room wall stamp: {}", error.error))?;
    } else {
        temporary
            .persist_noclobber(&checked.path)
            .map_err(|error| {
                format!(
                    "publish Room wall stamp without replacement: {}",
                    error.error
                )
            })?;
    }
    Ok(digest(&bytes))
}

fn checked_path(path: &Path) -> Result<CheckedPath, String> {
    if !path.is_absolute() || path.as_os_str().as_encoded_bytes().len() > MAX_PATH_BYTES {
        return Err("Room wall stamp path must be absolute and within 4096 bytes".into());
    }
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Room wall stamp filename must be Unicode")?;
    validate_filename(filename)?;
    let parent = path
        .parent()
        .ok_or("Room wall stamp directory is missing")?;
    validate_parent(parent)?;
    Ok(CheckedPath {
        path: path.to_owned(),
        parent: parent.to_owned(),
    })
}

fn validate_filename(filename: &str) -> Result<(), String> {
    if filename.len() > MAX_NAME_BYTES
        || !filename.ends_with(SUFFIX)
        || filename.len() == SUFFIX.len()
        || filename.ends_with('.')
        || filename.ends_with(' ')
        || filename.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    ':' | '/' | '\\' | '<' | '>' | '"' | '|' | '?' | '*'
                )
        })
    {
        return Err("Room wall stamp filename must end in .room-stamp.json and be portable".into());
    }
    let base = filename
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if matches!(
        base.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "COM¹" | "COM²" | "COM³" | "LPT¹" | "LPT²" | "LPT³"
    ) || (base.len() == 4
        && (base.starts_with("COM") || base.starts_with("LPT"))
        && base.as_bytes()[3].is_ascii_digit())
    {
        return Err("Room wall stamp filename is reserved by Windows".into());
    }
    Ok(())
}

fn validate_parent(parent: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in parent.components() {
        match component {
            Component::Prefix(prefix) if matches!(prefix.kind(), std::path::Prefix::Disk(_)) => {
                current.push(component);
            }
            Component::RootDir => current.push(component),
            Component::Normal(_) => {
                current.push(component);
                let metadata = fs::symlink_metadata(&current)
                    .map_err(|error| format!("inspect Room wall stamp directory: {error}"))?;
                if !metadata.is_dir() || is_reparse(&metadata) {
                    return Err(
                        "Room wall stamp path must use real directories, not reparse points".into(),
                    );
                }
                verify_directory(&current, &metadata)?;
            }
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(
                    "Room wall stamp path cannot contain traversal or device prefixes".into(),
                );
            }
        }
    }
    if !parent.is_dir() {
        return Err("Room wall stamp parent must be an existing directory".into());
    }
    Ok(())
}

fn verify_directory(path: &Path, observed: &Metadata) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(LINUX_NOFOLLOW_NONBLOCK);
    }
    let directory = options
        .open(path)
        .map_err(|error| format!("open Room wall stamp directory: {error}"))?;
    let opened = directory.metadata().map_err(|error| error.to_string())?;
    if !opened.is_dir() || is_reparse(&opened) || !same_directory_stamp(observed, &opened) {
        return Err("Room wall stamp directory changed while opening".into());
    }
    Ok(())
}

fn same_directory_stamp(left: &Metadata, right: &Metadata) -> bool {
    #[cfg(windows)]
    {
        same_metadata_stamp(left, right)
    }
    #[cfg(target_os = "linux")]
    {
        // Staging a sibling changes directory length/timestamps, not its identity.
        left.dev() == right.dev() && left.ino() == right.ino() && left.mode() == right.mode()
    }
}

fn read_optional(checked: &CheckedPath) -> Result<Option<Vec<u8>>, String> {
    read_optional_with_hooks(checked, || {}, || {})
}

// Per-call observers make read races deterministic without shared test state.
fn read_optional_with_hooks(
    checked: &CheckedPath,
    before_open: impl FnOnce(),
    after_read: impl FnOnce(),
) -> Result<Option<Vec<u8>>, String> {
    let before = match fs::symlink_metadata(&checked.path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("inspect Room wall stamp: {error}")),
    };
    if !before.is_file() || is_reparse(&before) || before.len() > MAX_BYTES as u64 {
        return Err(format!(
            "Room wall stamp must be a regular file of at most {MAX_BYTES} bytes"
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
        // Reject swapped symlinks and avoid blocking on a swapped FIFO.
        options.custom_flags(LINUX_NOFOLLOW_NONBLOCK);
    }
    let mut file = options
        .open(&checked.path)
        .map_err(|error| format!("open Room wall stamp: {error}"))?;
    let opened = file
        .metadata()
        .map_err(|error| format!("inspect opened Room wall stamp: {error}"))?;
    if !opened.is_file() || is_reparse(&opened) || !same_metadata_stamp(&before, &opened) {
        return Err("Room wall stamp changed while opening".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read Room wall stamp: {error}"))?;
    after_read();
    #[cfg(target_os = "linux")]
    {
        let handle_after = file
            .metadata()
            .map_err(|error| format!("recheck opened Room wall stamp: {error}"))?;
        if !handle_after.is_file() || !same_metadata_stamp(&opened, &handle_after) {
            return Err("Room wall stamp changed while reading".into());
        }
    }
    let after = fs::symlink_metadata(&checked.path)
        .map_err(|error| format!("recheck Room wall stamp: {error}"))?;
    if !after.is_file()
        || is_reparse(&after)
        || !same_metadata_stamp(&opened, &after)
        || after.len() != bytes.len() as u64
        || bytes.len() > MAX_BYTES
    {
        return Err("Room wall stamp changed or exceeded its byte limit while reading".into());
    }
    recheck_parent(&checked.parent)?;
    Ok(Some(bytes))
}

fn recheck_parent(parent: &Path) -> Result<(), String> {
    validate_parent(parent)
}

fn is_reparse(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        metadata.file_type().is_symlink()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(target_os = "linux")]
    {
        metadata.file_type().is_symlink()
    }
}

fn same_metadata_stamp(left: &Metadata, right: &Metadata) -> bool {
    #[cfg(windows)]
    {
        left.creation_time() == right.creation_time()
            && left.file_attributes() == right.file_attributes()
            && left.len() == right.len()
            && left.last_write_time() == right.last_write_time()
    }
    #[cfg(target_os = "linux")]
    {
        same_directory_stamp(left, right)
            && left.nlink() == right.nlink()
            && left.len() == right.len()
            && left.mtime() == right.mtime()
            && left.mtime_nsec() == right.mtime_nsec()
            && left.ctime() == right.ctime()
            && left.ctime_nsec() == right.ctime_nsec()
    }
}

#[cfg(test)]
#[path = "room_wall_stamp_store/tests.rs"]
mod tests;

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
#[path = "room_wall_stamp_store/linux_tests.rs"]
mod linux_tests;
