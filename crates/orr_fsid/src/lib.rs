//! Folder and file identity that works the same on Windows and Unix.
//!
//! `fs::canonicalize` on Windows returns `\\?\C:\...` paths. The first component
//! (`\\?\C:`) is only a prefix, not a folder. Reading its metadata fails with os error 1.
//! `ancestors_to_check` leaves that prefix out.
//!
//! Std `Metadata` has no stable file index on Windows, so a creation-time comparison is
//! weak. This crate reads the volume serial number and the file index from an open handle
//! on Windows (`GetFileInformationByHandle`) and the device and inode number on Unix.
use std::{
    fs::File,
    io,
    path::{Component, Path, PathBuf},
};

/// Identity of an open file or folder: the volume plus the index inside the volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileId {
    volume: u64,
    index: u64,
}

/// Read the identity of an open handle.
pub fn file_id(file: &File) -> io::Result<FileId> {
    imp::file_id(file)
}

/// Open a folder without following a link at the end of the path.
///
/// The call fails when the path is a file, a symbolic link, a junction or another reparse
/// point. On Windows the handle uses backup semantics, which a folder handle needs.
pub fn open_directory(path: &Path) -> io::Result<File> {
    let file = imp::open_directory(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_dir() || imp::is_link(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path must be a real folder, not a link or reparse point",
        ));
    }
    Ok(file)
}

/// Open a regular file without following a link at the end of the path.
///
/// The call fails when the path is a folder, a symbolic link, a reparse point, a FIFO or
/// another special file. On Unix the open does not wait for a writer.
pub fn open_file(path: &Path) -> io::Result<File> {
    let file = imp::open_file(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || imp::is_link(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path must be a regular file, not a link or reparse point",
        ));
    }
    Ok(file)
}

/// Identity of what `path` names, after the call follows links.
///
/// Use it to ask whether two paths reach the same file or folder. A link and its target
/// give the same identity. On Unix the open does not wait for a writer.
pub fn file_id_at(path: &Path) -> io::Result<FileId> {
    file_id(&imp::open_following(path)?)
}

/// True when `metadata` describes a symbolic link or a Windows reparse point.
pub fn is_link(metadata: &std::fs::Metadata) -> bool {
    imp::is_link(metadata)
}

/// True when `path` names the same folder as the open handle `pinned`.
pub fn same_directory(path: &Path, pinned: &File) -> io::Result<bool> {
    Ok(file_id(&open_directory(path)?)? == file_id(pinned)?)
}

/// True when `path` names the same regular file as the open handle `held`.
pub fn same_file(path: &Path, held: &File) -> io::Result<bool> {
    Ok(file_id(&open_file(path)?)? == file_id(held)?)
}

/// True when the two open handles name the same file or folder.
pub fn same_handle(first: &File, second: &File) -> io::Result<bool> {
    Ok(file_id(first)? == file_id(second)?)
}

/// A folder opened once and checked again later by path.
///
/// The handle stays open while this value lives, so the identity cannot be reused by
/// another folder in the meantime.
#[derive(Debug)]
pub struct PinnedDirectory {
    path: PathBuf,
    file: File,
    id: FileId,
}

impl PinnedDirectory {
    /// Open `path` as a real folder and remember its identity.
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = open_directory(path)?;
        let id = file_id(&file)?;
        Ok(Self {
            path: path.to_path_buf(),
            file,
            id,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn file(&self) -> &File {
        &self.file
    }

    pub fn id(&self) -> FileId {
        self.id
    }

    /// Fail when the pinned path no longer names the pinned folder.
    pub fn verify_unchanged(&self) -> io::Result<()> {
        self.verify_unchanged_at(&self.path)
    }

    /// Fail when `path` is not the pinned path or no longer names the pinned folder.
    pub fn verify_unchanged_at(&self, path: &Path) -> io::Result<()> {
        if path != self.path
            || file_id(&open_directory(path)?)? != self.id
            || file_id(&self.file)? != self.id
        {
            return Err(io::Error::other("folder changed since it was opened"));
        }
        Ok(())
    }
}

/// The folders that make up an absolute `path`, from the root down to `path` itself.
///
/// A Windows prefix such as `C:` or `\\?\C:` is not a folder and is left out. A relative
/// path or a `..` component is an error.
pub fn ancestors_to_check(path: &Path) -> io::Result<Vec<PathBuf>> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path must be absolute",
        ));
    }
    let mut current = PathBuf::new();
    let mut folders = Vec::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "path must not contain parent traversal",
                ))
            }
            Component::CurDir => {}
            Component::Prefix(_) => current.push(component.as_os_str()),
            Component::RootDir | Component::Normal(_) => {
                current.push(component.as_os_str());
                folders.push(current.clone());
            }
        }
    }
    Ok(folders)
}

/// Check that every folder in the absolute `path` exists and is a real folder, not a link.
pub fn check_real_folders(path: &Path) -> io::Result<()> {
    for folder in ancestors_to_check(path)? {
        let metadata = std::fs::symlink_metadata(&folder)?;
        if !metadata.is_dir() || imp::is_link(&metadata) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not a real folder", folder.display()),
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
mod imp {
    use super::FileId;
    use std::{
        fs::{File, Metadata, OpenOptions},
        io,
        os::windows::{
            fs::{MetadataExt, OpenOptionsExt},
            io::AsRawHandle,
        },
        path::Path,
    };

    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

    #[repr(C)]
    #[derive(Default)]
    struct ByHandleFileInformation {
        attributes: u32,
        created: [u32; 2],
        accessed: [u32; 2],
        written: [u32; 2],
        volume: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(
            handle: *mut std::ffi::c_void,
            info: *mut ByHandleFileInformation,
        ) -> i32;
    }

    pub fn file_id(file: &File) -> io::Result<FileId> {
        let mut info = ByHandleFileInformation::default();
        // SAFETY: the handle is valid while `file` is borrowed and `info` has the Win32 layout.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(FileId {
            volume: u64::from(info.volume),
            index: (u64::from(info.index_high) << 32) | u64::from(info.index_low),
        })
    }

    pub fn open_directory(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
    }

    pub fn is_link(metadata: &Metadata) -> bool {
        metadata.file_type().is_symlink()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    pub fn open_file(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
    }

    pub fn open_following(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
    }
}

#[cfg(unix)]
mod imp {
    use super::FileId;
    use std::{
        fs::{File, Metadata, OpenOptions},
        io,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
        path::Path,
    };

    pub fn file_id(file: &File) -> io::Result<FileId> {
        let metadata = file.metadata()?;
        Ok(FileId {
            volume: metadata.dev(),
            index: metadata.ino(),
        })
    }

    pub fn open_directory(path: &Path) -> io::Result<File> {
        // The kernel checks the type while it opens the path, so no other entry can be
        // swapped in between a check and the open. `O_DIRECTORY` refuses a FIFO, a file
        // or a device before the open can block. `O_NONBLOCK` keeps any other open from
        // waiting. `O_NOFOLLOW` refuses a symbolic link at the end of the path.
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
    }

    pub fn is_link(metadata: &Metadata) -> bool {
        metadata.file_type().is_symlink()
    }

    pub fn open_file(path: &Path) -> io::Result<File> {
        // `O_NOFOLLOW` refuses a final symbolic link. `O_NONBLOCK` keeps the open from
        // waiting on a FIFO that a path swap may have put in place of the file.
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)
    }

    pub fn open_following(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use super::FileId;
    use std::{
        fs::{File, Metadata},
        io,
        path::Path,
    };

    fn unsupported<T>() -> io::Result<T> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "file identity is unavailable on this platform",
        ))
    }
    pub fn file_id(_: &File) -> io::Result<FileId> {
        unsupported()
    }
    pub fn open_directory(_: &Path) -> io::Result<File> {
        unsupported()
    }
    pub fn open_file(_: &Path) -> io::Result<File> {
        unsupported()
    }
    pub fn open_following(_: &Path) -> io::Result<File> {
        unsupported()
    }
    pub fn is_link(metadata: &Metadata) -> bool {
        metadata.file_type().is_symlink()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn identity_matches_for_one_folder_and_differs_for_two() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        let a = file_id(&open_directory(&first).unwrap()).unwrap();
        let b = file_id(&open_directory(&first).unwrap()).unwrap();
        let c = file_id(&open_directory(&second).unwrap()).unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn a_file_is_not_a_folder() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("file.txt");
        fs::write(&file, b"x").unwrap();
        assert!(open_directory(&file).is_err());
    }

    #[test]
    fn pinned_folder_detects_a_different_folder_at_the_same_path() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("project");
        let moved = temp.path().join("moved");
        fs::create_dir(&path).unwrap();
        let pinned = PinnedDirectory::open(&path).unwrap();
        pinned.verify_unchanged().unwrap();
        assert!(same_directory(&path, pinned.file()).unwrap());
        fs::rename(&path, &moved).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(pinned.verify_unchanged().is_err());
        assert!(!same_directory(&path, pinned.file()).unwrap());
        assert!(same_directory(&moved, pinned.file()).unwrap());
    }

    #[test]
    fn a_file_keeps_its_identity_and_a_replacement_differs() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("file.txt");
        let moved = temp.path().join("moved.txt");
        fs::write(&path, b"x").unwrap();
        let held = open_file(&path).unwrap();
        assert!(same_file(&path, &held).unwrap());
        fs::rename(&path, &moved).unwrap();
        fs::write(&path, b"x").unwrap();
        assert!(!same_file(&path, &held).unwrap());
        assert!(same_file(&moved, &held).unwrap());
    }

    #[test]
    fn identity_at_a_path_follows_a_link_to_the_same_target() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("file.txt");
        let alias = temp.path().join("alias.txt");
        let other = temp.path().join("other.txt");
        fs::write(&file, b"x").unwrap();
        fs::write(&other, b"x").unwrap();
        fs::hard_link(&file, &alias).unwrap();
        assert_eq!(file_id_at(&file).unwrap(), file_id_at(&alias).unwrap());
        assert_ne!(file_id_at(&file).unwrap(), file_id_at(&other).unwrap());
        assert_eq!(
            file_id_at(temp.path()).unwrap(),
            file_id(&open_directory(temp.path()).unwrap()).unwrap()
        );
        assert!(file_id_at(&temp.path().join("missing")).is_err());
    }

    #[test]
    fn two_handles_to_one_file_match_and_a_replacement_does_not() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("file.txt");
        fs::write(&path, b"x").unwrap();
        let first = open_file(&path).unwrap();
        let second = open_file(&path).unwrap();
        assert!(same_handle(&first, &second).unwrap());
        fs::remove_file(&path).unwrap();
        fs::write(&path, b"x").unwrap();
        let replacement = open_file(&path).unwrap();
        assert!(!same_handle(&first, &replacement).unwrap());
    }

    #[test]
    fn a_folder_is_not_a_file() {
        let temp = tempfile::tempdir().unwrap();
        assert!(open_file(temp.path()).is_err());
    }

    #[test]
    fn pinned_folder_rejects_another_path() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("project");
        let other = temp.path().join("other");
        fs::create_dir(&path).unwrap();
        fs::create_dir(&other).unwrap();
        let pinned = PinnedDirectory::open(&path).unwrap();
        assert!(pinned.verify_unchanged_at(&other).is_err());
    }

    #[test]
    fn ancestors_are_absolute_folders_without_a_bare_prefix() {
        let temp = tempfile::tempdir().unwrap();
        let canonical = fs::canonicalize(temp.path()).unwrap();
        let folders = ancestors_to_check(&canonical).unwrap();
        assert!(!folders.is_empty());
        assert_eq!(folders.last().unwrap(), &canonical);
        for folder in &folders {
            assert!(fs::symlink_metadata(folder).unwrap().is_dir(), "{folder:?}");
        }
        check_real_folders(&canonical).unwrap();
        assert!(ancestors_to_check(Path::new("relative")).is_err());
    }

    #[test]
    fn parent_traversal_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("a").join("..").join("b");
        assert!(ancestors_to_check(&path).is_err());
    }

    #[cfg(unix)]
    mod unix {
        use super::*;
        use std::{sync::mpsc, time::Duration};

        fn make_fifo(path: &Path) {
            let status = std::process::Command::new("mkfifo")
                .arg(path)
                .status()
                .unwrap();
            assert!(status.success(), "{status:?}");
        }

        /// Run `check` on another thread. When it does not finish in time, open the FIFO
        /// for writing so the blocked thread can end, then fail the test.
        fn finishes_in_time(fifo: &Path, check: impl FnOnce() + Send + 'static) {
            let (done, wait) = mpsc::channel();
            std::thread::spawn(move || {
                check();
                let _ = done.send(());
            });
            if wait.recv_timeout(Duration::from_secs(5)).is_err() {
                let _release = fs::OpenOptions::new().read(true).write(true).open(fifo);
                panic!("opening a FIFO as a folder blocked");
            }
        }

        #[test]
        fn a_fifo_is_refused_without_blocking() {
            let temp = tempfile::tempdir().unwrap();
            let fifo = temp.path().join("fifo");
            make_fifo(&fifo);
            let path = fifo.clone();
            finishes_in_time(&fifo, move || {
                assert!(open_directory(&path).is_err());
                assert!(PinnedDirectory::open(&path).is_err());
            });
        }

        #[test]
        fn a_pinned_folder_replaced_by_a_fifo_is_refused_without_blocking() {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("project");
            fs::create_dir(&path).unwrap();
            let pinned = PinnedDirectory::open(&path).unwrap();
            fs::rename(&path, temp.path().join("moved")).unwrap();
            make_fifo(&path);
            let fifo = path.clone();
            finishes_in_time(&fifo, move || {
                assert!(pinned.verify_unchanged().is_err());
                assert!(same_directory(&path, pinned.file()).is_err());
            });
        }

        #[test]
        fn a_symbolic_link_to_a_folder_is_refused() {
            let temp = tempfile::tempdir().unwrap();
            let target = temp.path().join("target");
            let link = temp.path().join("link");
            fs::create_dir(&target).unwrap();
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert!(open_directory(&link).is_err());
        }
    }

    #[cfg(windows)]
    mod windows {
        use super::*;
        use std::path::Component;

        #[test]
        fn canonical_paths_have_a_verbatim_prefix_that_has_no_metadata() {
            let temp = tempfile::tempdir().unwrap();
            let canonical = fs::canonicalize(temp.path()).unwrap();
            let Some(Component::Prefix(prefix)) = canonical.components().next() else {
                panic!("a canonical Windows path starts with a prefix");
            };
            // The failure this crate avoids: the bare prefix is not a folder.
            assert!(fs::symlink_metadata(Path::new(prefix.as_os_str())).is_err());
            assert!(ancestors_to_check(&canonical)
                .unwrap()
                .iter()
                .all(|folder| folder.as_os_str() != prefix.as_os_str()));
            check_real_folders(&canonical).unwrap();
        }

        #[test]
        fn a_junction_is_refused_as_a_folder_and_as_an_ancestor() {
            let temp = tempfile::tempdir().unwrap();
            let canonical = fs::canonicalize(temp.path()).unwrap();
            let target = canonical.join("target");
            let junction = canonical.join("junction");
            fs::create_dir(&target).unwrap();
            let status = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&junction)
                .arg(&target)
                .output()
                .unwrap();
            assert!(status.status.success(), "{status:?}");
            assert!(open_directory(&junction).is_err());
            assert!(open_file(&junction).is_err());
            assert!(check_real_folders(&junction).is_err());
            assert!(check_real_folders(&junction.join("inner")).is_err());
        }

        #[test]
        fn the_file_index_separates_folders_that_share_a_creation_time() {
            let temp = tempfile::tempdir().unwrap();
            let first = temp.path().join("first");
            let second = temp.path().join("second");
            fs::create_dir(&first).unwrap();
            fs::create_dir(&second).unwrap();
            let one = file_id(&open_directory(&first).unwrap()).unwrap();
            let two = file_id(&open_directory(&second).unwrap()).unwrap();
            assert_ne!(one.index, two.index);
            assert_eq!(one.volume, two.volume);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_is_refused_as_a_folder() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        let link = temp.path().join("link");
        fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(open_directory(&link).is_err());
        assert!(check_real_folders(&link).is_err());
    }
}
