//! Atomic no-replace file publication; no deletion of uncertain or foreign objects.
use super::{absolute, sampler::Verified};
use crate::authoring_io::{directory, error, read_at, recheck_directory};
use rustix::fs::{self as unix, AtFlags, Mode, OFlags};
use std::{
    fs::File,
    io::Write,
    os::unix::fs::MetadataExt,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};
static SERIAL: AtomicU64 = AtomicU64::new(0);
pub(super) fn preflight(path: &Path) -> Result<(), String> {
    let path = absolute(path)?;
    let fd = directory(path.parent().ok_or("output parent missing")?)?;
    match unix::statat(
        &fd,
        path.file_name().ok_or("output filename missing")?,
        AtFlags::SYMLINK_NOFOLLOW,
    ) {
        Err(rustix::io::Errno::NOENT) => Ok(()),
        Ok(_) => Err("output already exists".into()),
        Err(e) => Err(error(e)),
    }
}
pub(super) fn publish(path: &Path, verified: &Verified) -> Result<Option<String>, String> {
    write(path, verified.prepared().bytes(), |_, _| Ok(()))
}
pub(super) fn publish_scene(
    path: &Path,
    verified: &super::scene::Verified,
) -> Result<Option<String>, String> {
    write(path, verified.bytes(), |_, _| Ok(()))
}
pub(super) fn publish_offline(
    path: &Path,
    verified: &super::offline::OfflineHtml,
) -> Result<Option<String>, String> {
    write_bounded(
        path,
        verified.bytes(),
        super::offline::MAX_OUTPUT,
        |_, _| Ok(()),
    )
}
fn write(
    path: &Path,
    bytes: &[u8],
    checkpoint: impl FnMut(&str, &Path) -> Result<(), String>,
) -> Result<Option<String>, String> {
    write_bounded(path, bytes, 65536, checkpoint)
}
fn write_bounded(
    path: &Path,
    bytes: &[u8],
    max: usize,
    mut checkpoint: impl FnMut(&str, &Path) -> Result<(), String>,
) -> Result<Option<String>, String> {
    if bytes.len() > max {
        return Err("manifest exceeds64KiB".into());
    }
    let path = absolute(path)?;
    let root = path.parent().ok_or("output parent missing")?;
    let target = path.file_name().ok_or("output filename missing")?;
    let parent = directory(root)?;
    preflight(&path)?;
    let mut allocated = None;
    for _ in 0..128 {
        let name = format!(
            ".orr-web-animation-{}-{}.tmp",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        );
        match unix::openat(
            &parent,
            name.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        ) {
            Ok(fd) => {
                allocated = Some((name, File::from(fd)));
                break;
            }
            Err(rustix::io::Errno::EXIST) => continue,
            Err(e) => return Err(error(e)),
        }
    }
    let (name, mut file) = allocated.ok_or("unable to claim private output sibling")?;
    let stage = root.join(&name);
    let mut published = false;
    let result = (|| -> Result<(), String> {
        let info = file.metadata().map_err(error)?;
        file.write_all(bytes).map_err(error)?;
        file.sync_all().map_err(error)?;
        checkpoint("before-link", &stage)?;
        recheck_directory(root, &parent)?;
        let named =
            unix::statat(&parent, name.as_str(), AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
        if named.st_dev != info.dev() || named.st_ino != info.ino() {
            return Err("staged identity changed".into());
        }
        if read_at(&parent, name.as_ref(), max)? != bytes {
            return Err("staged bytes changed".into());
        }
        unix::linkat(&parent, name.as_str(), &parent, target, AtFlags::empty()).map_err(error)?;
        published = true;
        checkpoint("after-link", &path)?;
        let output = unix::statat(&parent, target, AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
        if output.st_dev != info.dev() || output.st_ino != info.ino() {
            return Err("published identity changed".into());
        }
        let sibling =
            unix::statat(&parent, name.as_str(), AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
        if sibling.st_dev != info.dev() || sibling.st_ino != info.ino() {
            return Err("sibling identity changed; retained".into());
        }
        unix::unlinkat(&parent, name.as_str(), AtFlags::empty()).map_err(error)?;
        recheck_directory(root, &parent)?;
        parent.sync_all().map_err(error)?;
        Ok(())
    })();
    match result {
        Ok(()) => Ok(None),
        Err(e) if published => Ok(Some(format!(
            "published; durability or cleanup uncertain: {e}; inspect {}",
            stage.display()
        ))),
        Err(e) => Err(format!(
            "{e}; unpublished sibling retained at {}; no automatic deletion",
            stage.display()
        )),
    }
}
#[cfg(test)]
#[path = "publish_tests.rs"]
mod tests;
