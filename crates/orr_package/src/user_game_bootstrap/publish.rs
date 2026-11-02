//! Same-parent no-replace publication. Failed stages are retained, never recursively removed.
use crate::authoring_io::{directory, error, read_at, recheck_directory, DIR_FLAGS};
use rustix::fs::{self as unix, AtFlags, Mode, OFlags, RenameFlags};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::Write,
    os::unix::fs::MetadataExt,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};
static SERIAL: AtomicU64 = AtomicU64::new(0);
fn absent(parent: &File, name: &std::ffi::OsStr) -> Result<(), String> {
    match unix::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Err(rustix::io::Errno::NOENT) => Ok(()),
        Ok(_) => Err("destination already exists; no files changed".into()),
        Err(e) => Err(error(e)),
    }
}
pub(super) fn preflight(destination: &Path) -> Result<(), String> {
    let parent = directory(destination.parent().ok_or("destination parent missing")?)?;
    absent(
        &parent,
        destination.file_name().ok_or("destination name missing")?,
    )
}
fn relative(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && !path.contains(['\\', '\0'])
        && path.split('/').count() <= 128
        && path
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != "..")
}
fn subdir(root: &File, path: &Path) -> Result<File, String> {
    let mut fd = root.try_clone().map_err(error)?;
    for part in path.components() {
        let std::path::Component::Normal(name) = part else {
            return Err("unsafe stage directory".into());
        };
        fd = File::from(unix::openat(&fd, name, DIR_FLAGS, Mode::empty()).map_err(error)?);
    }
    Ok(fd)
}
fn inventory(path: &Path, prefix: &str, out: &mut BTreeSet<String>) -> Result<(), String> {
    if prefix.split('/').count() > 128 {
        return Err("staged depth exceeds bound".into());
    }
    for entry in fs::read_dir(path).map_err(error)? {
        let entry = entry.map_err(error)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "nonUnicode staged path")?;
        let relative = format!("{prefix}{name}");
        if out.len() >= 10000 {
            return Err("staged inventory exceeds bound".into());
        }
        let ty = entry.file_type().map_err(error)?;
        if ty.is_dir() {
            out.insert(format!("{relative}/"));
            inventory(&entry.path(), &format!("{relative}/"), out)?;
        } else if ty.is_file() {
            out.insert(relative);
        } else {
            return Err("nonregular staged input".into());
        }
    }
    Ok(())
}
fn verify(
    stage_path: &Path,
    stage: &File,
    files: &BTreeMap<String, Vec<u8>>,
    expected: &BTreeSet<String>,
    owned: &BTreeMap<String, (u64, u64)>,
) -> Result<(), String> {
    recheck_directory(stage_path, stage)?;
    let mut actual = BTreeSet::new();
    inventory(stage_path, "", &mut actual)?;
    if &actual != expected {
        return Err("staged inventory changed".into());
    }
    for (path, (dev, ino)) in owned {
        let p = Path::new(path.trim_end_matches('/'));
        let parent = subdir(stage, p.parent().unwrap_or(Path::new("")))?;
        let name = p.file_name().ok_or("missing staged name")?;
        let stat = unix::statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
        if stat.st_dev != *dev || stat.st_ino != *ino {
            return Err("staged object identity changed".into());
        }
        if let Some(bytes) = files.get(path) {
            if read_at(&parent, name, 32 * 1024 * 1024)? != *bytes {
                return Err("staged bytes changed".into());
            }
        }
    }
    recheck_directory(stage_path, stage)
}
pub(super) fn publish(
    destination: &Path,
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<Option<String>, String> {
    publish_with(destination, files, |_, _| Ok(()))
}
fn publish_with(
    destination: &Path,
    files: &BTreeMap<String, Vec<u8>>,
    mut checkpoint: impl FnMut(&str, &Path) -> Result<(), String>,
) -> Result<Option<String>, String> {
    if files.is_empty()
        || files.len() > 128
        || files.values().map(Vec::len).sum::<usize>() > 64 * 1024 * 1024
        || files
            .iter()
            .any(|(p, v)| !relative(p) || v.len() > 32 * 1024 * 1024)
    {
        return Err("generated files exceed bootstrap bounds".into());
    }
    let root = destination.parent().ok_or("destination parent missing")?;
    let target = destination.file_name().ok_or("destination name missing")?;
    let parent = directory(root)?;
    absent(&parent, target)?;
    let mut expected: BTreeSet<String> = files.keys().cloned().collect();
    let mut dirs = BTreeSet::new();
    for path in files.keys() {
        let p = Path::new(path);
        for ancestor in p.ancestors().skip(1) {
            if !ancestor.as_os_str().is_empty() {
                dirs.insert(ancestor.to_str().ok_or("bad directory")?.to_owned());
            }
        }
    }
    for d in &dirs {
        if files.contains_key(d) {
            return Err("file/directory collision".into());
        }
        expected.insert(format!("{d}/"));
    }
    let mut name = None;
    for _ in 0..128 {
        let candidate = format!(
            ".orr-bootstrap-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        );
        match unix::mkdirat(
            &parent,
            candidate.as_str(),
            Mode::RUSR | Mode::WUSR | Mode::XUSR,
        ) {
            Ok(()) => {
                name = Some(candidate);
                break;
            }
            Err(rustix::io::Errno::EXIST) => continue,
            Err(e) => return Err(error(e)),
        }
    }
    let name = name.ok_or("cannot allocate private stage")?;
    let stage_path = root.join(&name);
    let stage = File::from(
        unix::openat(&parent, name.as_str(), DIR_FLAGS, Mode::empty())
            .map_err(|e| format!("{e}; stage retained at {}", stage_path.display()))?,
    );
    let mut published = false;
    let result = (|| -> Result<(), String> {
        checkpoint("stage-created", &stage_path)?;
        recheck_directory(&stage_path, &stage)?;
        let mut owned = BTreeMap::new();
        for d in &dirs {
            let p = Path::new(d);
            let fd = subdir(&stage, p.parent().unwrap_or(Path::new("")))?;
            let leaf = p.file_name().ok_or("bad directory")?;
            unix::mkdirat(
                &fd,
                leaf,
                Mode::RUSR
                    | Mode::WUSR
                    | Mode::XUSR
                    | Mode::RGRP
                    | Mode::XGRP
                    | Mode::ROTH
                    | Mode::XOTH,
            )
            .map_err(error)?;
            let child = subdir(&stage, p)?;
            let info = child.metadata().map_err(error)?;
            owned.insert(format!("{d}/"), (info.dev(), info.ino()));
        }
        for (path, bytes) in files {
            let p = Path::new(path);
            let fd = subdir(&stage, p.parent().unwrap_or(Path::new("")))?;
            let leaf = p.file_name().ok_or("bad filename")?;
            let mut output = File::from(
                unix::openat(
                    &fd,
                    leaf,
                    OFlags::WRONLY
                        | OFlags::CREATE
                        | OFlags::EXCL
                        | OFlags::NOFOLLOW
                        | OFlags::CLOEXEC,
                    Mode::RUSR | Mode::WUSR | Mode::RGRP | Mode::ROTH,
                )
                .map_err(error)?,
            );
            let info = output.metadata().map_err(error)?;
            owned.insert(path.clone(), (info.dev(), info.ino()));
            output.write_all(bytes).map_err(error)?;
            checkpoint("file-written", &stage_path)?;
            output.sync_all().map_err(error)?;
        }
        for d in dirs.iter().rev() {
            subdir(&stage, Path::new(d))?.sync_all().map_err(error)?;
        }
        stage.sync_all().map_err(error)?;
        checkpoint("before-rename", &stage_path)?;
        recheck_directory(root, &parent)?;
        verify(&stage_path, &stage, files, &expected, &owned)?;
        unix::renameat_with(
            &parent,
            name.as_str(),
            &parent,
            target,
            RenameFlags::NOREPLACE,
        )
        .map_err(error)?;
        published = true;
        checkpoint("after-rename", destination)?;
        recheck_directory(root, &parent)?;
        recheck_directory(destination, &stage)?;
        parent.sync_all().map_err(error)?;
        Ok(())
    })();
    match result {
        Ok(()) => Ok(None),
        Err(e) if published => Ok(Some(e)),
        Err(e) => Err(format!(
            "{e}; unpublished stage retained at {}; inspect manually, no automatic deletion",
            stage_path.display()
        )),
    }
}
#[cfg(test)]
#[path = "publish_tests.rs"]
mod tests;
