use sha2::{Digest, Sha256};
use std::{
    fs::{self, Metadata, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

const MAX_PATH_BYTES: usize = 4096;
const MAX_PATH_COMPONENTS: usize = 128;
const MAX_FILENAME_UTF16_UNITS: usize = 255;
const SUFFIX: &str = ".mesh-prop-prefab.json";
#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
#[cfg(windows)]
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

use crate::mesh_prop_prefab::MAX_BYTES;

struct CheckedPath {
    path: PathBuf,
    parent: PathBuf,
}

pub(crate) fn load(path: &Path) -> Result<(Vec<u8>, String), String> {
    let checked = checked_path(path)?;
    let bytes = read_file(&checked)?;
    let hash = digest(&bytes);
    Ok((bytes, hash))
}

pub(crate) fn save_new(path: &Path, bytes: &[u8]) -> Result<String, String> {
    if bytes.len() > MAX_BYTES {
        return Err(format!("mesh prop prefab exceeds {MAX_BYTES} bytes"));
    }
    let checked = checked_path(path)?;
    let mut temporary = tempfile::NamedTempFile::new_in(&checked.parent)
        .map_err(|error| format!("create mesh prop prefab staging file: {error}"))?;
    temporary
        .write_all(bytes)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|error| format!("write mesh prop prefab: {error}"))?;
    recheck_parent(&checked.parent)?;
    temporary
        .persist_noclobber(&checked.path)
        .map_err(|error| {
            format!(
                "publish mesh prop prefab without replacement: {}",
                error.error
            )
        })?;
    Ok(digest(bytes))
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn checked_path(path: &Path) -> Result<CheckedPath, String> {
    let encoded = path.as_os_str().as_encoded_bytes();
    if !path.is_absolute()
        || encoded.len() > MAX_PATH_BYTES
        || encoded.starts_with(b"\\\\")
        || encoded.starts_with(b"//")
    {
        return Err("mesh prop prefab path must be absolute, local, and within 4096 bytes".into());
    }
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("mesh prop prefab filename must be Unicode")?;
    validate_filename(filename)?;
    let parent = path
        .parent()
        .ok_or("mesh prop prefab directory is missing")?;
    validate_parent(parent)?;
    Ok(CheckedPath {
        path: path.to_owned(),
        parent: parent.to_owned(),
    })
}

fn validate_filename(filename: &str) -> Result<(), String> {
    if !filename.ends_with(SUFFIX)
        || filename.len() == SUFFIX.len()
        || filename.encode_utf16().count() > MAX_FILENAME_UTF16_UNITS
        || filename.chars().any(char::is_control)
        || filename.ends_with('.')
        || filename.ends_with(' ')
    {
        return Err(
            "mesh prop prefab filename must end in .mesh-prop-prefab.json and be portable".into(),
        );
    }
    validate_windows_component(filename)
}

fn validate_parent(parent: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    let mut normal_count = 0;
    for component in parent.components() {
        match component {
            Component::Prefix(prefix) if matches!(prefix.kind(), std::path::Prefix::Disk(_)) => {
                current.push(component);
            }
            Component::RootDir => current.push(component),
            Component::Normal(name) => {
                let name = name
                    .to_str()
                    .ok_or("mesh prop prefab path components must be Unicode")?;
                validate_windows_component(name)?;
                normal_count += 1;
                if normal_count > MAX_PATH_COMPONENTS {
                    return Err("mesh prop prefab path has too many components".into());
                }
                current.push(name);
                let metadata = fs::symlink_metadata(&current)
                    .map_err(|error| format!("inspect mesh prop prefab directory: {error}"))?;
                if !metadata.is_dir() || is_reparse(&metadata) {
                    return Err(
                        "mesh prop prefab path must use real directories, not symlinks or reparse points".into(),
                    );
                }
                verify_open_directory(&current, &metadata)?;
            }
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(
                    "mesh prop prefab path cannot contain traversal or device prefixes".into(),
                );
            }
        }
    }
    if !parent.is_dir() {
        return Err("mesh prop prefab parent must be an existing directory".into());
    }
    Ok(())
}

fn validate_windows_component(component: &str) -> Result<(), String> {
    if component.is_empty()
        || component.ends_with('.')
        || component.ends_with(' ')
        || component.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    ':' | '/' | '\\' | '<' | '>' | '"' | '|' | '?' | '*'
                )
        })
    {
        return Err("mesh prop prefab path contains a non-portable component".into());
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
        return Err("mesh prop prefab path contains a Windows-reserved device name".into());
    }
    Ok(())
}

fn verify_open_directory(path: &Path, observed: &Metadata) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .map_err(|error| format!("open mesh prop prefab directory: {error}"))?;
        let opened = directory
            .metadata()
            .map_err(|error| format!("inspect opened mesh prop prefab directory: {error}"))?;
        if !opened.is_dir() || is_reparse(&opened) || !same_metadata_stamp(observed, &opened) {
            return Err("mesh prop prefab directory changed while opening".into());
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (path, observed);
    }
    Ok(())
}

fn read_file(checked: &CheckedPath) -> Result<Vec<u8>, String> {
    let before = fs::symlink_metadata(&checked.path)
        .map_err(|error| format!("inspect mesh prop prefab: {error}"))?;
    if !before.is_file() || is_reparse(&before) || before.len() > MAX_BYTES as u64 {
        return Err(format!(
            "mesh prop prefab must be a regular file of at most {MAX_BYTES} bytes"
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = options
        .open(&checked.path)
        .map_err(|error| format!("open mesh prop prefab: {error}"))?;
    let opened = file
        .metadata()
        .map_err(|error| format!("inspect opened mesh prop prefab: {error}"))?;
    if !opened.is_file() || is_reparse(&opened) || !same_metadata_stamp(&before, &opened) {
        return Err("mesh prop prefab changed while opening".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read mesh prop prefab: {error}"))?;
    let after = fs::symlink_metadata(&checked.path)
        .map_err(|error| format!("recheck mesh prop prefab: {error}"))?;
    if !after.is_file()
        || is_reparse(&after)
        || !same_metadata_stamp(&opened, &after)
        || after.len() != bytes.len() as u64
        || bytes.len() > MAX_BYTES
    {
        return Err("mesh prop prefab changed or exceeded its byte limit while reading".into());
    }
    recheck_parent(&checked.parent)?;
    Ok(bytes)
}

fn recheck_parent(parent: &Path) -> Result<(), String> {
    validate_parent(parent)
}

fn is_reparse(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn same_metadata_stamp(left: &Metadata, right: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        left.creation_time() == right.creation_time()
            && left.file_attributes() == right.file_attributes()
            && left.len() == right.len()
            && left.last_write_time() == right.last_write_time()
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        left.dev() == right.dev()
            && left.ino() == right.ino()
            && left.mode() == right.mode()
            && left.nlink() == right.nlink()
            && left.len() == right.len()
            && left.mtime() == right.mtime()
            && left.mtime_nsec() == right.mtime_nsec()
            && left.ctime() == right.ctime()
            && left.ctime_nsec() == right.ctime_nsec()
    }
    #[cfg(not(any(windows, unix)))]
    {
        left.len() == right.len()
            && left.modified().ok() == right.modified().ok()
            && left.created().ok() == right.created().ok()
            && left.is_file() == right.is_file()
            && left.is_dir() == right.is_dir()
    }
}

#[cfg(test)]
#[path = "mesh_prop_prefab_store/tests.rs"]
mod tests;
