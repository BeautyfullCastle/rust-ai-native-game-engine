//! Linux-only immutable native run generations in an explicitly selected private directory.
//! All data operations remain relative to the opened directory; no arbitrary load paths.
use rustix::fs::{self, AtFlags, FileType, Mode, OFlags, RenameFlags};
use std::{
    fs::File,
    io::{Read, Write},
    os::fd::OwnedFd,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

pub const MAX_SAVE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_DIRECTORY_ENTRIES: usize = 256;
static NEXT: AtomicU64 = AtomicU64::new(0);

/// The selected directory must already exist, be owned by this user and private (0700).
/// A renamed directory stays pinned by this handle. Existing files are never replaced.
#[derive(Debug)]
pub struct RunStore {
    directory: OwnedFd,
}

fn err(e: impl std::fmt::Display) -> String {
    format!("run storage: {e}")
}
fn absolute(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute()
        || path.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::CurDir | Component::Prefix(_)
            )
        })
    {
        return Err(err("use an absolute directory path without . or .."));
    }
    Ok(path.to_path_buf())
}
fn valid_name(name: &str) -> bool {
    let Some(inner) = name
        .strip_prefix("run-")
        .and_then(|n| n.strip_suffix(".json"))
    else {
        return false;
    };
    let fields: Vec<_> = inner.split('-').collect();
    fields.len() == 3
        && fields.iter().zip([32, 8, 16]).all(|(f, len)| {
            f.len() == len
                && f.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}
fn check_directory(fd: &OwnedFd) -> Result<(), String> {
    let stat = fs::fstat(fd).map_err(err)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory
        || stat.st_uid != rustix::process::geteuid().as_raw()
        || stat.st_mode & 0o077 != 0
    {
        return Err(err("selected directory must be user-owned and private (0700); permissions were not changed"));
    }
    Ok(())
}
fn checked_file(fd: OwnedFd) -> Result<File, String> {
    let stat = fs::fstat(&fd).map_err(err)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
        || stat.st_uid != rustix::process::geteuid().as_raw()
        || stat.st_mode & 0o077 != 0
        || stat.st_nlink != 1
        || stat.st_size < 0
        || stat.st_size as u64 > MAX_SAVE_BYTES as u64
    {
        return Err(err(
            "save must be a private, user-owned, single-link regular file of at most 4 MiB",
        ));
    }
    Ok(fd.into())
}
impl RunStore {
    pub fn open(path: &Path, forbidden_roots: &[PathBuf]) -> Result<Self, String> {
        let path = absolute(path)?;
        for root in forbidden_roots {
            let root = std::fs::canonicalize(root).map_err(err)?;
            if path.starts_with(root) {
                return Err(err(
                    "save directory is inside a source or executable bundle root",
                ));
            }
        }
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let mut directory = fs::open("/", flags, Mode::empty()).map_err(err)?;
        for component in path.components() {
            if let Component::Normal(name) = component {
                directory = fs::openat(&directory, name, flags, Mode::empty()).map_err(err)?;
            }
        }
        check_directory(&directory)?;
        let store = Self { directory };
        store.list()?;
        Ok(store)
    }
    /// Bounded directory scan, newest filenames first. Foreign names are never loaded or deleted.
    pub fn list(&self) -> Result<Vec<String>, String> {
        check_directory(&self.directory)?;
        let dir = fs::Dir::read_from(&self.directory).map_err(err)?;
        let mut names = Vec::new();
        let mut count = 0;
        for entry in dir {
            let entry = entry.map_err(err)?;
            let bytes = entry.file_name().to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            count += 1;
            if count > MAX_DIRECTORY_ENTRIES {
                return Err(err(
                    "directory exceeds 256 entries; choose a smaller save directory",
                ));
            }
            let Ok(name) = std::str::from_utf8(bytes) else {
                continue;
            };
            if valid_name(name) {
                // Validate the opened object, not a possibly stale directory-entry type.
                self.open_save(name)?;
                names.push(name.to_owned());
            }
        }
        names.sort_unstable_by(|a, b| b.cmp(a));
        Ok(names)
    }
    fn open_save(&self, name: &str) -> Result<File, String> {
        if !valid_name(name) {
            return Err(err("invalid save generation name"));
        }
        let fd = fs::openat(
            &self.directory,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(err)?;
        checked_file(fd)
    }
    pub fn read(&self, name: &str) -> Result<Vec<u8>, String> {
        check_directory(&self.directory)?;
        let file = self.open_save(name)?;
        let mut bytes = Vec::new();
        file.take((MAX_SAVE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(err)?;
        if bytes.len() > MAX_SAVE_BYTES {
            return Err(err("save grew beyond 4 MiB"));
        }
        Ok(bytes)
    }
    pub fn save(&self, bytes: &[u8]) -> Result<String, String> {
        if bytes.len() > MAX_SAVE_BYTES {
            return Err(err("save exceeds 4 MiB"));
        }
        self.list()?;
        // Require room for the staging entry as well as the final immutable generation.
        let mut dir = fs::Dir::read_from(&self.directory).map_err(err)?;
        let mut count = 0;
        for entry in &mut dir {
            let entry = entry.map_err(err)?;
            if !matches!(entry.file_name().to_bytes(), b"." | b"..") {
                count += 1;
            }
            if count >= MAX_DIRECTORY_ENTRIES {
                return Err(err("save directory is full (256 entries)"));
            }
        }
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(err)?
            .as_nanos();
        for _ in 0..32 {
            let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
            let name = format!(
                "run-{stamp:032x}-{:08x}-{nonce:016x}.json",
                std::process::id()
            );
            let staging = format!(
                ".pending-{stamp:032x}-{:08x}-{nonce:016x}",
                std::process::id()
            );
            let fd = match fs::openat(
                &self.directory,
                staging.as_str(),
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            ) {
                Ok(fd) => fd,
                Err(rustix::io::Errno::EXIST) => continue,
                Err(e) => return Err(err(e)),
            };
            let mut file: File = fd.into();
            let result = (|| {
                file.write_all(bytes).map_err(err)?;
                file.sync_all().map_err(err)?;
                let written = fs::fstat(&file).map_err(err)?;
                let named =
                    fs::statat(&self.directory, staging.as_str(), AtFlags::SYMLINK_NOFOLLOW)
                        .map_err(err)?;
                if written.st_dev != named.st_dev
                    || written.st_ino != named.st_ino
                    || FileType::from_raw_mode(named.st_mode) != FileType::RegularFile
                    || named.st_nlink != 1
                {
                    return Err(err("staging object changed before publication"));
                }
                // This private-directory policy does not defend against a malicious
                // process running as the same user racing after this identity check.
                fs::renameat_with(
                    &self.directory,
                    staging.as_str(),
                    &self.directory,
                    name.as_str(),
                    RenameFlags::NOREPLACE,
                )
                .map_err(err)?;
                fs::fsync(&self.directory).map_err(|e| {
                    err(format!(
                        "generation {name} was published but directory sync failed: {e}"
                    ))
                })?;
                Ok(name)
            })();
            // Leave a failed stage for explicit user inspection. Even in a private
            // directory, deleting by name could delete a same-user replacement.
            // A retained stage counts toward the bounded directory limit.
            return result
                .map_err(|e| format!("{e}; any unpublished stage {staging} was retained"));
        }
        Err(err(
            "could not reserve a unique generation; previous saves preserved",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn private_dir() -> tempfile::TempDir {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    use std::os::unix::fs::{symlink, PermissionsExt};
    fn directory() -> tempfile::TempDir {
        private_dir()
    }
    #[test]
    fn repeated_save_is_immutable_and_bounded() {
        let dir = directory();
        let store = RunStore::open(dir.path(), &[]).unwrap();
        let a = store.save(b"first").unwrap();
        let b = store.save(b"second").unwrap();
        assert_ne!(a, b);
        assert_eq!(store.read(&a).unwrap(), b"first");
        assert_eq!(store.read(&b).unwrap(), b"second");
        assert_eq!(store.list().unwrap().len(), 2);
        assert!(store.save(&vec![0; MAX_SAVE_BYTES + 1]).is_err());
        assert_eq!(store.read(&a).unwrap(), b"first");
        assert!(store.read("../outside").is_err());
    }
    #[test]
    fn rejects_symlink_components_unsafe_directory_and_forbidden_roots() {
        let outer = directory();
        let private = outer.path().join("private");
        std::fs::create_dir(&private).unwrap();
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o700)).unwrap();
        symlink(&private, outer.path().join("link")).unwrap();
        let nested = private.join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(RunStore::open(&outer.path().join("link/nested"), &[]).is_err());
        assert!(RunStore::open(&outer.path().join("link"), &[]).is_err());
        assert!(RunStore::open(&private, &[outer.path().to_path_buf()]).is_err());
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(RunStore::open(&private, &[]).is_err());
    }
    #[test]
    fn hostile_generation_never_followed_and_old_save_preserved() {
        let dir = directory();
        let store = RunStore::open(dir.path(), &[]).unwrap();
        let a = store.save(b"safe").unwrap();
        let hostile = "run-00000000000000000000000000000000-00000000-0000000000000000.json";
        symlink("/etc/passwd", dir.path().join(hostile)).unwrap();
        assert!(store.read(hostile).is_err());
        assert!(store.list().is_err());
        assert!(store.save(b"new").is_err());
        assert_eq!(store.read(&a).unwrap(), b"safe");
        std::fs::remove_file(dir.path().join(hostile)).unwrap();
        fs::mknodat(
            &store.directory,
            hostile,
            FileType::Fifo,
            Mode::RUSR | Mode::WUSR,
            0,
        )
        .unwrap();
        assert!(store.read(hostile).is_err());
    }
    #[test]
    fn foreign_entries_are_preserved_and_scan_is_bounded() {
        let dir = directory();
        let store = RunStore::open(dir.path(), &[]).unwrap();
        for n in 0..=MAX_DIRECTORY_ENTRIES {
            std::fs::write(dir.path().join(format!("foreign-{n}")), b"untouched").unwrap();
        }
        assert!(store.list().is_err());
        assert!(store.save(b"new").is_err());
        assert_eq!(
            std::fs::read(dir.path().join("foreign-0")).unwrap(),
            b"untouched"
        );
    }
    #[test]
    fn capacity_boundary_allows_last_generation_then_refuses() {
        let dir = directory();
        let store = RunStore::open(dir.path(), &[]).unwrap();
        for n in 0..MAX_DIRECTORY_ENTRIES - 1 {
            std::fs::write(dir.path().join(format!("foreign-{n}")), b"keep").unwrap();
        }
        let last = store.save(b"last").unwrap();
        assert_eq!(store.list().unwrap(), vec![last.clone()]);
        assert!(store.save(b"over limit").is_err());
        assert_eq!(store.read(&last).unwrap(), b"last");
    }

    #[test]
    fn rejects_linked_oversized_or_public_generation_without_losing_previous() {
        let dir = directory();
        let store = RunStore::open(dir.path(), &[]).unwrap();
        let safe = store.save(b"safe").unwrap();
        let other = store.save(b"other").unwrap();
        let other_path = dir.path().join(&other);
        std::fs::hard_link(&other_path, dir.path().join("foreign-link")).unwrap();
        assert!(store.read(&other).is_err());
        std::fs::remove_file(dir.path().join("foreign-link")).unwrap();
        std::fs::set_permissions(&other_path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(store.read(&other).is_err());
        std::fs::set_permissions(&other_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&other_path)
            .unwrap()
            .set_len((MAX_SAVE_BYTES + 1) as u64)
            .unwrap();
        assert!(store.read(&other).is_err());
        assert!(store.save(b"new").is_err());
        assert_eq!(store.read(&safe).unwrap(), b"safe");
    }

    #[test]
    fn opened_directory_survives_path_replacement_without_redirecting_write() {
        let outer = directory();
        let path = outer.path().join("saves");
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let store = RunStore::open(&path, &[]).unwrap();
        std::fs::rename(&path, outer.path().join("original")).unwrap();
        symlink("/tmp", &path).unwrap();
        let name = store.save(b"pinned").unwrap();
        assert_eq!(
            std::fs::read(outer.path().join("original").join(name)).unwrap(),
            b"pinned"
        );
    }
}
