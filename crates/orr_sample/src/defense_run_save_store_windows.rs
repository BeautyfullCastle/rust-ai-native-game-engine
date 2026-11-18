//! Windows transport of the native run generations. It keeps the Linux protocol: an
//! explicitly selected folder, a bounded scan, one new immutable file per save, a staged
//! file that is published without replacing anything, and no link or junction followed.
//!
//! Windows has no directory descriptors, so the folder is pinned by its identity (volume
//! serial number and file index, from `orr_fsid`) and checked again by path before each
//! scan, read and write. A stage is opened with a share mode that allows neither delete
//! nor rename, so nobody can swap it while it is written. `MoveFileExW` without
//! `MOVEFILE_REPLACE_EXISTING` fails when the name is taken, and `MOVEFILE_WRITE_THROUGH`
//! returns only after the move is on disk, which stands in for the directory sync of
//! other systems. The per-user data folder (by default under `%LOCALAPPDATA%`) is
//! protected by the access list it inherits; this store does not read owners or access lists.
use super::{valid_name, MAX_DIRECTORY_ENTRIES, MAX_SAVE_BYTES};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::windows::{
        ffi::OsStrExt,
        fs::OpenOptionsExt,
        io::AsRawHandle,
    },
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{
    Foundation::HANDLE,
    Storage::FileSystem::{GetFileInformationByHandle, MoveFileExW, BY_HANDLE_FILE_INFORMATION, MOVEFILE_WRITE_THROUGH},
};

const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0010;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_SHARE_READ: u32 = 1;
const FILE_SHARE_WRITE: u32 = 2;
static NEXT: AtomicU64 = AtomicU64::new(0);

/// The selected folder must already exist and every part of its path must be a real folder.
/// A folder that is renamed or replaced after it was opened is refused, not written into.
/// Existing files are never replaced.
#[derive(Debug)]
pub struct RunStore {
    directory: orr_fsid::PinnedDirectory,
}

fn err(e: impl std::fmt::Display) -> String {
    format!("run storage: {e}")
}

/// An absolute disk path of at most 128 parts with no `.`, `..` or characters that Windows
/// reads in a special way.
fn validate_path(path: &Path) -> Result<(), String> {
    let mut components = path.components();
    let disk = matches!(
        components.next(),
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_))
    ) && matches!(components.next(), Some(Component::RootDir));
    if !disk || path.file_name().is_none() {
        return Err(err("use an absolute directory path on a disk"));
    }
    if path.components().count() > 128 || path.as_os_str().as_encoded_bytes().len() > 4096 {
        return Err(err("directory path exceeds bounds"));
    }
    let text = path
        .to_str()
        .ok_or_else(|| err("directory path must be Unicode on Windows"))?;
    let text = text.strip_prefix(r"\\?\").unwrap_or(text);
    let mut parts = text.split(['\\', '/']);
    // The first part is the drive, which holds the only colon.
    let _drive = parts.next();
    let unsafe_part = parts.any(|part| {
        part == "."
            || part == ".."
            || part.len() > 255
            || part.contains([':', '\0', '<', '>', '"', '|', '?', '*'])
            || part.ends_with(['.', ' '])
    });
    if unsafe_part
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(err("directory path contains an unsafe component"));
    }
    Ok(())
}

/// Whether `path` is `root` or lies below it. Windows compares names without case.
fn inside(path: &Path, root: &Path) -> bool {
    let fold = |p: &Path| -> Vec<String> { p.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect() };
    let (path, root) = (fold(path), fold(root));
    path.len() >= root.len() && path[..root.len()] == root[..]
}

/// What an open handle says about its file.
struct Stat {
    attributes: u32,
    links: u32,
    size: u64,
}

fn stat(file: &File) -> Result<Stat, String> {
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: the handle belongs to `file`, which outlives the call, and `info` is writable.
    let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut info) };
    if ok == 0 {
        return Err(err(std::io::Error::last_os_error()));
    }
    Ok(Stat {
        attributes: info.dwFileAttributes,
        links: info.nNumberOfLinks,
        size: (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow),
    })
}

fn is_regular(stat: &Stat) -> bool {
    stat.attributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) == 0 && stat.links == 1
}

fn wide(path: &Path) -> Result<Vec<u16>, String> {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(err("path contains a NUL"));
    }
    wide.push(0);
    Ok(wide)
}

/// `%LOCALAPPDATA%\orrery\games\stormglass\runs`, or the same under `%USERPROFILE%\AppData\Local`.
pub fn default_directory() -> Result<PathBuf, String> {
    let absolute = |name: &str| std::env::var_os(name).map(PathBuf::from).filter(|p| p.is_absolute());
    let base = absolute("LOCALAPPDATA")
        .or_else(|| absolute("USERPROFILE").map(|home| home.join("AppData").join("Local")))
        .ok_or_else(|| err("saves need an absolute LOCALAPPDATA or USERPROFILE"))?;
    Ok(base.join("orrery").join("games").join("stormglass").join("runs"))
}

impl RunStore {
    /// Opens an existing folder. `forbidden_roots` are source, executable and scene folders.
    pub fn open(path: &Path, forbidden_roots: &[PathBuf]) -> Result<Self, String> {
        validate_path(path)?;
        orr_fsid::check_real_folders(path).map_err(err)?;
        let canonical = fs::canonicalize(path).map_err(err)?;
        for root in forbidden_roots {
            let root = fs::canonicalize(root).map_err(err)?;
            if inside(&canonical, &root) {
                return Err(err("save directory is inside a source or executable bundle root"));
            }
        }
        let directory = orr_fsid::PinnedDirectory::open(path).map_err(err)?;
        let store = Self { directory };
        store.list()?;
        Ok(store)
    }

    /// Opens the per-user folder of `default_directory`. A missing part is created, one real
    /// folder at a time; an existing part must be a real folder.
    pub fn open_default(forbidden_roots: &[PathBuf]) -> Result<Self, String> {
        let path = default_directory()?;
        validate_path(&path)?;
        for folder in orr_fsid::ancestors_to_check(&path).map_err(err)? {
            match fs::symlink_metadata(&folder) {
                Ok(_) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => match fs::create_dir(&folder) {
                    Ok(()) => (),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
                    Err(e) => return Err(err(e)),
                },
                Err(e) => return Err(err(e)),
            }
        }
        Self::open(&path, forbidden_roots)
    }

    fn check_directory(&self) -> Result<(), String> {
        self.directory.verify_unchanged().map_err(err)
    }

    /// Bounded folder scan, newest filenames first. Foreign names are never loaded or deleted.
    pub fn list(&self) -> Result<Vec<String>, String> {
        self.check_directory()?;
        let mut names = Vec::new();
        let mut count = 0;
        for entry in fs::read_dir(self.directory.path()).map_err(err)? {
            let entry = entry.map_err(err)?;
            count += 1;
            if count > MAX_DIRECTORY_ENTRIES {
                return Err(err("directory exceeds 256 entries; choose a smaller save directory"));
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else { continue };
            if valid_name(&name) {
                // Validate the opened object, not a possibly stale directory-entry type.
                self.open_save(&name)?;
                names.push(name);
            }
        }
        names.sort_unstable_by(|a, b| b.cmp(a));
        Ok(names)
    }

    fn open_save(&self, name: &str) -> Result<File, String> {
        if !valid_name(name) {
            return Err(err("invalid save generation name"));
        }
        // Open the object itself: a link is inspected, not followed.
        let file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(self.directory.path().join(name))
            .map_err(err)?;
        let stat = stat(&file)?;
        if !is_regular(&stat) || stat.size > MAX_SAVE_BYTES as u64 {
            return Err(err("save must be a single-link regular file of at most 4 MiB"));
        }
        Ok(file)
    }

    pub fn read(&self, name: &str) -> Result<Vec<u8>, String> {
        self.check_directory()?;
        let file = self.open_save(name)?;
        let mut bytes = Vec::new();
        file.take((MAX_SAVE_BYTES + 1) as u64).read_to_end(&mut bytes).map_err(err)?;
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
        let mut count = 0;
        for entry in fs::read_dir(self.directory.path()).map_err(err)? {
            entry.map_err(err)?;
            count += 1;
            if count >= MAX_DIRECTORY_ENTRIES {
                return Err(err("save directory is full (256 entries)"));
            }
        }
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_err(err)?.as_nanos();
        for _ in 0..32 {
            let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
            let name = format!("run-{stamp:032x}-{:08x}-{nonce:016x}.json", std::process::id());
            let staging = format!(".pending-{stamp:032x}-{:08x}-{nonce:016x}", std::process::id());
            let staging_path = self.directory.path().join(&staging);
            self.check_directory()?;
            let file = match OpenOptions::new()
                .write(true)
                .create_new(true)
                // Nobody can delete or rename the stage while this handle is open.
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(&staging_path)
            {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(err(e)),
            };
            let result = self.publish(file, bytes, &staging_path, &name);
            // Leave a failed stage for explicit user inspection. A retained stage counts
            // toward the bounded folder limit.
            return result.map(|()| name).map_err(|e| format!("{e}; any unpublished stage {staging} was retained"));
        }
        Err(err("could not reserve a unique generation; previous saves preserved"))
    }

    fn publish(&self, mut file: File, bytes: &[u8], staging_path: &Path, name: &str) -> Result<(), String> {
        file.write_all(bytes).map_err(err)?;
        file.sync_all().map_err(err)?;
        let written = stat(&file)?;
        let named = OpenOptions::new()
            .access_mode(0)
            .share_mode(7)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(staging_path)
            .map_err(err)?;
        let named_stat = stat(&named)?;
        if orr_fsid::file_id(&file).map_err(err)? != orr_fsid::file_id(&named).map_err(err)?
            || !is_regular(&named_stat)
            || named_stat.size != written.size
        {
            return Err(err("staging object changed before publication"));
        }
        drop(named);
        // The handle must be closed before its file can be moved.
        drop(file);
        self.check_directory()?;
        let from = wide(staging_path)?;
        let to = wide(&self.directory.path().join(name))?;
        // SAFETY: both buffers are NUL terminated UTF-16 and live for the call. Without
        // MOVEFILE_REPLACE_EXISTING the move fails when `name` already exists.
        let moved = unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) };
        if moved == 0 {
            return Err(err(std::io::Error::last_os_error()));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "defense_run_save_store_windows_tests.rs"]
mod tests;
