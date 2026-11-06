//! Explicit view-side audio preferences. Startup never creates files.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Preferences {
    schema: u32,
    profile: String,
    pub(crate) gain_percent: u8,
    pub(crate) muted: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            schema: 1,
            profile: "stormglass-audio-v1".into(),
            gain_percent: 100,
            muted: true,
        }
    }
}
impl Preferences {
    pub(crate) fn new(gain_percent: u8, muted: bool) -> Result<Self, String> {
        let value = Self {
            gain_percent,
            muted,
            ..Self::default()
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), String> {
        if self.schema != 1 || self.profile != "stormglass-audio-v1" || self.gain_percent > 100 {
            return Err("unsupported or invalid audio settings".into());
        }
        Ok(())
    }
    fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 1024 {
            return Err("audio settings exceed 1024 bytes".into());
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| "malformed audio settings")?;
        value.validate()?;
        Ok(value)
    }
}

// Closed, host-selected contracts. Neither filenames nor validation are supplied by user data.
#[cfg(any(target_os = "linux", windows))]
#[derive(Clone, Copy)]
pub(crate) enum StorageProfile {
    Audio,
    #[cfg(feature = "coop-defense-controls")]
    Controls,
}
#[cfg(any(target_os = "linux", windows))]
impl StorageProfile {
    fn primary(self) -> &'static str {
        match self {
            Self::Audio => "audio-settings.json",
            #[cfg(feature = "coop-defense-controls")]
            Self::Controls => "controls-settings.json",
        }
    }
    fn lock(self) -> &'static str {
        match self {
            Self::Audio => "audio-settings.lock",
            #[cfg(feature = "coop-defense-controls")]
            Self::Controls => "controls-settings.lock",
        }
    }
    fn prefix(self) -> &'static str {
        match self {
            Self::Audio => ".audio-settings-pending-",
            #[cfg(feature = "coop-defense-controls")]
            Self::Controls => ".controls-settings-pending-",
        }
    }
    fn limit(self) -> u32 {
        match self {
            Self::Audio => 1024,
            #[cfg(feature = "coop-defense-controls")]
            Self::Controls => 8192,
        }
    }
    fn validate(self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > self.limit() as usize {
            return Err("settings exceed profile byte budget".into());
        }
        match self {
            Self::Audio => Preferences::decode(bytes).map(|_| ()),
            #[cfg(feature = "coop-defense-controls")]
            Self::Controls => crate::defense_controls::Document::decode(bytes).map(|_| ()),
        }
    }
}

#[derive(Default)]
#[cfg(any(target_os = "linux", windows))]
pub(crate) struct Panel {
    pub(crate) configured: bool,
    pub(crate) writable: bool,
    pub(crate) save_requested: bool,
    pub(crate) status: String,
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use rustix::fs::{self, AtFlags, FileType, FlockOperation, Mode, OFlags};
    use std::{
        fs::File,
        io::{Read, Write},
        os::fd::OwnedFd,
        os::unix::ffi::OsStrExt,
        path::{Component, Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    #[cfg(test)]
    const PRIMARY: &str = "audio-settings.json";
    #[cfg(test)]
    const LOCK: &str = "audio-settings.lock";
    static NEXT: AtomicU64 = AtomicU64::new(0);

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Identity {
        device: u64,
        inode: u64,
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Snapshot {
        identity: Identity,
        bytes: Vec<u8>,
    }
    pub(crate) enum Outcome {
        Saved,
        NotSaved(String),
        PublishedUncertain(String),
    }
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Stage {
        Prepared,
        Published,
    }

    pub(crate) struct Store {
        directory: OwnedFd,
        baseline: Option<Snapshot>,
        writable: bool,
        profile: StorageProfile,
    }
    pub(crate) struct Loaded {
        pub(crate) store: Store,
        pub(crate) preferences: Preferences,
        pub(crate) status: String,
    }
    fn error(e: impl std::fmt::Display) -> String {
        format!("audio settings: {e}")
    }
    fn identity(stat: &fs::Stat) -> Identity {
        Identity {
            device: stat.st_dev,
            inode: stat.st_ino,
        }
    }
    fn directory_ok(fd: &OwnedFd) -> Result<(), String> {
        let stat = fs::fstat(fd).map_err(error)?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::Directory
            || stat.st_uid != rustix::process::geteuid().as_raw()
            || stat.st_mode & 0o077 != 0
        {
            return Err(error(
                "choose an existing user-owned private directory (0700)",
            ));
        }
        Ok(())
    }
    fn file_ok(file: &File, limit: u64) -> Result<fs::Stat, String> {
        let stat = fs::fstat(file).map_err(error)?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
            || stat.st_uid != rustix::process::geteuid().as_raw()
            || stat.st_mode & 0o077 != 0
            || stat.st_nlink != 1
            || stat.st_size < 0
            || stat.st_size as u64 > limit
        {
            return Err(error(
                "expected a private, owned, single-link bounded regular file",
            ));
        }
        Ok(stat)
    }
    fn link_is(directory: &OwnedFd, name: &str, expected: Identity) -> Result<(), String> {
        let stat = fs::statat(directory, name, AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
        if identity(&stat) != expected
            || FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
        {
            return Err(error("file identity changed"));
        }
        Ok(())
    }
    impl Store {
        pub(crate) fn open(path: &Path, forbidden_roots: &[PathBuf]) -> Result<Loaded, String> {
            let (store, bytes) = Self::open_profile(path, forbidden_roots, StorageProfile::Audio)?;
            let (preferences, status) = match bytes {
                Some(bytes) => (
                    Preferences::decode(&bytes)?,
                    "Saved audio settings restored".into(),
                ),
                None if store.writable => (
                    Preferences::default(),
                    "Session only until SAVE AUDIO".into(),
                ),
                None => (
                    Preferences::default(),
                    "Audio settings unreadable or unsupported; muted defaults, Save disabled"
                        .into(),
                ),
            };
            Ok(Loaded {
                store,
                preferences,
                status,
            })
        }
        pub(crate) fn open_profile(
            path: &Path,
            forbidden_roots: &[PathBuf],
            profile: StorageProfile,
        ) -> Result<(Self, Option<Vec<u8>>), String> {
            let bytes = path.as_os_str().as_bytes();
            if !path.is_absolute()
                || path.parent().is_none()
                || bytes.len() > 4096
                || path.components().count() > 128
                || bytes
                    .split(|b| *b == b'/')
                    .any(|s| s == b"." || s == b".." || s.contains(&0))
            {
                return Err(error("use an absolute non-root directory without . or .."));
            }
            for root in forbidden_roots {
                let root = std::fs::canonicalize(root).map_err(error)?;
                if path.starts_with(root) {
                    return Err(error(
                        "directory is inside a source, audio bank or executable root",
                    ));
                }
            }
            let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
            let mut directory = fs::open("/", flags, Mode::empty()).map_err(error)?;
            for part in path.components() {
                if let Component::Normal(name) = part {
                    directory =
                        fs::openat(&directory, name, flags, Mode::empty()).map_err(error)?;
                }
            }
            directory_ok(&directory)?;
            let mut store = Self {
                directory,
                baseline: None,
                writable: true,
                profile,
            };
            match store.read() {
                Ok(Some(snapshot)) => {
                    let bytes = snapshot.bytes.clone();
                    store.baseline = Some(snapshot);
                    Ok((store, Some(bytes)))
                }
                Ok(None) => Ok((store, None)),
                Err(_) => {
                    store.writable = false;
                    Ok((store, None))
                }
            }
        }
        pub(crate) fn writable(&self) -> bool {
            self.writable
        }
        fn read(&self) -> Result<Option<Snapshot>, String> {
            directory_ok(&self.directory)?;
            let fd = match fs::openat(
                &self.directory,
                self.profile.primary(),
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            ) {
                Ok(fd) => fd,
                Err(rustix::io::Errno::NOENT) => return Ok(None),
                Err(e) => return Err(error(e)),
            };
            let file: File = fd.into();
            let stat = file_ok(&file, u64::from(self.profile.limit()))?;
            let mut bytes = Vec::new();
            (&file)
                .take(u64::from(self.profile.limit()) + 1)
                .read_to_end(&mut bytes)
                .map_err(error)?;
            self.profile.validate(&bytes)?;
            let after = file_ok(&file, u64::from(self.profile.limit()))?;
            if stat.st_size != after.st_size
                || stat.st_mtime != after.st_mtime
                || stat.st_mtime_nsec != after.st_mtime_nsec
            {
                return Err(error("settings changed while reading"));
            }
            Ok(Some(Snapshot {
                identity: identity(&stat),
                bytes,
            }))
        }
        pub(crate) fn save(&mut self, preferences: &Preferences) -> Outcome {
            self.save_with(preferences, |_| Ok(()))
        }
        fn save_with(
            &mut self,
            preferences: &Preferences,
            mut hook: impl FnMut(Stage) -> Result<(), String>,
        ) -> Outcome {
            let bytes = match serde_json::to_vec(preferences) {
                Ok(bytes) => bytes,
                Err(e) => return Outcome::NotSaved(error(e)),
            };
            self.save_bytes_with(bytes, &mut hook)
        }
        #[cfg(feature = "coop-defense-controls")]
        pub(crate) fn save_bytes(&mut self, bytes: Vec<u8>) -> Outcome {
            self.save_bytes_with(bytes, |_| Ok(()))
        }
        #[cfg(all(test, feature = "coop-defense-controls"))]
        pub(crate) fn test_fail_publication(&mut self, bytes: Vec<u8>, after: bool) -> Outcome {
            self.save_bytes_with(bytes, |stage| {
                if (stage == Stage::Published) == after {
                    Err("injected controls publication failure".into())
                } else {
                    Ok(())
                }
            })
        }
        fn save_bytes_with(
            &mut self,
            bytes: Vec<u8>,
            mut hook: impl FnMut(Stage) -> Result<(), String>,
        ) -> Outcome {
            match self.publish(bytes, &mut hook) {
                Ok(()) => Outcome::Saved,
                Err((true, e)) => Outcome::PublishedUncertain(e),
                Err((false, e)) => Outcome::NotSaved(e),
            }
        }
        fn publish(
            &mut self,
            bytes: Vec<u8>,
            hook: &mut impl FnMut(Stage) -> Result<(), String>,
        ) -> Result<(), (bool, String)> {
            let before = |e| (false, e);
            if !self.writable {
                return Err(before(error("Save disabled for this session")));
            }
            self.profile.validate(&bytes).map_err(before)?;
            directory_ok(&self.directory).map_err(before)?;
            let lock: File = fs::openat(
                &self.directory,
                self.profile.lock(),
                OFlags::RDWR
                    | OFlags::CREATE
                    | OFlags::NOFOLLOW
                    | OFlags::NONBLOCK
                    | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(error)
            .map_err(before)?
            .into();
            let lock_id = identity(&file_ok(&lock, 0).map_err(before)?);
            fs::flock(&lock, FlockOperation::NonBlockingLockExclusive)
                .map_err(error)
                .map_err(before)?;
            link_is(&self.directory, self.profile.lock(), lock_id).map_err(before)?;
            if self.read().map_err(before)? != self.baseline {
                return Err(before(error("settings changed on disk; restart to reload")));
            }
            let mut created = None;
            for _ in 0..32 {
                let name = format!(
                    "{}{}-{}",
                    self.profile.prefix(),
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                );
                match fs::openat(
                    &self.directory,
                    name.as_str(),
                    OFlags::WRONLY
                        | OFlags::CREATE
                        | OFlags::EXCL
                        | OFlags::NOFOLLOW
                        | OFlags::CLOEXEC,
                    Mode::RUSR | Mode::WUSR,
                ) {
                    Ok(fd) => {
                        created = Some((name, File::from(fd)));
                        break;
                    }
                    Err(rustix::io::Errno::EXIST) => {}
                    Err(e) => return Err(before(error(e))),
                }
            }
            let (name, mut stage) =
                created.ok_or_else(|| before(error("staging name budget exhausted")))?;
            let stage_id = identity(&fs::fstat(&stage).map_err(error).map_err(before)?);
            let result = (|| {
                file_ok(&stage, u64::from(self.profile.limit())).map_err(before)?;
                stage.write_all(&bytes).map_err(error).map_err(before)?;
                stage.sync_all().map_err(error).map_err(before)?;
                hook(Stage::Prepared).map_err(before)?;
                file_ok(&stage, u64::from(self.profile.limit())).map_err(before)?;
                directory_ok(&self.directory).map_err(before)?;
                link_is(&self.directory, self.profile.lock(), lock_id).map_err(before)?;
                link_is(&self.directory, &name, stage_id).map_err(before)?;
                if self.read().map_err(before)? != self.baseline {
                    return Err(before(error("settings changed before publication")));
                }
                fs::renameat(
                    &self.directory,
                    name.as_str(),
                    &self.directory,
                    self.profile.primary(),
                )
                .map_err(error)
                .map_err(before)?;
                self.baseline = Some(Snapshot {
                    identity: stage_id,
                    bytes,
                });
                hook(Stage::Published).map_err(|e| (true, e))?;
                fs::fsync(&self.directory)
                    .map_err(error)
                    .map_err(|e| (true, e))?;
                Ok(())
            })();
            // Remove only our own unpublished inode; never a substituted file.
            if link_is(&self.directory, &name, stage_id).is_ok() {
                let _ = fs::unlinkat(&self.directory, name.as_str(), AtFlags::empty());
            }
            result
        }
    }

    #[cfg(test)]
    mod tests {
        include!("defense_audio_settings_store_tests.rs");
    }
}

#[cfg(target_os = "linux")]
pub(crate) use linux::{Loaded, Outcome, Store};

#[cfg(windows)]
#[path = "defense_audio_settings_windows.rs"]
mod windows;
#[cfg(all(test, windows))]
pub(crate) use windows::tests::private_directory as windows_test_directory;
#[cfg(all(test, windows, feature = "coop-defense-controls"))]
pub(crate) use windows::tests::write_private as windows_test_write;
#[cfg(windows)]
pub(crate) use windows::{Loaded, Outcome, Store};

#[cfg(any(target_os = "linux", windows))]
pub(crate) fn install(ui: &mut crate::defense_ui::DefenseUi, loaded: Loaded) {
    ui.audio.gain_percent = loaded.preferences.gain_percent;
    ui.audio.muted = loaded.preferences.muted;
    ui.audio_settings = Panel {
        configured: true,
        writable: loaded.store.writable(),
        save_requested: false,
        status: loaded.status,
    };
    ui.audio_settings_store = Some(loaded.store);
}

#[cfg(any(target_os = "linux", windows))]
pub(crate) fn process_save(ui: &mut crate::defense_ui::DefenseUi) {
    if !std::mem::take(&mut ui.audio_settings.save_requested) {
        return;
    }
    let Some(store) = ui.audio_settings_store.as_mut() else {
        return;
    };
    let outcome = match Preferences::new(ui.audio.gain_percent, ui.audio.muted) {
        Ok(preferences) => store.save(&preferences),
        Err(e) => Outcome::NotSaved(e),
    };
    ui.audio_settings.status = match outcome {
        Outcome::Saved => "Audio settings saved".into(),
        Outcome::NotSaved(e) => format!("Not saved: {e}"),
        Outcome::PublishedUncertain(e) => format!("Published; durability uncertain: {e}"),
        #[cfg(windows)]
        Outcome::Uncertain(e) => format!("Save outcome uncertain; restart to inspect: {e}"),
    }
    .chars()
    .take(240)
    .map(|c| if c.is_control() { ' ' } else { c })
    .collect();
    ui.audio_settings.writable = store.writable();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_profile_requires_bounded_gain_and_safe_defaults() {
        assert_eq!(
            (
                Preferences::default().gain_percent,
                Preferences::default().muted
            ),
            (100, true)
        );
        for value in [0, 50, 100] {
            Preferences::new(value, false).unwrap();
        }
        assert!(Preferences::new(101, false).is_err());
        for json in [
            r#"{"schema":2,"profile":"stormglass-audio-v1","gain_percent":40,"muted":false}"#,
            r#"{"schema":1,"profile":"arena-controls-v1","gain_percent":40,"muted":false}"#,
            r#"{"schema":1,"profile":"stormglass-audio-v1","gain_percent":101,"muted":false}"#,
            r#"{"schema":1,"profile":"stormglass-audio-v1","gain_percent":40,"muted":false,"extra":0}"#,
            r#"{"schema":1,"profile":"stormglass-audio-v1","gain_percent":40,"gain_percent":50,"muted":false}"#,
        ] {
            assert!(Preferences::decode(json.as_bytes()).is_err());
        }
        assert!(Preferences::decode(&[b' '; 1025]).is_err());
    }
}
