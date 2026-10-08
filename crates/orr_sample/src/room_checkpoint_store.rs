//! Dedicated, bounded RoomEscape key checkpoint persistence.
//! A session owns one advisory lock. Invalid or changed bytes are never repaired
//! automatically; reset writes a false checkpoint rather than deleting history.
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    path::{Component, Path, PathBuf},
};

pub const PROFILE: &str = "room-key-checkpoint-v1";
pub const MAX_CHECKPOINT_BYTES: usize = 4096;
pub const PRIMARY_NAME: &str = "checkpoint.json";
pub const LOCK_NAME: &str = "checkpoint.lock";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckpointKey {
    pub game_id: [u8; 16],
    pub challenge: [u8; 32],
}
impl CheckpointKey {
    fn validate(&self) -> Result<(), String> {
        if self.game_id[6] >> 4 != 4 || self.game_id[8] >> 6 != 2 {
            return Err("checkpoint requires a UUIDv4 game identity".into());
        }
        Ok(())
    }
    fn game(&self) -> String {
        let h = hex(&self.game_id);
        format!(
            "{}-{}-{}-{}-{}",
            &h[..8],
            &h[8..12],
            &h[12..16],
            &h[16..20],
            &h[20..]
        )
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Closed envelope: serde rejects duplicate/unknown/missing fields and non-bools.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointV1 {
    schema: u32,
    game_id: String,
    profile: String,
    challenge: String,
    key_collected: bool,
}
impl CheckpointV1 {
    fn new(key: &CheckpointKey, key_collected: bool) -> Self {
        Self {
            schema: 1,
            game_id: key.game(),
            profile: PROFILE.into(),
            challenge: hex(&key.challenge),
            key_collected,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckpointPaths {
    directory: PathBuf,
}
impl CheckpointPaths {
    /// Caller must also exclude known project/export roots.
    pub fn from_directory(directory: impl Into<PathBuf>) -> Result<Self, String> {
        let directory = directory.into();
        validate_directory_path(&directory)?;
        Ok(Self { directory })
    }
    pub fn from_environment(key: &CheckpointKey) -> Result<Self, String> {
        Self::from_environment_values(
            std::env::var_os("XDG_DATA_HOME").as_deref(),
            std::env::var_os("HOME").as_deref(),
            key,
        )
    }
    pub fn from_environment_values(
        xdg: Option<&OsStr>,
        home: Option<&OsStr>,
        key: &CheckpointKey,
    ) -> Result<Self, String> {
        key.validate()?;
        let base = if let Some(path) = xdg.map(Path::new).filter(|p| p.is_absolute()) {
            path.to_path_buf()
        } else if let Some(path) = home.map(Path::new).filter(|p| p.is_absolute()) {
            path.join(".local/share")
        } else {
            return Err("checkpoint requires absolute XDG_DATA_HOME or HOME".into());
        };
        Self::from_directory(
            base.join("orrery/games")
                .join(key.game())
                .join(PROFILE)
                .join(hex(&key.challenge)),
        )
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
}
fn validate_directory_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || path.parent().is_none() {
        return Err("checkpoint directory must be an absolute non-root path".into());
    }
    if path.components().count() > 128 || path.as_os_str().as_encoded_bytes().len() > 4096 {
        return Err("checkpoint directory path exceeds bounds".into());
    }
    // Path::components normalizes embedded '.', so inspect the original bytes too.
    if path
        .as_os_str()
        .as_encoded_bytes()
        .split(|byte| *byte == b'/')
        .any(|part| part == b"." || part == b".." || part.len() > 255 || part.contains(&0))
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err("checkpoint directory contains an unsafe path component".into());
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitOutcome {
    DurablySaved,
    /// Publication happened, but its durability could not be confirmed.
    DurabilityUncertain(String),
}

#[derive(Debug)]
pub struct CheckpointStore {
    key_collected: bool,
    notice: Option<String>,
    #[cfg(target_os = "linux")]
    session: Option<linux::Session>,
}
impl CheckpointStore {
    pub fn open(paths: CheckpointPaths, key: CheckpointKey) -> Result<Self, String> {
        key.validate()?;
        #[cfg(target_os = "linux")]
        {
            Ok(linux::open(paths, key))
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = paths;
            Ok(Self {
                key_collected: false,
                notice: Some("checkpoint persistence is supported only on Linux".into()),
            })
        }
    }
    pub fn key_collected(&self) -> bool {
        self.key_collected
    }
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }
    pub fn save_key(&mut self) -> Result<CommitOutcome, String> {
        self.commit_with_checkpoint(true, |_| Ok(()))
    }
    /// Persist a new empty run. Never merges the previous collected key back in.
    pub fn reset(&mut self) -> Result<CommitOutcome, String> {
        self.commit_with_checkpoint(false, |_| Ok(()))
    }
    fn commit_with_checkpoint(
        &mut self,
        collected: bool,
        checkpoint: impl FnMut(&str) -> Result<(), String>,
    ) -> Result<CommitOutcome, String> {
        #[cfg(target_os = "linux")]
        {
            let session = self.session.as_mut().ok_or_else(|| {
                self.notice
                    .clone()
                    .unwrap_or_else(|| "checkpoint is read-only".into())
            })?;
            let outcome = session.commit(collected, checkpoint)?;
            self.key_collected = collected;
            match &outcome {
                CommitOutcome::DurablySaved => self.notice = None,
                CommitOutcome::DurabilityUncertain(reason) => {
                    self.notice = Some(reason.clone());
                    // Keep the session lock until drop, but prohibit retries.
                    session.writable = false;
                }
            }
            Ok(outcome)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (collected, checkpoint);
            Err("checkpoint persistence is supported only on Linux".into())
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use rustix::fs::{self, AtFlags, FileType, FlockOperation, Mode, OFlags};
    use std::{
        fs::File,
        io::{Read, Write},
        os::unix::fs::MetadataExt,
        sync::atomic::{AtomicU64, Ordering},
    };
    const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC);
    const FILE_FLAGS: OFlags = OFlags::NOFOLLOW
        .union(OFlags::NONBLOCK)
        .union(OFlags::CLOEXEC);
    static NEXT_STAGE: AtomicU64 = AtomicU64::new(0);

    #[derive(Debug, PartialEq, Eq)]
    enum Snapshot {
        Missing,
        Valid { collected: bool, bytes: Vec<u8> },
        Preserved(String),
    }
    fn decode(bytes: Vec<u8>, key: &CheckpointKey) -> Snapshot {
        if bytes.len() > MAX_CHECKPOINT_BYTES {
            return Snapshot::Preserved("checkpoint exceeds bounds; preserved read-only".into());
        }
        if bytes
            .iter()
            .copied()
            .find(|byte| !byte.is_ascii_whitespace())
            != Some(b'{')
        {
            return Snapshot::Preserved(
                "checkpoint envelope must be an object; preserved read-only".into(),
            );
        }
        match serde_json::from_slice::<CheckpointV1>(&bytes) {
            Ok(value)
                if value.schema == 1
                    && value.game_id == key.game()
                    && value.profile == PROFILE
                    && value.challenge == hex(&key.challenge) =>
            {
                Snapshot::Valid {
                    collected: value.key_collected,
                    bytes,
                }
            }
            Ok(_) => Snapshot::Preserved(
                "unsupported checkpoint schema or identity mismatch; preserved read-only".into(),
            ),
            Err(_) => Snapshot::Preserved(
                "invalid checkpoint envelope; existing bytes preserved read-only".into(),
            ),
        }
    }
    fn error(error: impl std::fmt::Display) -> String {
        error.to_string()
    }
    /// Every component is opened relative to its verified parent. Missing reads
    /// return None without creating even a lock file. Creation is confined to
    /// explicit standalone profile-session startup, never metadata admission.
    fn open_directory(paths: &CheckpointPaths, create: bool) -> Result<Option<File>, String> {
        validate_directory_path(paths.directory())?;
        let mut directory =
            File::from(fs::open("/", DIRECTORY_FLAGS, Mode::empty()).map_err(error)?);
        for part in paths.directory().components() {
            let Component::Normal(name) = part else {
                continue;
            };
            let next = match fs::openat(&directory, name, DIRECTORY_FLAGS, Mode::empty()) {
                Ok(next) => next,
                Err(rustix::io::Errno::NOENT) if !create => return Ok(None),
                Err(rustix::io::Errno::NOENT) => {
                    if directory.metadata().map_err(error)?.mode() & 0o222 == 0 {
                        return Err("checkpoint parent directory is read-only".into());
                    }
                    match fs::mkdirat(&directory, name, Mode::RUSR | Mode::WUSR | Mode::XUSR) {
                        Ok(()) => directory.sync_all().map_err(error)?,
                        Err(rustix::io::Errno::EXIST) => (),
                        Err(failure) => return Err(error(failure)),
                    }
                    fs::openat(&directory, name, DIRECTORY_FLAGS, Mode::empty()).map_err(error)?
                }
                Err(failure) => {
                    return Err(format!(
                        "unsafe or unavailable checkpoint directory: {failure}"
                    ));
                }
            };
            directory = File::from(next);
            let metadata = directory.metadata().map_err(error)?;
            let owner = metadata.uid();
            if (owner != 0 && owner != rustix::process::geteuid().as_raw())
                || (metadata.mode() & 0o022 != 0 && !(owner == 0 && metadata.mode() & 0o1000 != 0))
            {
                return Err(
                    "checkpoint ancestor must be trusted and not writable by other users".into(),
                );
            }
        }
        let metadata = directory.metadata().map_err(error)?;
        if metadata.uid() != rustix::process::geteuid().as_raw() || metadata.mode() & 0o022 != 0 {
            return Err(
                "checkpoint directory must be owned by this user and not writable by other users"
                    .into(),
            );
        }
        if create && metadata.mode() & 0o200 == 0 {
            return Err("checkpoint directory is read-only".into());
        }
        Ok(Some(directory))
    }

    fn stat_regular(directory: &File, name: &str) -> Result<Option<fs::Stat>, String> {
        let stat = match fs::statat(directory, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(failure) => return Err(error(failure)),
        };
        if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
            || stat.st_nlink != 1
            || stat.st_uid != rustix::process::geteuid().as_raw()
            || stat.st_mode & 0o022 != 0
        {
            return Err(format!(
                "{name} must be a user-owned regular file without links or other-user writes"
            ));
        }
        Ok(Some(stat))
    }

    fn same_file(a: &fs::Stat, b: &fs::Stat) -> bool {
        a.st_dev == b.st_dev && a.st_ino == b.st_ino
    }

    fn same_identity(a: Option<&fs::Stat>, b: Option<&fs::Stat>) -> bool {
        match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                same_file(a, b)
                    && a.st_mode == b.st_mode
                    && a.st_nlink == b.st_nlink
                    && a.st_size == b.st_size
                    && a.st_mtime == b.st_mtime
                    && a.st_mtime_nsec == b.st_mtime_nsec
                    && a.st_ctime == b.st_ctime
                    && a.st_ctime_nsec == b.st_ctime_nsec
            }
            _ => false,
        }
    }

    fn read_snapshot(
        directory: &File,
        name: &str,
        key: &CheckpointKey,
    ) -> Result<Snapshot, String> {
        let Some(before) = stat_regular(directory, name)? else {
            return Ok(Snapshot::Missing);
        };
        if before.st_size < 0 || before.st_size as u64 > MAX_CHECKPOINT_BYTES as u64 {
            return Ok(Snapshot::Preserved(format!(
                "{name} exceeds {MAX_CHECKPOINT_BYTES} bytes; preserved read-only"
            )));
        }
        let file = File::from(
            fs::openat(directory, name, OFlags::RDONLY | FILE_FLAGS, Mode::empty())
                .map_err(error)?,
        );
        let opened = fs::fstat(&file).map_err(error)?;
        if !same_file(&before, &opened)
            || FileType::from_raw_mode(opened.st_mode) != FileType::RegularFile
            || opened.st_nlink != 1
            || opened.st_uid != rustix::process::geteuid().as_raw()
        {
            return Err(format!("{name} changed while opening"));
        }
        let mut bytes = Vec::new();
        (&file)
            .take((MAX_CHECKPOINT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(error)?;
        if bytes.len() > MAX_CHECKPOINT_BYTES {
            return Ok(Snapshot::Preserved(format!(
                "{name} exceeds {MAX_CHECKPOINT_BYTES} bytes; preserved read-only"
            )));
        }
        let after = fs::fstat(&file).map_err(error)?;
        if before.st_size != after.st_size
            || before.st_mtime != after.st_mtime
            || before.st_mtime_nsec != after.st_mtime_nsec
            || before.st_ctime != after.st_ctime
            || before.st_ctime_nsec != after.st_ctime_nsec
        {
            return Err(format!("{name} changed while reading"));
        }
        Ok(decode(bytes, key))
    }

    fn lock(directory: &File) -> Result<File, String> {
        let old = stat_regular(directory, LOCK_NAME)?;
        if old.as_ref().is_some_and(|stat| stat.st_mode & 0o200 == 0) {
            return Err("checkpoint lock is read-only".into());
        }
        let flags = if old.is_some() {
            OFlags::RDWR | FILE_FLAGS
        } else {
            OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | FILE_FLAGS
        };
        let fd = match fs::openat(directory, LOCK_NAME, flags, Mode::RUSR | Mode::WUSR) {
            Ok(fd) => fd,
            // Another cooperating writer may have created the stable lock.
            Err(rustix::io::Errno::EXIST) if old.is_none() => fs::openat(
                directory,
                LOCK_NAME,
                OFlags::RDWR | FILE_FLAGS,
                Mode::empty(),
            )
            .map_err(error)?,
            Err(failure) => return Err(error(failure)),
        };
        let file = File::from(fd);
        let Some(named) = stat_regular(directory, LOCK_NAME)? else {
            return Err("checkpoint lock disappeared".into());
        };
        if !same_file(&named, &fs::fstat(&file).map_err(error)?) {
            return Err("checkpoint lock changed while opening".into());
        }
        fs::flock(&file, FlockOperation::NonBlockingLockExclusive).map_err(|failure| {
            format!("checkpoint is locked or locking is unavailable: {failure}")
        })?;
        // Never remove or replace this inode, including on failure.
        Ok(file)
    }

    struct Stage<'a> {
        directory: &'a File,
        name: String,
        file: File,
    }
    impl Drop for Stage<'_> {
        fn drop(&mut self) {
            let _ = fs::unlinkat(self.directory, self.name.as_str(), AtFlags::empty());
        }
    }
    impl<'a> Stage<'a> {
        fn create(directory: &'a File) -> Result<Self, String> {
            for _ in 0..64 {
                let counter = NEXT_STAGE.fetch_add(1, Ordering::Relaxed);
                let name = format!(".checkpoint-stage-{}-{counter}", std::process::id());
                match fs::openat(
                    directory,
                    name.as_str(),
                    OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | FILE_FLAGS,
                    Mode::RUSR | Mode::WUSR,
                ) {
                    Ok(fd) => {
                        return Ok(Self {
                            directory,
                            name,
                            file: File::from(fd),
                        });
                    }
                    Err(rustix::io::Errno::EXIST) => (),
                    Err(failure) => return Err(error(failure)),
                }
            }
            Err("could not allocate a unique checkpoint stage".into())
        }
        fn publish(&self, name: &str) -> Result<(), String> {
            fs::renameat(self.directory, self.name.as_str(), self.directory, name).map_err(error)
        }
    }

    #[derive(Debug)]
    pub(super) struct Session {
        paths: CheckpointPaths,
        key: CheckpointKey,
        directory: File,
        lock: File,
        snapshot: Snapshot,
        identity: Option<fs::Stat>,
        pub(super) writable: bool,
    }
    pub(super) fn open(paths: CheckpointPaths, key: CheckpointKey) -> CheckpointStore {
        let mut collected = false;
        let result = (|| -> Result<Session, String> {
            let directory = match open_directory(&paths, false)? {
                Some(directory) => directory,
                None => open_directory(&paths, true)?.ok_or("checkpoint directory unavailable")?,
            };
            // Read-only contenders may display the current checkpoint, but never
            // acquire a later lock and overwrite a reset from an older session.
            let lock = if directory.metadata().map_err(error)?.mode() & 0o200 == 0 {
                Err("checkpoint directory is read-only".into())
            } else {
                lock(&directory)
            };
            let snapshot = read_snapshot(&directory, PRIMARY_NAME, &key)?;
            if let Snapshot::Valid {
                collected: value, ..
            } = &snapshot
            {
                collected = *value;
            }
            let lock = lock?;
            if let Snapshot::Preserved(reason) = &snapshot {
                return Err(reason.clone());
            }
            let identity = stat_regular(&directory, PRIMARY_NAME)?;
            if identity.as_ref().is_some_and(|s| s.st_mode & 0o200 == 0) {
                return Err("checkpoint file is read-only".into());
            }
            Ok(Session {
                paths,
                key,
                directory,
                lock,
                snapshot,
                identity,
                writable: true,
            })
        })();
        match result {
            Ok(session) => CheckpointStore {
                key_collected: collected,
                notice: None,
                session: Some(session),
            },
            Err(reason) => CheckpointStore {
                key_collected: collected,
                notice: Some(reason),
                session: None,
            },
        }
    }
    impl Session {
        fn verify_current(&self) -> Result<(), String> {
            let reopened =
                open_directory(&self.paths, false)?.ok_or("checkpoint directory disappeared")?;
            if !same_file(
                &fs::fstat(&self.directory).map_err(error)?,
                &fs::fstat(&reopened).map_err(error)?,
            ) {
                return Err("checkpoint directory changed; refusing publication".into());
            }
            let named_lock =
                stat_regular(&self.directory, LOCK_NAME)?.ok_or("checkpoint lock disappeared")?;
            if !same_file(&named_lock, &fs::fstat(&self.lock).map_err(error)?) {
                return Err("checkpoint lock changed; refusing publication".into());
            }
            if read_snapshot(&self.directory, PRIMARY_NAME, &self.key)? != self.snapshot
                || !same_identity(
                    self.identity.as_ref(),
                    stat_regular(&self.directory, PRIMARY_NAME)?.as_ref(),
                )
            {
                return Err("checkpoint changed outside this session; preserved read-only".into());
            }
            Ok(())
        }
        pub(super) fn commit(
            &mut self,
            collected: bool,
            mut checkpoint: impl FnMut(&str) -> Result<(), String>,
        ) -> Result<CommitOutcome, String> {
            if !self.writable {
                return Err("checkpoint session is read-only after uncertain durability".into());
            }
            self.verify_current()?;
            let mut bytes =
                serde_json::to_vec(&CheckpointV1::new(&self.key, collected)).map_err(error)?;
            bytes.push(b'\n');
            if bytes.len() > MAX_CHECKPOINT_BYTES {
                return Err("checkpoint exceeds bounds".into());
            }
            let mut published = false;
            let result = (|| -> Result<(), String> {
                let mut stage = Stage::create(&self.directory)?;
                checkpoint("primary-write")?;
                stage.file.write_all(&bytes).map_err(error)?;
                checkpoint("primary-sync")?;
                stage.file.sync_all().map_err(error)?;
                checkpoint("before-publication")?;
                self.verify_current()?;
                checkpoint("primary-rename")?;
                self.verify_current()?;
                stage.publish(PRIMARY_NAME)?;
                published = true;
                checkpoint("after-publication")?;
                checkpoint("directory-sync")?;
                self.directory.sync_all().map_err(error)?;
                Ok(())
            })();
            if !published {
                return result.map(|()| CommitOutcome::DurablySaved);
            }
            self.snapshot = Snapshot::Valid { collected, bytes };
            let identity_result = (|| -> Result<Option<fs::Stat>, String> {
                let identity = stat_regular(&self.directory, PRIMARY_NAME)?;
                if identity.is_none()
                    || read_snapshot(&self.directory, PRIMARY_NAME, &self.key)? != self.snapshot
                {
                    return Err("checkpoint changed after publication".into());
                }
                Ok(identity)
            })();
            match (result, identity_result) {
                (Ok(()), Ok(identity)) => {
                    self.identity = identity;
                    Ok(CommitOutcome::DurablySaved)
                }
                (Err(reason), _) | (_, Err(reason)) => {
                    Ok(CommitOutcome::DurabilityUncertain(format!(
                        "checkpoint published but durability confirmation failed: {reason}; do not retry automatically"
                    )))
                }
            }
        }
    }
}
#[cfg(all(test, target_os = "linux"))]
#[path = "room_checkpoint_store_tests.rs"]
mod tests;
