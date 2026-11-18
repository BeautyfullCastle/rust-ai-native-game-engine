//! The operating system pieces of starter creation. Linux keeps its exact checks
//! (`O_NOFOLLOW`, `RENAME_NOREPLACE`, file modes). Windows uses reparse-point checks
//! and a native move that never replaces. Callers see one set of functions.
use std::{
    fs::{self, File},
    io,
    path::{Component, Path, PathBuf},
};

// The Room creator on Windows compiles the same file into the binary and into
// `windows_project`. One more copy here keeps the checks in one place.
#[cfg(windows)]
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../windows_room/platform.rs"]
mod win;

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use std::{
        fs::{OpenOptions, Permissions},
        os::unix::fs::{OpenOptionsExt, PermissionsExt},
    };

    pub fn destination(path: &Path) -> Result<PathBuf, String> {
        let text = path.to_str().ok_or("output path must be UTF-8")?;
        if !path.is_absolute()
            || text.len() > 4096
            || text.chars().any(char::is_control)
            || text.split('/').any(|part| matches!(part, "." | ".."))
            || path.components().count() > 64
        {
            return Err(
                "output must be an absolute bounded path without dot or parent traversal".into(),
            );
        }
        let leaf = path.file_name().ok_or("output needs a new directory leaf")?;
        if leaf.len() > 200 {
            return Err("output leaf exceeds 200 bytes".into());
        }
        let parent = path.parent().ok_or("output requires an existing parent")?;
        let mut current = PathBuf::new();
        for component in parent.components() {
            if !matches!(component, Component::RootDir | Component::Normal(_)) {
                return Err("invalid output parent component".into());
            }
            current.push(component);
            let meta = fs::symlink_metadata(&current)
                .map_err(|e| format!("output parent {}: {e}", current.display()))?;
            if meta.file_type().is_symlink() || !meta.is_dir() {
                return Err(
                    "output parent must contain only existing nonsymlink directories".into(),
                );
            }
        }
        let output = fs::canonicalize(parent).map_err(|e| e.to_string())?.join(leaf);
        match fs::symlink_metadata(&output) {
            Ok(_) => Err("output destination already exists".into()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(output),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn publish_no_replace(stage: &Path, output: &Path) -> Result<(), String> {
        crate::project_publish::publish_no_replace(stage, output)
    }

    /// A new file that only this process can write until `finish_file` runs.
    pub fn create_file(path: &Path) -> io::Result<File> {
        OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)
    }

    pub fn finish_file(file: &File) -> io::Result<()> {
        file.set_permissions(Permissions::from_mode(0o644))
    }

    /// Opens a regular file without following a final symlink.
    pub fn open_nofollow(path: &Path, nonblock: bool) -> io::Result<File> {
        let mut flags = rustix::fs::OFlags::NOFOLLOW;
        if nonblock {
            flags |= rustix::fs::OFlags::NONBLOCK;
        }
        OpenOptions::new().read(true).custom_flags(flags.bits() as i32).open(path)
    }

    /// Whether `metadata` describes a directory that is not a link.
    pub fn plain_directory(metadata: &fs::Metadata) -> bool {
        metadata.is_dir() && !metadata.file_type().is_symlink()
    }

    pub fn accepts_component(component: Component<'_>) -> bool {
        matches!(component, Component::RootDir | Component::Normal(_))
    }

    pub fn has_traversal(text: &str) -> bool {
        text.split('/').any(|part| matches!(part, "." | ".."))
    }

    pub fn stage_name(relative: &Path) -> Option<String> {
        relative.to_str().map(str::to_owned)
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};

    use super::win;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    pub fn destination(path: &Path) -> Result<PathBuf, String> {
        win::destination(path)
    }

    /// `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING`: it fails when the output exists.
    pub fn publish_no_replace(stage: &Path, output: &Path) -> Result<(), String> {
        win::publish(stage, output)
    }

    pub fn create_file(path: &Path) -> io::Result<File> {
        OpenOptions::new().write(true).create_new(true).open(path)
    }

    pub fn finish_file(_file: &File) -> io::Result<()> {
        Ok(())
    }

    /// Opens the link itself when `path` is a reparse point; callers then see it is no regular file.
    pub fn open_nofollow(path: &Path, _nonblock: bool) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
    }

    pub fn plain_directory(metadata: &fs::Metadata) -> bool {
        win::is_directory(metadata)
    }

    pub fn accepts_component(component: Component<'_>) -> bool {
        matches!(
            component,
            Component::Prefix(_) | Component::RootDir | Component::Normal(_)
        )
    }

    pub fn has_traversal(text: &str) -> bool {
        text.split(['\\', '/']).any(|part| matches!(part, "." | ".."))
    }

    /// The stage file name with `/` separators, as the expected file list spells it.
    pub fn stage_name(relative: &Path) -> Option<String> {
        relative.to_str().map(|text| text.replace('\\', "/"))
    }
}

pub(super) use imp::{
    create_file, destination, finish_file, open_nofollow, publish_no_replace, stage_name,
};

/// Checks a caller supplied package directory: absolute, bounded, without traversal,
/// and with only plain (non link) directories on the way. `what` names it in errors.
pub(super) fn package_directory(root: &Path, what: &str) -> Result<(), String> {
    let text = root.to_str().ok_or(format!("{what} path must be UTF-8"))?;
    if !root.is_absolute()
        || text.len() > 4096
        || text.chars().any(char::is_control)
        || imp::has_traversal(text)
        || root.components().count() > 64
    {
        return Err(format!("{what} requires an absolute bounded path without traversal"));
    }
    let mut current = PathBuf::new();
    for component in root.components() {
        if !imp::accepts_component(component) {
            return Err(format!("invalid {what} path"));
        }
        current.push(component);
        if matches!(component, Component::Prefix(_)) {
            // A drive prefix is not a directory path until the root component follows.
            continue;
        }
        let metadata = fs::symlink_metadata(&current).map_err(|e| e.to_string())?;
        if !imp::plain_directory(&metadata) {
            return Err(format!("{what} ancestors must be nonsymlink directories"));
        }
    }
    Ok(())
}
