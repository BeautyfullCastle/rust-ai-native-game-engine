//! Linux descriptor-relative primitives shared by offline authoring callers.
//! Callers own path policy and must pass already validated absolute directory paths.
//! Trusted stable ancestry is required; this module does not establish a sandbox.
use rustix::fs::{self as unix, AtFlags, Mode, OFlags};
use std::{
    fs::{self, File},
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Component, Path},
};

pub(crate) const DIR_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

pub(crate) fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub(crate) fn directory(path: &Path) -> Result<File, String> {
    let mut fd = File::open("/").map_err(error)?;
    for part in path.components() {
        match part {
            Component::RootDir => {}
            Component::Normal(name) => {
                fd = File::from(unix::openat(&fd, name, DIR_FLAGS, Mode::empty()).map_err(error)?);
            }
            Component::CurDir => {}
            _ => return Err("unsafe directory component".into()),
        }
    }
    Ok(fd)
}

pub(crate) fn same(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    a.dev() == b.dev() && a.ino() == b.ino()
}

pub(crate) fn same_snapshot(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    same(a, b)
        && a.len() == b.len()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
}

pub(crate) fn recheck_directory(path: &Path, fd: &File) -> Result<(), String> {
    let current = directory(path)?;
    if !same(
        &current.metadata().map_err(error)?,
        &fd.metadata().map_err(error)?,
    ) {
        return Err("directory identity changed".into());
    }
    Ok(())
}

pub(crate) fn read_at(
    parent: &File,
    name: &std::ffi::OsStr,
    max: usize,
) -> Result<Vec<u8>, String> {
    let mut file = File::from(
        unix::openat(
            parent,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(error)?,
    );
    let before = file.metadata().map_err(error)?;
    if !before.is_file() || before.len() > max as u64 {
        return Err("input must be a bounded regular file".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    let after = file.metadata().map_err(error)?;
    let current = unix::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
    if bytes.len() > max
        || bytes.len() as u64 != before.len()
        || !same_snapshot(&before, &after)
        || current.st_dev != after.dev()
        || current.st_ino != after.ino()
    {
        return Err("input changed during bounded snapshot".into());
    }
    Ok(bytes)
}
