//! Bounded, view-side preferences for the shared Arena controls profile.
//!
//! Schema 1 is the first format. There is deliberately no legacy migration and
//! no project/build identity here. Discovery and loading never create files.
//! The Linux writer uses a stable nonblocking advisory lock and fd-relative,
//! no-follow operations. Cooperating writers serialize commits; files owned by
//! another user, symlinks, hard links and special files are refused.
//!
//! Publication has one commit point: replacing `settings.json`. The old valid
//! primary is staged as a backup before that point, but the backup name is only
//! replaced AFTER it. Thus every prepublication failure preserves both names.
//! A crash after publication leaves a valid new primary and either the existing
//! valid backup or its staged replacement. A postpublication failure must never
//! be treated as permission to blindly retry or retain the old active settings.
use orr_input::{ActionMap, Button, Key};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    path::{Component, Path, PathBuf},
};

pub const PROFILE: &str = "arena-controls-v1";
pub const SCHEMA: u32 = 1;
pub const MAX_SETTINGS_BYTES: usize = 4096;
pub const PRIMARY_NAME: &str = "settings.json";
pub const BACKUP_NAME: &str = "settings.json.bak";
pub const LOCK_NAME: &str = "settings.lock";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FireBinding {
    #[default]
    Space,
    LeftMouse,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerSettingsV1 {
    pub schema: u32,
    pub profile: String,
    pub fire_binding: FireBinding,
}

impl Default for PlayerSettingsV1 {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            profile: PROFILE.into(),
            fire_binding: FireBinding::Space,
        }
    }
}

impl PlayerSettingsV1 {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != SCHEMA {
            return Err(format!(
                "unsupported player settings schema {} (expected {SCHEMA})",
                self.schema
            ));
        }
        if self.profile != PROFILE {
            return Err("unsupported player settings profile".into());
        }
        Ok(())
    }

    /// Build and validate the COMPLETE map, not a patch to an arbitrary map.
    pub fn action_map(&self) -> Result<ActionMap, String> {
        self.validate()?;
        let mut map = crate::arena_input::default_map();
        let fire = map
            .actions
            .iter_mut()
            .find(|action| action.name == "fire")
            .ok_or("default Arena map has no fire action")?;
        fire.bindings = vec![match self.fire_binding {
            FireBinding::Space => Button::Keyboard { key: Key::Space },
            FireBinding::LeftMouse => Button::Mouse { button: 0 },
        }];
        crate::arena_input::validate_map(&map)?;
        Ok(map)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsPaths {
    directory: PathBuf,
}

impl SettingsPaths {
    /// An explicit external directory; never resolved relative to cwd.
    /// Callers which know project/export roots must exclude those roots.
    pub fn from_directory(directory: impl Into<PathBuf>) -> Result<Self, String> {
        let directory = directory.into();
        validate_directory_path(&directory)?;
        Ok(Self { directory })
    }

    /// Absolute XDG_CONFIG_HOME first, then absolute HOME/.config. No fallback
    /// to cwd, project, export, executable or `.orr` directories is permitted.
    pub fn from_environment() -> Result<Self, String> {
        Self::from_environment_values(
            std::env::var_os("XDG_CONFIG_HOME").as_deref(),
            std::env::var_os("HOME").as_deref(),
        )
    }

    /// Pure resolver also used by sandboxed tests without changing process env.
    pub fn from_environment_values(
        xdg: Option<&OsStr>,
        home: Option<&OsStr>,
    ) -> Result<Self, String> {
        let base = if let Some(path) = xdg.map(Path::new).filter(|path| path.is_absolute()) {
            path.to_path_buf()
        } else if let Some(path) = home.map(Path::new).filter(|path| path.is_absolute()) {
            path.join(".config")
        } else {
            return Err("player settings need an absolute XDG_CONFIG_HOME, absolute HOME, or explicit settings directory".into());
        };
        Self::from_directory(base.join("orrery").join(PROFILE))
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn primary(&self) -> PathBuf {
        self.directory.join(PRIMARY_NAME)
    }
    pub fn backup(&self) -> PathBuf {
        self.directory.join(BACKUP_NAME)
    }
    pub fn lock(&self) -> PathBuf {
        self.directory.join(LOCK_NAME)
    }
}

fn validate_directory_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || path.parent().is_none() {
        return Err("settings directory must be an absolute non-root path".into());
    }
    if path.components().count() > 128 || path.as_os_str().as_encoded_bytes().len() > 4096 {
        return Err("settings directory path exceeds bounds".into());
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
        return Err("settings directory contains an unsafe path component".into());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadState {
    Missing,
    Primary,
    RecoveredBackup,
    /// Both usable copies are absent or malformed. Defaults require Apply to save.
    Defaults,
    /// Unsupported data or unsafe/unavailable storage. Never writable this session.
    ReadOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedSettings {
    pub settings: PlayerSettingsV1,
    pub state: LoadState,
    pub notice: Option<String>,
}

impl LoadedSettings {
    pub fn writable(&self) -> bool {
        self.state != LoadState::ReadOnly
    }
    pub fn read_only(notice: impl Into<String>) -> Self {
        Self {
            settings: PlayerSettingsV1::default(),
            state: LoadState::ReadOnly,
            notice: Some(notice.into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitOutcome {
    Published,
    /// Neither primary nor backup was replaced. Active settings must not change.
    NotPublished(String),
    /// Primary WAS replaced. Install that value, report uncertainty, do not retry.
    PublishedDurabilityUncertain(String),
}

#[derive(Clone, Debug)]
pub struct SettingsStore {
    paths: SettingsPaths,
}

impl SettingsStore {
    pub fn new(paths: SettingsPaths) -> Self {
        Self { paths }
    }
    pub fn paths(&self) -> &SettingsPaths {
        &self.paths
    }

    pub fn load(&self) -> LoadedSettings {
        #[cfg(target_os = "linux")]
        {
            linux::load(&self.paths)
        }
        #[cfg(not(target_os = "linux"))]
        {
            LoadedSettings::read_only(
                "persistent player settings are currently supported only on Linux",
            )
        }
    }

    pub fn commit(&self, settings: &PlayerSettingsV1) -> CommitOutcome {
        if let Err(error) = settings.action_map() {
            return CommitOutcome::NotPublished(error);
        }
        #[cfg(target_os = "linux")]
        {
            linux::commit(&self.paths, settings, |_| Ok(()))
        }
        #[cfg(not(target_os = "linux"))]
        {
            CommitOutcome::NotPublished(
                "persistent player settings are currently supported only on Linux".into(),
            )
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
        Valid {
            settings: PlayerSettingsV1,
            bytes: Vec<u8>,
        },
        Unsupported(String),
        Malformed {
            reason: String,
            bytes: Vec<u8>,
        },
    }

    // Probe only discriminators before strict v1 decoding. Future schemas may
    // add fields or use new binding representations. They are preserved even
    // when their other fields are not understood or a valid v1 backup exists.
    #[derive(Deserialize)]
    struct Envelope {
        schema: Option<serde_json::Value>,
        profile: Option<serde_json::Value>,
    }

    fn decode(bytes: Vec<u8>) -> Snapshot {
        let envelope: Envelope = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(error) => {
                return Snapshot::Malformed {
                    reason: format!("invalid settings JSON: {error}"),
                    bytes,
                };
            }
        };
        // Do not narrow the discriminator to u32: a newer writer may have a
        // larger integer version. serde_json represents integers beyond u64 as
        // f64; those are also conservatively protected, never treated as v1.
        let unsupported_schema = envelope.schema.as_ref().is_some_and(|schema| {
            schema
                .as_u64()
                .is_some_and(|schema| schema != u64::from(SCHEMA))
                || schema
                    .as_f64()
                    .is_some_and(|schema| schema > f64::from(u32::MAX))
        });
        if unsupported_schema
            || envelope
                .profile
                .as_ref()
                .and_then(serde_json::Value::as_str)
                .is_some_and(|profile| profile != PROFILE)
        {
            return Snapshot::Unsupported(
                "unsupported settings schema or profile; existing files preserved read-only".into(),
            );
        }
        match serde_json::from_slice::<PlayerSettingsV1>(&bytes) {
            Ok(settings) => match settings.action_map() {
                Ok(_) => Snapshot::Valid { settings, bytes },
                Err(reason) => Snapshot::Malformed { reason, bytes },
            },
            Err(error) => Snapshot::Malformed {
                reason: format!("invalid settings: {error}"),
                bytes,
            },
        }
    }

    fn error(error: impl std::fmt::Display) -> String {
        error.to_string()
    }

    /// Every component is opened relative to its verified parent. Missing reads
    /// return None without creating even a lock file. mkdir is commit-only.
    fn open_directory(paths: &SettingsPaths, create: bool) -> Result<Option<File>, String> {
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
                        return Err("settings parent directory is read-only".into());
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
                        "unsafe or unavailable settings directory: {failure}"
                    ));
                }
            };
            directory = File::from(next);
        }
        let metadata = directory.metadata().map_err(error)?;
        if metadata.uid() != rustix::process::geteuid().as_raw() || metadata.mode() & 0o022 != 0 {
            return Err(
                "settings directory must be owned by this user and not writable by other users"
                    .into(),
            );
        }
        if create && metadata.mode() & 0o200 == 0 {
            return Err("settings directory is read-only".into());
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
        {
            return Err(format!(
                "{name} must be a user-owned regular file without links"
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

    fn read_snapshot(directory: &File, name: &str) -> Result<Snapshot, String> {
        let Some(before) = stat_regular(directory, name)? else {
            return Ok(Snapshot::Missing);
        };
        if before.st_size < 0 || before.st_size as u64 > MAX_SETTINGS_BYTES as u64 {
            return Ok(Snapshot::Unsupported(format!(
                "{name} exceeds {MAX_SETTINGS_BYTES} bytes; preserved read-only"
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
        {
            return Err(format!("{name} changed while opening"));
        }
        let mut bytes = Vec::new();
        (&file)
            .take((MAX_SETTINGS_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(error)?;
        if bytes.len() > MAX_SETTINGS_BYTES {
            return Ok(Snapshot::Unsupported(format!(
                "{name} exceeds {MAX_SETTINGS_BYTES} bytes; preserved read-only"
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
        Ok(decode(bytes))
    }

    fn loaded(primary: &Snapshot, backup: &Snapshot) -> LoadedSettings {
        let current = match primary {
            Snapshot::Valid { settings, .. } => settings.clone(),
            _ => PlayerSettingsV1::default(),
        };
        if let Snapshot::Unsupported(reason) = primary {
            return LoadedSettings::read_only(reason.clone());
        }
        if let Snapshot::Unsupported(reason) = backup {
            return LoadedSettings {
                settings: current,
                state: LoadState::ReadOnly,
                notice: Some(format!("Backup: {reason}")),
            };
        }
        if matches!(primary, Snapshot::Valid { .. }) {
            return LoadedSettings {
                settings: current,
                state: LoadState::Primary,
                notice: None,
            };
        }
        if let Snapshot::Valid { settings, .. } = backup {
            return LoadedSettings { settings: settings.clone(), state: LoadState::RecoveredBackup,
                notice: Some("Primary settings are missing or invalid; recovered the valid backup. Apply to save this choice.".into()) };
        }
        if matches!((primary, backup), (Snapshot::Missing, Snapshot::Missing)) {
            return LoadedSettings {
                settings: current,
                state: LoadState::Missing,
                notice: None,
            };
        }
        let reason = match primary {
            Snapshot::Malformed { reason, .. } => reason.as_str(),
            _ => "backup settings are invalid",
        };
        LoadedSettings {
            settings: current,
            state: LoadState::Defaults,
            notice: Some(format!(
                "Using defaults because {reason}; existing files remain unchanged until Apply."
            )),
        }
    }

    pub(super) fn load(paths: &SettingsPaths) -> LoadedSettings {
        let directory = match open_directory(paths, false) {
            Ok(Some(directory)) => directory,
            Ok(None) => return loaded(&Snapshot::Missing, &Snapshot::Missing),
            Err(failure) => return LoadedSettings::read_only(failure),
        };
        let primary = match read_snapshot(&directory, PRIMARY_NAME) {
            Ok(primary) => primary,
            Err(failure) => return LoadedSettings::read_only(failure),
        };
        let backup = match read_snapshot(&directory, BACKUP_NAME) {
            Ok(backup) => backup,
            Err(failure) => {
                let mut result = loaded(&primary, &Snapshot::Missing);
                result.state = LoadState::ReadOnly;
                result.notice = Some(failure);
                return result;
            }
        };
        let mut result = loaded(&primary, &backup);
        for name in [PRIMARY_NAME, BACKUP_NAME, LOCK_NAME] {
            match stat_regular(&directory, name) {
                Ok(Some(stat)) if stat.st_mode & 0o200 == 0 => {
                    result.state = LoadState::ReadOnly;
                    result.notice =
                        Some(format!("{name} is read-only; loaded values remain active"));
                }
                Err(failure) => {
                    result.state = LoadState::ReadOnly;
                    result.notice = Some(failure);
                }
                _ => (),
            }
        }
        if directory
            .metadata()
            .is_ok_and(|metadata| metadata.mode() & 0o200 == 0)
        {
            result.state = LoadState::ReadOnly;
            result.notice =
                Some("settings directory is read-only; loaded values remain active".into());
        }
        result
    }

    fn lock(directory: &File) -> Result<File, String> {
        let old = stat_regular(directory, LOCK_NAME)?;
        if old.as_ref().is_some_and(|stat| stat.st_mode & 0o200 == 0) {
            return Err("settings lock is read-only".into());
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
            return Err("settings lock disappeared".into());
        };
        if !same_file(&named, &fs::fstat(&file).map_err(error)?) {
            return Err("settings lock changed while opening".into());
        }
        fs::flock(&file, FlockOperation::NonBlockingLockExclusive).map_err(|failure| {
            format!("settings are locked or locking is unavailable: {failure}")
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
                let name = format!(".settings-stage-{}-{counter}", std::process::id());
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
            Err("could not allocate a unique settings stage".into())
        }
        fn publish(&self, name: &str) -> Result<(), String> {
            fs::renameat(self.directory, self.name.as_str(), self.directory, name).map_err(error)
        }
    }

    pub(super) fn commit(
        paths: &SettingsPaths,
        settings: &PlayerSettingsV1,
        mut checkpoint: impl FnMut(&str) -> Result<(), String>,
    ) -> CommitOutcome {
        let mut published = false;
        let result = (|| -> Result<(), String> {
            settings.action_map()?;
            let mut bytes = serde_json::to_vec_pretty(settings).map_err(error)?;
            bytes.push(b'\n');
            if bytes.len() > MAX_SETTINGS_BYTES {
                return Err("serialized settings exceed limit".into());
            }
            let directory = open_directory(paths, true)?.ok_or("settings directory missing")?;
            let _lock = lock(&directory)?;
            for name in [PRIMARY_NAME, BACKUP_NAME] {
                if stat_regular(&directory, name)?.is_some_and(|stat| stat.st_mode & 0o200 == 0) {
                    return Err(format!("{name} is read-only"));
                }
            }
            let primary = read_snapshot(&directory, PRIMARY_NAME)?;
            let backup = read_snapshot(&directory, BACKUP_NAME)?;
            let primary_identity = stat_regular(&directory, PRIMARY_NAME)?;
            let backup_identity = stat_regular(&directory, BACKUP_NAME)?;
            let state = loaded(&primary, &backup);
            if !state.writable() {
                return Err(state
                    .notice
                    .unwrap_or_else(|| "settings are read-only".into()));
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
                .ok_or("settings directory disappeared before publication")?;
            if !same_file(
                &fs::fstat(&directory).map_err(error)?,
                &fs::fstat(&reopened).map_err(error)?,
            ) {
                return Err("settings directory changed before publication".into());
            }
            // A non-cooperating writer may ignore our lock while staging.
            // Re-read BOTH byte snapshots, including malformed bytes, so a
            // newly arrived future document is never silently downgraded.
            for (name, original, identity) in [
                (PRIMARY_NAME, &primary, &primary_identity),
                (BACKUP_NAME, &backup, &backup_identity),
            ] {
                if read_snapshot(&directory, name)? != *original
                    || !same_identity(identity.as_ref(), stat_regular(&directory, name)?.as_ref())
                {
                    return Err(format!(
                        "{name} changed during staging; settings were not published"
                    ));
                }
            }
            let named_lock = stat_regular(&directory, LOCK_NAME)?
                .ok_or("settings lock disappeared before publication")?;
            if !same_file(&named_lock, &fs::fstat(&_lock).map_err(error)?) {
                return Err("settings lock changed before publication".into());
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
            (_, Ok(())) => CommitOutcome::Published,
            (false, Err(failure)) => CommitOutcome::NotPublished(failure),
            (true, Err(failure)) => CommitOutcome::PublishedDurabilityUncertain(format!(
                "settings were published, but backup/durability confirmation failed: {failure}; do not retry automatically"
            )),
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "player_settings_store_tests.rs"]
mod tests;
