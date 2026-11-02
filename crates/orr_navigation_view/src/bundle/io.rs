//! Linux no-follow snapshots, not a sandbox. Trusted quiescent ancestry required.
use rustix::fs::{self as unix, AtFlags, Dir, FileType, Mode, OFlags};
use std::{
    collections::BTreeSet,
    fs::{File, Metadata},
    io::Read,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::{Component, Path, PathBuf},
};
const DIR: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub(crate) fn checked_path(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty()
        || path.as_os_str().as_bytes().len() > 4096
        || path.as_os_str().as_bytes().contains(&0)
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)))
    {
        return Err("unsafe bundle path".into());
    }
    let path = if path.is_absolute() {
        path.into()
    } else {
        std::env::current_dir().map_err(error)?.join(path)
    };
    let mut result = PathBuf::new();
    for part in path.components() {
        if !matches!(part, Component::CurDir) {
            result.push(part.as_os_str());
        }
    }
    if result.as_os_str().as_bytes().len() > 4096 || result.components().count() > 64 {
        return Err("bundle path bound exceeded".into());
    }
    Ok(result)
}
pub(crate) fn checked_relative(value: &str) -> Result<PathBuf, String> {
    if value.is_empty()
        || value.len() > 4096
        || value.contains(['\\', ':', '\0'])
        || value.split('/').count() > 64
        || value
            .split('/')
            .any(|p| matches!(p, "" | "." | ".." | ".orr"))
    {
        return Err("unsafe confined bundle relative path".into());
    }
    Ok(PathBuf::from(value))
}
pub(crate) fn directory(path: &Path) -> Result<File, String> {
    let mut file = File::open("/").map_err(error)?;
    for part in path.components() {
        match part {
            Component::RootDir => {}
            Component::Normal(name) => {
                file = File::from(unix::openat(&file, name, DIR, Mode::empty()).map_err(error)?)
            }
            _ => return Err("unsafe directory component".into()),
        }
    }
    Ok(file)
}
fn same(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.len() == b.len()
        && a.mode() == b.mode()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
}
pub(crate) fn recheck_directory(path: &Path, fd: &File) -> Result<(), String> {
    let current = directory(path)?.metadata().map_err(error)?;
    let original = fd.metadata().map_err(error)?;
    if current.dev() != original.dev() || current.ino() != original.ino() {
        return Err("bundle directory identity changed".into());
    }
    Ok(())
}
pub(crate) struct Snapshot {
    pub(crate) bytes: Vec<u8>,
    pub(crate) mode: u32,
    path: PathBuf,
    metadata: Metadata,
    max: usize,
}
impl Snapshot {
    pub(crate) fn read(path: &Path, max: usize) -> Result<Self, String> {
        let path = checked_path(path)?;
        let parent_path = path.parent().ok_or("missing bundle file parent")?;
        let parent = directory(parent_path)?;
        let name = path.file_name().ok_or("missing bundle filename")?;
        let named = unix::statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
        if FileType::from_raw_mode(named.st_mode) != FileType::RegularFile {
            return Err("bundle input must be regular, not symlink".into());
        }
        let mut file = File::from(
            unix::openat(
                &parent,
                name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(error)?,
        );
        let before = file.metadata().map_err(error)?;
        if !before.is_file()
            || before.len() > max as u64
            || before.dev() != named.st_dev
            || before.ino() != named.st_ino
        {
            return Err("bundle file changed or exceeds bound".into());
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(max as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(error)?;
        let after = file.metadata().map_err(error)?;
        let current = unix::statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
        if bytes.len() > max
            || bytes.len() as u64 != before.len()
            || !same(&before, &after)
            || current.st_dev != after.dev()
            || current.st_ino != after.ino()
        {
            return Err("bundle file changed during snapshot".into());
        }
        recheck_directory(parent_path, &parent)?;
        Ok(Self {
            bytes,
            mode: after.mode() & 0o7777,
            path,
            metadata: after,
            max,
        })
    }
    pub(crate) fn recheck(&self) -> Result<(), String> {
        let current = Self::read(&self.path, self.max)?;
        if current.bytes != self.bytes || !same(&self.metadata, &current.metadata) {
            return Err("bundle snapshot changed".into());
        }
        Ok(())
    }
}
pub(super) fn inventory(root: &Path, files: &BTreeSet<PathBuf>) -> Result<(), String> {
    let root = checked_path(root)?;
    let fd = directory(&root)?;
    let mut dirs = BTreeSet::new();
    for file in files {
        let mut parent = file.parent();
        while let Some(path) = parent {
            if path.as_os_str().is_empty() {
                break;
            }
            dirs.insert(path.to_owned());
            parent = path.parent();
        }
    }
    let mut seen = BTreeSet::new();
    let mut seen_dirs = BTreeSet::new();
    fn scan(
        fd: &File,
        prefix: &Path,
        files: &BTreeSet<PathBuf>,
        dirs: &BTreeSet<PathBuf>,
        seen: &mut BTreeSet<PathBuf>,
        seen_dirs: &mut BTreeSet<PathBuf>,
    ) -> Result<(), String> {
        let before = fd.metadata().map_err(error)?;
        for entry in Dir::read_from(fd).map_err(error)? {
            let entry = entry.map_err(error)?;
            let name = std::ffi::OsStr::from_bytes(entry.file_name().to_bytes());
            if name == "." || name == ".." {
                continue;
            }
            let path = prefix.join(name);
            let stat = unix::statat(fd, name, AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
            match FileType::from_raw_mode(stat.st_mode) {
                FileType::Directory => {
                    if !dirs.contains(&path) || !seen_dirs.insert(path.clone()) {
                        return Err("unexpected or duplicate bundle directory".into());
                    }
                    let child =
                        File::from(unix::openat(fd, name, DIR, Mode::empty()).map_err(error)?);
                    let info = child.metadata().map_err(error)?;
                    if info.dev() != stat.st_dev || info.ino() != stat.st_ino {
                        return Err("bundle directory changed".into());
                    }
                    scan(&child, &path, files, dirs, seen, seen_dirs)?;
                    let after = unix::statat(fd, name, AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
                    if after.st_dev != info.dev() || after.st_ino != info.ino() {
                        return Err("bundle directory replaced".into());
                    }
                }
                FileType::RegularFile => {
                    if !files.contains(&path) || !seen.insert(path) {
                        return Err("unexpected or duplicate bundle file".into());
                    }
                }
                _ => return Err("nonregular bundle inventory member".into()),
            }
        }
        if !same(&before, &fd.metadata().map_err(error)?) {
            return Err("bundle inventory changed during scan".into());
        }
        Ok(())
    }
    scan(&fd, Path::new(""), files, &dirs, &mut seen, &mut seen_dirs)?;
    if &seen != files || seen_dirs != dirs {
        return Err("bundle inventory incomplete".into());
    }
    recheck_directory(&root, &fd)
}
