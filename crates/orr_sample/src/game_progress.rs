//! Bounded Linux highscore persistence outside game projects and exports.
//! Discovery is read-only. Commits merge under a stable advisory lock, publish
//! atomically, and distinguish publication from confirmed durable storage.
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    path::{Component, Path, PathBuf},
};

pub const PROFILE: &str = "collect-dodge-highscore-v1";
pub const MAX_PROGRESS_BYTES: usize = 4096;
pub const PRIMARY_NAME: &str = "highscore.json";
pub const BACKUP_NAME: &str = "highscore.json.bak";
pub const LOCK_NAME: &str = "highscore.lock";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgressKey {
    pub game_id: [u8; 16],
    pub challenge: [u8; 32],
    pub goal: u32,
}
impl ProgressKey {
    fn validate(&self) -> Result<(), String> {
        if self.game_id[6] >> 4 != 4
            || self.game_id[8] >> 6 != 2
            || self.goal == 0
            || self.goal > 32
        {
            return Err("progress requires a UUIDv4 game identity and goal in 1..=32".into());
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProgressV1 {
    version: u32,
    game_id: String,
    profile: String,
    challenge: String,
    #[serde(rename = "best_collected")]
    best: u32,
}
impl ProgressV1 {
    fn new(key: &ProgressKey, best: u32) -> Self {
        Self {
            version: 1,
            game_id: key.game(),
            profile: PROFILE.into(),
            challenge: hex(&key.challenge),
            best,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgressPaths {
    directory: PathBuf,
}
impl ProgressPaths {
    /// Caller must also exclude known project/export roots.
    pub fn from_directory(directory: impl Into<PathBuf>) -> Result<Self, String> {
        let directory = directory.into();
        validate_directory_path(&directory)?;
        Ok(Self { directory })
    }
    pub fn from_environment(key: &ProgressKey) -> Result<Self, String> {
        Self::from_environment_values(
            std::env::var_os("XDG_DATA_HOME").as_deref(),
            std::env::var_os("HOME").as_deref(),
            key,
        )
    }
    pub fn from_environment_values(
        xdg: Option<&OsStr>,
        home: Option<&OsStr>,
        key: &ProgressKey,
    ) -> Result<Self, String> {
        key.validate()?;
        let base = if let Some(path) = xdg.map(Path::new).filter(|p| p.is_absolute()) {
            path.to_path_buf()
        } else if let Some(path) = home.map(Path::new).filter(|p| p.is_absolute()) {
            path.join(".local/share")
        } else {
            return Err("progress requires absolute XDG_DATA_HOME or HOME".into());
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
        return Err("progress directory must be an absolute non-root path".into());
    }
    if path.components().count() > 128 || path.as_os_str().as_encoded_bytes().len() > 4096 {
        return Err("progress directory path exceeds bounds".into());
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
        return Err("progress directory contains an unsafe path component".into());
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitOutcome {
    DurablySaved,
    /// Primary was published; no automatic retry is safe.
    DurabilityUncertain(String),
}
#[derive(Clone, Debug)]
struct Loaded {
    best: u32,
    writable: bool,
    notice: Option<String>,
}
impl Loaded {
    fn readonly(best: u32, notice: impl Into<String>) -> Self {
        Self {
            best,
            writable: false,
            notice: Some(notice.into()),
        }
    }
}
#[derive(Debug)]
pub struct ProgressStore {
    paths: ProgressPaths,
    key: ProgressKey,
    loaded: Loaded,
}
impl ProgressStore {
    pub fn open(paths: ProgressPaths, key: ProgressKey) -> Result<Self, String> {
        key.validate()?;
        #[cfg(target_os = "linux")]
        let loaded = linux::load(&paths, &key);
        #[cfg(not(target_os = "linux"))]
        let loaded = Loaded::readonly(0, "progress persistence is supported only on Linux");
        Ok(Self { paths, key, loaded })
    }
    pub fn best(&self) -> u32 {
        self.loaded.best
    }
    pub fn notice(&self) -> Option<&str> {
        self.loaded.notice.as_deref()
    }
    pub fn submit_completed(&mut self, score: u32) -> Result<CommitOutcome, String> {
        self.submit_with_checkpoint(score, |_| Ok(()))
    }
    fn submit_with_checkpoint(
        &mut self,
        score: u32,
        checkpoint: impl FnMut(&str) -> Result<(), String>,
    ) -> Result<CommitOutcome, String> {
        if score > self.key.goal {
            return Err("score exceeds challenge goal".into());
        }
        if !self.loaded.writable {
            return Err(self
                .loaded
                .notice
                .clone()
                .unwrap_or_else(|| "progress is read-only".into()));
        }
        #[cfg(target_os = "linux")]
        {
            let (best, outcome) = linux::commit(
                &self.paths,
                &self.key,
                score.max(self.loaded.best),
                checkpoint,
            )?;
            self.loaded.best = best;
            self.loaded.notice = match &outcome {
                CommitOutcome::DurablySaved => None,
                CommitOutcome::DurabilityUncertain(reason) => Some(reason.clone()),
            };
            if matches!(outcome, CommitOutcome::DurabilityUncertain(_)) {
                self.loaded.writable = false;
            }
            Ok(outcome)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = checkpoint;
            Err("progress persistence is supported only on Linux".into())
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
        Valid { best: u32, bytes: Vec<u8> },
        Unsupported(String),
        Malformed { reason: String, bytes: Vec<u8> },
    }
    #[derive(Deserialize)]
    struct Envelope {
        version: Option<serde_json::Value>,
        game_id: Option<serde_json::Value>,
        profile: Option<serde_json::Value>,
        challenge: Option<serde_json::Value>,
    }
    fn decode(bytes: Vec<u8>, key: &ProgressKey) -> Snapshot {
        let envelope: Envelope = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(e)
                if e.to_string().contains("duplicate field")
                    || e.to_string().contains("number out of range") =>
            {
                return Snapshot::Unsupported(
                    "ambiguous progress identity/version; preserved read-only".into(),
                );
            }
            Err(e) => {
                return Snapshot::Malformed {
                    reason: e.to_string(),
                    bytes,
                };
            }
        };
        let value: serde_json::Value = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(e) => {
                return Snapshot::Malformed {
                    reason: e.to_string(),
                    bytes,
                };
            }
        };
        if value.get("version").is_some()
            && envelope
                .version
                .as_ref()
                .and_then(serde_json::Value::as_u64)
                != Some(1)
        {
            return Snapshot::Unsupported(
                "unsupported progress version; preserved read-only".into(),
            );
        }
        for (field, observed, expected) in [
            ("game_id", envelope.game_id.as_ref(), key.game()),
            ("profile", envelope.profile.as_ref(), PROFILE.to_owned()),
            (
                "challenge",
                envelope.challenge.as_ref(),
                hex(&key.challenge),
            ),
        ] {
            if value.get(field).is_some()
                && observed.and_then(serde_json::Value::as_str) != Some(expected.as_str())
            {
                return Snapshot::Unsupported(
                    "progress identity mismatch; preserved read-only".into(),
                );
            }
        }
        match serde_json::from_slice::<ProgressV1>(&bytes) {
            Ok(v) if v.version == 1 => {
                if v.game_id != key.game()
                    || v.profile != PROFILE
                    || v.challenge != hex(&key.challenge)
                {
                    Snapshot::Unsupported("progress identity mismatch; preserved read-only".into())
                } else if v.best > key.goal {
                    Snapshot::Malformed {
                        reason: "best exceeds goal".into(),
                        bytes,
                    }
                } else {
                    Snapshot::Valid {
                        best: v.best,
                        bytes,
                    }
                }
            }
            Ok(_) => Snapshot::Malformed {
                reason: "unsupported legacy progress version".into(),
                bytes,
            },
            Err(e) => Snapshot::Malformed {
                reason: e.to_string(),
                bytes,
            },
        }
    }
    fn error(error: impl std::fmt::Display) -> String {
        error.to_string()
    }

    /// Every component is opened relative to its verified parent. Missing reads
    /// return None without creating even a lock file. mkdir is commit-only.
    fn open_directory(paths: &ProgressPaths, create: bool) -> Result<Option<File>, String> {
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
                        return Err("progress parent directory is read-only".into());
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
                        "unsafe or unavailable progress directory: {failure}"
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
                    "progress ancestor must be trusted and not writable by other users".into(),
                );
            }
        }
        let metadata = directory.metadata().map_err(error)?;
        if metadata.uid() != rustix::process::geteuid().as_raw() || metadata.mode() & 0o022 != 0 {
            return Err(
                "progress directory must be owned by this user and not writable by other users"
                    .into(),
            );
        }
        if create && metadata.mode() & 0o200 == 0 {
            return Err("progress directory is read-only".into());
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

    fn read_snapshot(directory: &File, name: &str, key: &ProgressKey) -> Result<Snapshot, String> {
        let Some(before) = stat_regular(directory, name)? else {
            return Ok(Snapshot::Missing);
        };
        if before.st_size < 0 || before.st_size as u64 > MAX_PROGRESS_BYTES as u64 {
            return Ok(Snapshot::Unsupported(format!(
                "{name} exceeds {MAX_PROGRESS_BYTES} bytes; preserved read-only"
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
            .take((MAX_PROGRESS_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(error)?;
        if bytes.len() > MAX_PROGRESS_BYTES {
            return Ok(Snapshot::Unsupported(format!(
                "{name} exceeds {MAX_PROGRESS_BYTES} bytes; preserved read-only"
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

    fn loaded(primary: &Snapshot, backup: &Snapshot) -> Loaded {
        let best = match primary {
            Snapshot::Valid { best, .. } => *best,
            _ => 0,
        };
        for snapshot in [primary, backup] {
            if let Snapshot::Unsupported(reason) = snapshot {
                return Loaded::readonly(best, reason.clone());
            }
        }
        if matches!(primary, Snapshot::Valid { .. }) {
            return Loaded {
                best,
                writable: true,
                notice: None,
            };
        }
        if let Snapshot::Valid { best, .. } = backup {
            return Loaded {
                best: *best,
                writable: true,
                notice: Some(
                    "Primary progress missing or corrupt; recovered matching backup".into(),
                ),
            };
        }
        if matches!((primary, backup), (Snapshot::Missing, Snapshot::Missing)) {
            return Loaded {
                best: 0,
                writable: true,
                notice: None,
            };
        }
        Loaded::readonly(
            0,
            "no valid progress copy; existing bytes preserved read-only",
        )
    }
    pub(super) fn load(paths: &ProgressPaths, key: &ProgressKey) -> Loaded {
        let directory = match open_directory(paths, false) {
            Ok(Some(d)) => d,
            Ok(None) => return loaded(&Snapshot::Missing, &Snapshot::Missing),
            Err(e) => return Loaded::readonly(0, e),
        };
        let primary = match read_snapshot(&directory, PRIMARY_NAME, key) {
            Ok(v) => v,
            Err(e) => return Loaded::readonly(0, e),
        };
        let backup = match read_snapshot(&directory, BACKUP_NAME, key) {
            Ok(v) => v,
            Err(e) => return Loaded::readonly(loaded(&primary, &Snapshot::Missing).best, e),
        };
        let mut result = loaded(&primary, &backup);
        for name in [PRIMARY_NAME, BACKUP_NAME, LOCK_NAME] {
            let failure = match stat_regular(&directory, name) {
                Ok(Some(s)) if s.st_mode & 0o200 == 0 => Some(format!("{name} is read-only")),
                Err(e) => Some(e),
                _ => None,
            };
            if let Some(e) = failure {
                result = Loaded::readonly(result.best, e);
            }
        }
        if directory.metadata().is_ok_and(|m| m.mode() & 0o200 == 0) {
            result = Loaded::readonly(result.best, "progress directory is read-only");
        }
        result
    }
    fn lock(directory: &File) -> Result<File, String> {
        let old = stat_regular(directory, LOCK_NAME)?;
        if old.as_ref().is_some_and(|stat| stat.st_mode & 0o200 == 0) {
            return Err("progress lock is read-only".into());
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
            return Err("progress lock disappeared".into());
        };
        if !same_file(&named, &fs::fstat(&file).map_err(error)?) {
            return Err("progress lock changed while opening".into());
        }
        fs::flock(&file, FlockOperation::NonBlockingLockExclusive).map_err(|failure| {
            format!("progress are locked or locking is unavailable: {failure}")
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
                let name = format!(".progress-stage-{}-{counter}", std::process::id());
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
            Err("could not allocate a unique progress stage".into())
        }
        fn publish(&self, name: &str) -> Result<(), String> {
            fs::renameat(self.directory, self.name.as_str(), self.directory, name).map_err(error)
        }
    }

    pub(super) fn commit(
        paths: &ProgressPaths,
        key: &ProgressKey,
        candidate: u32,
        mut checkpoint: impl FnMut(&str) -> Result<(), String>,
    ) -> Result<(u32, CommitOutcome), String> {
        let mut published = false;
        let mut committed_best = 0;
        let result = (|| -> Result<(), String> {
            let directory = open_directory(paths, true)?.ok_or("progress directory missing")?;
            let _lock = lock(&directory)?;
            for name in [PRIMARY_NAME, BACKUP_NAME] {
                if stat_regular(&directory, name)?.is_some_and(|stat| stat.st_mode & 0o200 == 0) {
                    return Err(format!("{name} is read-only"));
                }
            }
            let primary = read_snapshot(&directory, PRIMARY_NAME, key)?;
            let backup = read_snapshot(&directory, BACKUP_NAME, key)?;
            let primary_identity = stat_regular(&directory, PRIMARY_NAME)?;
            let backup_identity = stat_regular(&directory, BACKUP_NAME)?;
            let state = loaded(&primary, &backup);
            if !state.writable {
                return Err(state
                    .notice
                    .unwrap_or_else(|| "progress are read-only".into()));
            }
            committed_best = candidate.max(state.best);
            let mut bytes =
                serde_json::to_vec(&ProgressV1::new(key, committed_best)).map_err(error)?;
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
            primary_stage.file.write_all(&bytes).map_err(error)?;
            checkpoint("primary-sync")?;
            primary_stage.file.sync_all().map_err(error)?;
            let backup_stage = if let Some(backup_bytes) = backup_bytes {
                let mut stage = Stage::create(&directory)?;
                checkpoint("backup-write")?;
                stage.file.write_all(backup_bytes).map_err(error)?;
                checkpoint("backup-sync")?;
                stage.file.sync_all().map_err(error)?;
                Some(stage)
            } else {
                None
            };
            checkpoint("before-publication")?;
            // Recheck all public names immediately before replacing anything.
            // fd-relative operations cannot follow a swapped ancestor symlink.
            let reopened = open_directory(paths, false)?
                .ok_or("progress directory disappeared before publication")?;
            if !same_file(
                &fs::fstat(&directory).map_err(error)?,
                &fs::fstat(&reopened).map_err(error)?,
            ) {
                return Err("progress directory changed before publication".into());
            }
            // A non-cooperating writer may ignore our lock while staging.
            // Re-read BOTH byte snapshots, including malformed bytes, so a
            // newly arrived future document is never silently downgraded.
            for (name, original, identity) in [
                (PRIMARY_NAME, &primary, &primary_identity),
                (BACKUP_NAME, &backup, &backup_identity),
            ] {
                if read_snapshot(&directory, name, key)? != *original
                    || !same_identity(identity.as_ref(), stat_regular(&directory, name)?.as_ref())
                {
                    return Err(format!(
                        "{name} changed during staging; progress were not published"
                    ));
                }
            }
            let named_lock = stat_regular(&directory, LOCK_NAME)?
                .ok_or("progress lock disappeared before publication")?;
            if !same_file(&named_lock, &fs::fstat(&_lock).map_err(error)?) {
                return Err("progress lock changed before publication".into());
            }
            checkpoint("primary-rename")?;
            primary_stage.publish(PRIMARY_NAME)?;
            published = true;
            checkpoint("after-publication")?;
            if let Some(stage) = backup_stage {
                checkpoint("backup-rename")?;
                stage.publish(BACKUP_NAME)?;
            }
            checkpoint("directory-sync")?;
            directory.sync_all().map_err(error)?;
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
}

#[cfg(all(test, target_os = "linux"))]
#[path = "game_progress_store_tests.rs"]
mod tests;
