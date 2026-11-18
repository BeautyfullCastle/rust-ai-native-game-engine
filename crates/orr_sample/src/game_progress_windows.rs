//! Windows transport of the guarded progress store. It keeps the Linux protocol:
//! read-only discovery, a stable lock that is never replaced, two byte-checked
//! copies (primary and backup), staged files and a publication that never follows a
//! link. Windows has no directory descriptors, so every step re-opens by path and
//! compares the file identity (volume serial and file index). Locking is the
//! mandatory share mode of the lock file: a writer opens it with no sharing and a
//! reader opens it with read sharing only. The per-user data directory (by default
//! under `%LOCALAPPDATA%`) is protected by the access list it inherits; this store
//! does not read owners or access lists.
use super::shared::*;
use super::*;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    sync::atomic::{AtomicU64, Ordering},
};
use windows_sys::Win32::{
    Foundation::HANDLE,
    Storage::FileSystem::{
        GetFileInformationByHandle, MoveFileExW, BY_HANDLE_FILE_INFORMATION,
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    },
};

const FILE_ATTRIBUTE_READONLY: u32 = 0x0001;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0010;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_SHARE_READ: u32 = 1;
const SHARE_EVERYTHING: u32 = 7;
const ERROR_SHARING_VIOLATION: i32 = 32;
const ERROR_LOCK_VIOLATION: i32 = 33;
static NEXT_STAGE: AtomicU64 = AtomicU64::new(0);

/// What one open handle says about its file. Two stats of the same file have the
/// same `volume` and `index`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stat {
    volume: u32,
    index: u64,
    links: u32,
    size: u64,
    attributes: u32,
    written: u64,
    created: u64,
}

fn stat_file(file: &File) -> Result<Stat, String> {
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: the handle belongs to `file`, which outlives the call, and `info` is writable.
    let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut info) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let time = |low: u32, high: u32| (u64::from(high) << 32) | u64::from(low);
    Ok(Stat {
        volume: info.dwVolumeSerialNumber,
        index: time(info.nFileIndexLow, info.nFileIndexHigh),
        links: info.nNumberOfLinks,
        size: time(info.nFileSizeLow, info.nFileSizeHigh),
        attributes: info.dwFileAttributes,
        written: time(info.ftLastWriteTime.dwLowDateTime, info.ftLastWriteTime.dwHighDateTime),
        created: time(info.ftCreationTime.dwLowDateTime, info.ftCreationTime.dwHighDateTime),
    })
}

fn same_file(a: &Stat, b: &Stat) -> bool {
    a.volume == b.volume && a.index == b.index
}

fn same_identity(a: Option<&Stat>, b: Option<&Stat>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Opens the object itself (not what a link points to) only to ask about it.
fn open_for_stat(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .access_mode(0)
        .share_mode(SHARE_EVERYTHING)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

fn stat_directory(path: &Path) -> Result<Stat, String> {
    let stat = stat_file(&open_for_stat(path).map_err(error)?)?;
    if stat.attributes & FILE_ATTRIBUTE_DIRECTORY == 0
        || stat.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err("progress directory must be a plain directory".into());
    }
    Ok(stat)
}

fn stat_regular(directory: &Path, name: &str) -> Result<Option<Stat>, String> {
    let file = match open_for_stat(&directory.join(name)) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(error(e)),
    };
    let stat = stat_file(&file)?;
    if stat.attributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0
        || stat.links != 1
    {
        return Err(format!(
            "{name} must be a regular file without links or reparse points"
        ));
    }
    Ok(Some(stat))
}

/// Every existing component must be a plain directory. Missing ones are created
/// only when `create` is set (commit time). Nothing is created for a read.
fn open_directory(paths: &ProgressPaths, create: bool) -> Result<Option<PathBuf>, String> {
    validate_directory_path(paths.directory())?;
    let mut current = PathBuf::new();
    for part in paths.directory().components() {
        current.push(part);
        if !matches!(part, Component::Normal(_)) {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !create => return Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => match fs::create_dir(&current) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(e) => return Err(error(e)),
            },
            Err(e) => return Err(format!("unsafe or unavailable progress directory: {e}")),
        }
        let metadata = fs::symlink_metadata(&current)
            .map_err(|e| format!("unsafe or unavailable progress directory: {e}"))?;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("unsafe or unavailable progress directory: not a plain directory".into());
        }
    }
    Ok(Some(current))
}

fn read_snapshot(
    directory: &Path,
    name: &str,
    codec: StoreCodec<'_>,
) -> Result<Snapshot, String> {
    let Some(before) = stat_regular(directory, name)? else {
        return Ok(Snapshot::Missing);
    };
    if before.size > MAX_PROGRESS_BYTES as u64 {
        return Ok(Snapshot::Unsupported(format!(
            "{name} exceeds {MAX_PROGRESS_BYTES} bytes; preserved read-only"
        )));
    }
    let file = OpenOptions::new()
        .read(true)
        .share_mode(SHARE_EVERYTHING)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(directory.join(name))
        .map_err(error)?;
    let opened = stat_file(&file)?;
    if !same_file(&before, &opened) || opened.links != 1 {
        return Err(format!("{name} changed while opening"));
    }
    let mut bytes = Vec::new();
    (&file)
        .take((MAX_PROGRESS_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    if bytes.len() > MAX_PROGRESS_BYTES {
        return Ok(Snapshot::Unsupported(format!(
            "{name} exceeds {MAX_PROGRESS_BYTES} bytes; preserved read-only"
        )));
    }
    let after = stat_file(&file)?;
    if before.size != after.size || before.written != after.written {
        return Err(format!("{name} changed while reading"));
    }
    Ok(decode(bytes, codec))
}

pub(super) fn load(paths: &ProgressPaths, key: &ProgressKey) -> Loaded {
    load_with_checkpoint(paths, key, || {})
}

pub(super) fn load_with_checkpoint(
    paths: &ProgressPaths,
    key: &ProgressKey,
    checkpoint: impl FnOnce(),
) -> Loaded {
    load_codec_with_checkpoint(paths, StoreCodec::Collect(key), checkpoint)
}

#[cfg(feature = "room-progress")]
pub(super) fn load_codec(paths: &ProgressPaths, codec: StoreCodec<'_>) -> Loaded {
    load_codec_with_checkpoint(paths, codec, || {})
}

pub(super) fn load_codec_with_checkpoint(
    paths: &ProgressPaths,
    codec: StoreCodec<'_>,
    checkpoint: impl FnOnce(),
) -> Loaded {
    if let Err(e) = codec.validate() {
        return Loaded::readonly(0, e);
    }
    let names = codec.names();
    let directory = match open_directory(paths, false) {
        Ok(Some(d)) => d,
        Ok(None) => return loaded(&Snapshot::Missing, &Snapshot::Missing),
        Err(e) => return Loaded::readonly(0, e),
    };
    // Reading cooperates with publication. Never create a lock or directory
    // merely to discover saved progress.
    let guard = match read_lock(&directory, names.lock) {
        Ok(ReadLock::Busy) => {
            return Loaded::deferred("progress load deferred: another writer holds the lock");
        }
        Ok(guard) => guard,
        Err(e) => return Loaded::readonly(0, e),
    };
    checkpoint();
    let result = load_snapshots(&directory, codec);
    let named = match stat_regular(&directory, names.lock) {
        Ok(named) => named,
        Err(e) => return Loaded::readonly(0, e),
    };
    match guard {
        ReadLock::Missing if named.is_some() => {
            // The first writer creates its permanent lock before any publication.
            Loaded::deferred("progress load deferred: writer lock appeared during discovery")
        }
        ReadLock::Held(file) => match stat_file(&file) {
            Ok(opened) if named.as_ref().is_some_and(|s| same_file(s, &opened)) => result,
            Ok(_) => Loaded::readonly(0, "progress lock changed while loading"),
            Err(e) => Loaded::readonly(0, e),
        },
        ReadLock::Missing => result,
        ReadLock::Busy => unreachable!("busy load returned before reading"),
    }
}

fn load_snapshots(directory: &Path, codec: StoreCodec<'_>) -> Loaded {
    let names = codec.names();
    let primary = match read_snapshot(directory, names.primary, codec) {
        Ok(v) => v,
        Err(e) => return Loaded::readonly(0, e),
    };
    let backup = match read_snapshot(directory, names.backup, codec) {
        Ok(v) => v,
        Err(e) => return Loaded::readonly(loaded(&primary, &Snapshot::Missing).best, e),
    };
    let mut result = loaded(&primary, &backup);
    for name in [names.primary, names.backup, names.lock] {
        let failure = match stat_regular(directory, name) {
            Ok(Some(s)) if s.attributes & FILE_ATTRIBUTE_READONLY != 0 => {
                Some(format!("{name} is read-only"))
            }
            Err(e) => Some(e),
            _ => None,
        };
        if let Some(e) = failure {
            result = Loaded::readonly(result.best, e);
        }
    }
    result
}

enum ReadLock {
    Missing,
    Held(File),
    Busy,
}

fn is_lock_conflict(e: &std::io::Error) -> bool {
    matches!(
        e.raw_os_error(),
        Some(ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
    )
}

fn read_lock(directory: &Path, lock_name: &str) -> Result<ReadLock, String> {
    let Some(before) = stat_regular(directory, lock_name)? else {
        return Ok(ReadLock::Missing);
    };
    // Read access that allows only other readers: a writer, which needs write
    // access and no sharing, cannot open the file while this handle is open.
    let file = match OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(directory.join(lock_name))
    {
        Ok(file) => file,
        Err(e) if is_lock_conflict(&e) => return Ok(ReadLock::Busy),
        Err(e) => return Err(format!("progress read locking is unavailable: {e}")),
    };
    let opened = stat_file(&file)?;
    let named = stat_regular(directory, lock_name)?
        .ok_or("progress lock disappeared while opening for load")?;
    if !same_file(&before, &opened) || !same_file(&named, &opened) {
        return Err("progress lock changed while opening for load".into());
    }
    Ok(ReadLock::Held(file))
}

/// The exclusive writer lock: open for writing with no sharing at all.
fn lock(directory: &Path, lock_name: &str) -> Result<File, String> {
    let old = stat_regular(directory, lock_name)?;
    if old
        .as_ref()
        .is_some_and(|stat| stat.attributes & FILE_ATTRIBUTE_READONLY != 0)
    {
        return Err("progress lock is read-only".into());
    }
    let path = directory.join(lock_name);
    let open_existing = || {
        OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&path)
    };
    let opened = if old.is_some() {
        open_existing()
    } else {
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .share_mode(0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&path)
        {
            // Another cooperating writer may have created the stable lock.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => open_existing(),
            other => other,
        }
    };
    let file = opened
        .map_err(|e| format!("progress are locked or locking is unavailable: {e}"))?;
    let Some(named) = stat_regular(directory, lock_name)? else {
        return Err("progress lock disappeared".into());
    };
    if !same_file(&named, &stat_file(&file)?) {
        return Err("progress lock changed while opening".into());
    }
    // Never remove or replace this file, including on failure.
    Ok(file)
}

struct Stage<'a> {
    directory: &'a Path,
    name: String,
    file: Option<File>,
}

impl Drop for Stage<'_> {
    fn drop(&mut self) {
        self.file = None;
        let _ = fs::remove_file(self.directory.join(&self.name));
    }
}

impl<'a> Stage<'a> {
    fn create(directory: &'a Path) -> Result<Self, String> {
        for _ in 0..64 {
            let counter = NEXT_STAGE.fetch_add(1, Ordering::Relaxed);
            let name = format!(".progress-stage-{}-{counter}", std::process::id());
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(directory.join(&name))
            {
                Ok(file) => {
                    return Ok(Self {
                        directory,
                        name,
                        file: Some(file),
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(e) => return Err(error(e)),
            }
        }
        Err("could not allocate a unique progress stage".into())
    }

    fn write_and_sync(&mut self, bytes: &[u8], checkpoint_sync: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        let file = self.file.as_mut().ok_or("progress stage is closed")?;
        file.write_all(bytes).map_err(error)?;
        checkpoint_sync()?;
        let file = self.file.as_mut().ok_or("progress stage is closed")?;
        file.sync_all().map_err(error)
    }

    /// Moves the staged file over `name`. `MOVEFILE_WRITE_THROUGH` returns only after
    /// the move is on disk, which stands in for the directory sync of other systems.
    fn publish(&mut self, name: &str) -> Result<(), String> {
        // The stage handle must be closed before its file can be moved.
        self.file = None;
        let from = wide(&self.directory.join(&self.name))?;
        let to = wide(&self.directory.join(name))?;
        // SAFETY: both buffers are NUL terminated UTF-16 and live for the call.
        let moved = unsafe {
            MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if moved == 0 {
            return Err(error(std::io::Error::last_os_error()));
        }
        Ok(())
    }
}

fn wide(path: &Path) -> Result<Vec<u16>, String> {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err("progress path contains a NUL".into());
    }
    wide.push(0);
    Ok(wide)
}

pub(super) fn commit(
    paths: &ProgressPaths,
    key: &ProgressKey,
    candidate: u32,
    checkpoint: impl FnMut(&str) -> Result<(), String>,
) -> Result<(u32, CommitOutcome), String> {
    commit_codec(paths, StoreCodec::Collect(key), candidate, checkpoint)
}

pub(super) fn commit_codec(
    paths: &ProgressPaths,
    codec: StoreCodec<'_>,
    candidate: u32,
    mut checkpoint: impl FnMut(&str) -> Result<(), String>,
) -> Result<(u32, CommitOutcome), String> {
    codec.validate()?;
    codec.merge(0, candidate)?;
    let names = codec.names();
    let mut published = false;
    let mut committed_best = 0;
    let result = (|| -> Result<(), String> {
        let directory = open_directory(paths, true)?.ok_or("progress directory missing")?;
        let directory_identity = stat_directory(&directory)?;
        let lock_file = lock(&directory, names.lock)?;
        for name in [names.primary, names.backup] {
            if stat_regular(&directory, name)?
                .is_some_and(|stat| stat.attributes & FILE_ATTRIBUTE_READONLY != 0)
            {
                return Err(format!("{name} is read-only"));
            }
        }
        let primary = read_snapshot(&directory, names.primary, codec)?;
        let backup = read_snapshot(&directory, names.backup, codec)?;
        let primary_identity = stat_regular(&directory, names.primary)?;
        let backup_identity = stat_regular(&directory, names.backup)?;
        let state = loaded(&primary, &backup);
        if !matches!(state.write_state, WriteState::Ready) {
            return Err(state
                .notice
                .unwrap_or_else(|| "progress are read-only".into()));
        }
        committed_best = codec.merge(state.best, candidate)?;
        let mut bytes = codec.encode(committed_best)?;
        bytes.push(b'\n');
        if bytes.len() > MAX_PROGRESS_BYTES {
            return Err("serialized progress exceeds bounds".into());
        }
        // During recovery, preserve the known-good backup byte-for-byte.
        let backup_bytes = match (&primary, &backup) {
            (Snapshot::Valid { bytes, .. }, _) => Some(bytes.as_slice()),
            (_, Snapshot::Valid { .. }) => None,
            _ => Some(bytes.as_slice()),
        };
        let mut primary_stage = Stage::create(&directory)?;
        checkpoint("primary-write")?;
        primary_stage.write_and_sync(&bytes, || checkpoint("primary-sync"))?;
        let backup_stage = if let Some(backup_bytes) = backup_bytes {
            let mut stage = Stage::create(&directory)?;
            checkpoint("backup-write")?;
            stage.write_and_sync(backup_bytes, || checkpoint("backup-sync"))?;
            Some(stage)
        } else {
            None
        };
        checkpoint("before-publication")?;
        // Recheck every public name right before replacing anything.
        let reopened = open_directory(paths, false)?
            .ok_or("progress directory disappeared before publication")?;
        if !same_file(&directory_identity, &stat_directory(&reopened)?) {
            return Err("progress directory changed before publication".into());
        }
        // A non-cooperating writer may ignore our lock while staging. Re-read both
        // byte snapshots so a newly arrived future document is never downgraded.
        for (name, original, identity) in [
            (names.primary, &primary, &primary_identity),
            (names.backup, &backup, &backup_identity),
        ] {
            if read_snapshot(&directory, name, codec)? != *original
                || !same_identity(identity.as_ref(), stat_regular(&directory, name)?.as_ref())
            {
                return Err(format!(
                    "{name} changed during staging; progress were not published"
                ));
            }
        }
        let named_lock = stat_regular(&directory, names.lock)?
            .ok_or("progress lock disappeared before publication")?;
        if !same_file(&named_lock, &stat_file(&lock_file)?) {
            return Err("progress lock changed before publication".into());
        }
        checkpoint("primary-rename")?;
        primary_stage.publish(names.primary)?;
        published = true;
        checkpoint("after-publication")?;
        if let Some(mut stage) = backup_stage {
            checkpoint("backup-rename")?;
            stage.publish(names.backup)?;
        }
        checkpoint("directory-sync")?;
        Ok(())
    })();
    match (published, result) {
        (_, Ok(())) => Ok((committed_best, CommitOutcome::DurablySaved)),
        (false, Err(failure)) => Err(failure),
        (true, Err(failure)) => Ok((
            committed_best,
            CommitOutcome::DurabilityUncertain(format!(
                "progress were published, but backup/durability confirmation failed: {failure}; do not retry automatically"
            )),
        )),
    }
}
