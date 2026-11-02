//! Bounded no-follow records, never executable provenance or deployment.
use crate::authoring_io::{directory, error, read_at, recheck_directory, same_snapshot};
use rustix::fs::{self as unix, AtFlags, Mode, OFlags};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
};
const MAX_BINARY: u64 = 512 * 1024 * 1024;
pub(super) fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("empty path".into());
    }
    let p = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir().map_err(error)?.join(path)
    };
    if p.as_os_str().as_encoded_bytes().len() > 4096
        || p.components().count() > 128
        || p.components().any(|c| matches!(c, Component::ParentDir))
        || p.as_os_str().as_encoded_bytes().contains(&0)
    {
        return Err("path exceeds bounds or contains traversal/NUL".into());
    }
    Ok(p)
}
pub(super) fn manifest(path: &Path) -> Result<Vec<u8>, String> {
    let p = absolute(path)?;
    let root = p.parent().ok_or("missing parent")?;
    let fd = directory(root)?;
    let bytes = read_at(&fd, p.file_name().ok_or("missing filename")?, 1024 * 1024)?;
    recheck_directory(root, &fd)?;
    Ok(bytes)
}
pub(super) fn binary_hash(path: &Path) -> Result<String, String> {
    let p = absolute(path)?;
    let root = p.parent().ok_or("missing binary parent")?;
    let name = p.file_name().ok_or("missing binary filename")?;
    let parent = directory(root)?;
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
    if !before.is_file() || before.len() > MAX_BINARY {
        return Err("binary provenance requires a regular file <=512 MiB".into());
    }
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    let mut count = 0u64;
    loop {
        let n = file.read(&mut buffer).map_err(error)?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > MAX_BINARY {
            return Err("binary grew beyond512MiB".into());
        }
        hasher.update(&buffer[..n]);
    }
    let after = file.metadata().map_err(error)?;
    let named = unix::statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
    if count != before.len()
        || !same_snapshot(&before, &after)
        || named.st_dev != after.dev()
        || named.st_ino != after.ino()
    {
        return Err("binary changed during provenance hash".into());
    }
    recheck_directory(root, &parent)?;
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
pub(super) fn write_new(path: &Path, bytes: &[u8]) -> Result<Option<String>, String> {
    write_with(path, bytes, |_| Ok(()))
}
fn write_with(
    path: &Path,
    bytes: &[u8],
    mut checkpoint: impl FnMut(&str) -> Result<(), String>,
) -> Result<Option<String>, String> {
    if bytes.len() > 1024 * 1024 {
        return Err("manifest exceeds1MiB".into());
    }
    let p = absolute(path)?;
    let root = p.parent().ok_or("missing output parent")?;
    let name = p.file_name().ok_or("missing output filename")?;
    let parent = directory(root)?;
    recheck_directory(root, &parent)?;
    let mut file = File::from(
        unix::openat(
            &parent,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR | Mode::RGRP | Mode::ROTH,
        )
        .map_err(error)?,
    );
    let admitted = file.metadata().map_err(|e| {
        format!(
            "{e}; claimed partial output may remain; inspect manually, do not retry/remove blindly"
        )
    })?;
    let writing = (|| -> Result<(), String> {
        file.write_all(bytes).map_err(error)?;
        checkpoint("after-write")?;
        let named = unix::statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
        if named.st_dev != admitted.dev() || named.st_ino != admitted.ino() {
            return Err("output identity changed".into());
        }
        if read_at(&parent, name, bytes.len())? != bytes {
            return Err("output bytes changed".into());
        }
        recheck_directory(root, &parent)?;
        Ok(())
    })();
    if let Err(e) = writing {
        return Err(format!(
            "{e}; claimed partial output may remain; inspect manually, do not retry/remove blindly"
        ));
    }
    let sync = (|| -> Result<(), String> {
        checkpoint("before-file-sync")?;
        file.sync_all().map_err(error)?;
        checkpoint("before-parent-sync")?;
        parent.sync_all().map_err(error)
    })();
    Ok(sync.err())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exclusive_and_partial_failures_preserve_output() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("out");
        let e = write_with(&p, b"full", |phase| {
            if phase == "after-write" {
                Err("injected".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert!(e.contains("partial output"));
        assert_eq!(std::fs::read(&p).unwrap(), b"full");
        assert!(write_new(&p, b"replace").is_err());
        assert_eq!(std::fs::read(&p).unwrap(), b"full");
    }
    #[test]
    fn completed_sync_failure_warns_not_absence() {
        for phase in ["before-file-sync", "before-parent-sync"] {
            let t = tempfile::tempdir().unwrap();
            let p = t.path().join("out");
            let warning = write_with(&p, b"verified", |p| {
                if p == phase {
                    Err("injectedsync".into())
                } else {
                    Ok(())
                }
            })
            .unwrap();
            assert!(warning.is_some());
            assert_eq!(std::fs::read(&p).unwrap(), b"verified");
        }
    }
    #[test]
    fn binary_streaming_and_sparse_limit() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("binary");
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(
            binary_hash(&p).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        File::create(&p).unwrap().set_len(MAX_BINARY + 1).unwrap();
        assert!(binary_hash(&p).is_err());
    }
}
