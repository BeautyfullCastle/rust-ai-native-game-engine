//! Windows local-NTFS backend. Ancestor handles prohibit pathname replacement.
use super::{Preferences, StorageProfile};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use windows_sys::Win32::{Foundation::*, Storage::FileSystem::*};
#[path = "defense_audio_settings_windows_security.rs"]
mod security;
use security::{Security, READ_SECURITY};
#[cfg(feature = "coop-defense-controls")]
#[path = "defense_audio_settings_windows_setup.rs"]
mod setup;
#[cfg(feature = "coop-defense-controls")]
pub(crate) use setup::prepare;
#[cfg(test)]
const PRIMARY: &str = "audio-settings.json";
#[cfg(test)]
const LOCK: &str = "audio-settings.lock";
static NEXT: AtomicU64 = AtomicU64::new(0);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity {
    volume: u32,
    high: u32,
    low: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    identity: Identity,
    bytes: Vec<u8>,
}
#[derive(Debug)]
pub(crate) enum Outcome {
    Saved,
    NotSaved(String),
    PublishedUncertain(String),
    Uncertain(String),
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Prepared,
    Published,
}
pub(crate) struct Store {
    path: PathBuf,
    anchors: Vec<File>,
    security: Security,
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
fn info(file: &File) -> Result<BY_HANDLE_FILE_INFORMATION, String> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: valid live file handle and writable output of the exact Win32 type.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(error(security::last()));
    }
    Ok(info)
}
fn identity(info: &BY_HANDLE_FILE_INFORMATION) -> Identity {
    Identity {
        volume: info.dwVolumeSerialNumber,
        high: info.nFileIndexHigh,
        low: info.nFileIndexLow,
    }
}
fn open_directory(path: &Path) -> Result<File, String> {
    // Metadata-only opens do not reserve Windows sharing access; list access does.
    let file = OpenOptions::new()
        .access_mode(FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | READ_SECURITY)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(error)?;
    let metadata = info(&file)?;
    if metadata.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
        || metadata.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(error("directory traversal cannot include a reparse point"));
    }
    Ok(file)
}
fn volume_path(file: &File) -> Result<PathBuf, String> {
    let mut buffer = vec![0u16; 32768];
    // SAFETY: live handle, bounded writable buffer; GUID form does not depend on drive mappings.
    let length = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            VOLUME_NAME_GUID,
        )
    } as usize;
    if length == 0 || length >= buffer.len() {
        return Err(error("could not pin the NTFS volume path"));
    }
    use std::os::windows::ffi::OsStringExt;
    Ok(PathBuf::from(std::ffi::OsString::from_wide(
        &buffer[..length],
    )))
}
fn move_primary(from: &Path, to: &Path) -> Result<(), String> {
    let from = security::wide(from.as_os_str());
    let to = security::wide(to.as_os_str());
    // SAFETY: NUL-terminated paths into one pinned local NTFS directory; no copy fallback.
    if unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(error(security::last()))
    } else {
        Ok(())
    }
}
fn validate_path(path: &Path) -> Result<Vec<PathBuf>, String> {
    let text = path
        .to_str()
        .ok_or_else(|| error("directory path must be Unicode"))?;
    let bytes = text.as_bytes();
    if bytes.len() < 4
        || bytes.len() > 240
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':'
        || bytes[2] != b'\\'
    {
        return Err(error(
            "use a non-root absolute local drive path, e.g. C:\\Private\\Audio",
        ));
    }
    let mut paths = vec![PathBuf::from(&text[..3])];
    let mut current = paths[0].clone();
    for part in text[3..].split('\\') {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        let reserved = matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|n| {
                matches!(
                    n,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        });
        if part.is_empty()
            || part.ends_with(['.', ' '])
            || part
                .chars()
                .any(|c| c.is_control() || "<>:\"/|?*".contains(c))
            || reserved
        {
            return Err(error(
                "directory path contains a reserved or ambiguous component",
            ));
        }
        current.push(part);
        paths.push(current.clone());
    }
    Ok(paths)
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
                "Audio settings unreadable or unsupported; muted defaults, Save disabled".into(),
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
        let paths = validate_path(path)?;
        let root = security::wide(paths[0].as_os_str());
        // SAFETY: root is a NUL-terminated drive root.
        if unsafe { GetDriveTypeW(root.as_ptr()) } != 3 {
            return Err(error("settings require a fixed local NTFS drive"));
        }
        let mut anchors = Vec::new();
        for part in paths {
            anchors.push(open_directory(&part)?);
        }
        let mut filesystem = [0u16; 32];
        // SAFETY: live root handle and correctly sized writable UTF-16 buffer; unused outputs are null.
        let ok = unsafe {
            GetVolumeInformationByHandleW(
                anchors[0].as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                filesystem.as_mut_ptr(),
                filesystem.len() as u32,
            )
        };
        if ok == 0 || filesystem[..5] != [b'N' as u16, b'T' as u16, b'F' as u16, b'S' as u16, 0] {
            return Err(error("settings require a local NTFS filesystem"));
        }
        for root in forbidden_roots {
            // Root identity comparison also covers casing and DOS short-name aliases.
            let canonical = std::fs::canonicalize(root).map_err(error)?;
            let forbidden = identity(&info(&open_directory(&canonical)?)?);
            for anchor in &anchors {
                if identity(&info(anchor)?) == forbidden {
                    return Err(error(
                        "directory is inside a source, scene, audio bank or executable root",
                    ));
                }
            }
        }
        let security = Security::new().map_err(error)?;
        security.private(anchors.last().unwrap()).map_err(error)?;
        let path = volume_path(anchors.last().unwrap())?;
        let mut store = Self {
            path,
            anchors,
            security,
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
    fn file_ok(&self, file: &File, limit: u32) -> Result<Identity, String> {
        let metadata = info(file)?;
        // SAFETY: GetFileType only inspects a live handle.
        if unsafe { GetFileType(file.as_raw_handle()) } != FILE_TYPE_DISK
            || metadata.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)
                != 0
            || metadata.nNumberOfLinks != 1
            || metadata.nFileSizeHigh != 0
            || metadata.nFileSizeLow > limit
        {
            return Err(error("expected a private single-link bounded regular file"));
        }
        self.security.private(file).map_err(error)?;
        Ok(identity(&metadata))
    }
    fn read(&self) -> Result<Option<Snapshot>, String> {
        self.security
            .private(self.anchors.last().unwrap())
            .map_err(error)?;
        let file = match OpenOptions::new()
            .read(true)
            .access_mode(GENERIC_READ | READ_SECURITY)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(self.path.join(self.profile.primary()))
        {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(error(e)),
        };
        let id = self.file_ok(&file, self.profile.limit())?;
        let mut bytes = Vec::new();
        (&file)
            .take(u64::from(self.profile.limit()) + 1)
            .read_to_end(&mut bytes)
            .map_err(error)?;
        self.profile.validate(&bytes)?;
        Ok(Some(Snapshot {
            identity: id,
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
        self.save_with_move(preferences, &mut hook, move_primary)
    }
    fn save_with_move(
        &mut self,
        preferences: &Preferences,
        mut hook: impl FnMut(Stage) -> Result<(), String>,
        mut move_file: impl FnMut(&Path, &Path) -> Result<(), String>,
    ) -> Outcome {
        let bytes = match serde_json::to_vec(preferences) {
            Ok(bytes) => bytes,
            Err(e) => return Outcome::NotSaved(error(e)),
        };
        self.save_bytes_with_move(bytes, &mut hook, &mut move_file)
    }
    #[cfg(feature = "coop-defense-controls")]
    pub(crate) fn save_bytes(&mut self, bytes: Vec<u8>) -> Outcome {
        self.save_bytes_with_move(bytes, |_| Ok(()), move_primary)
    }
    #[cfg(all(test, feature = "coop-defense-controls"))]
    pub(crate) fn test_fail_publication(&mut self, bytes: Vec<u8>, after: bool) -> Outcome {
        self.save_bytes_with_move(
            bytes,
            |stage| {
                if (stage == Stage::Published) == after {
                    Err("injected controls publication failure".into())
                } else {
                    Ok(())
                }
            },
            move_primary,
        )
    }
    fn save_bytes_with_move(
        &mut self,
        bytes: Vec<u8>,
        mut hook: impl FnMut(Stage) -> Result<(), String>,
        mut move_file: impl FnMut(&Path, &Path) -> Result<(), String>,
    ) -> Outcome {
        let mut published = false;
        let mut uncertain = false;
        let result = self.publish(
            bytes,
            &mut hook,
            &mut move_file,
            &mut published,
            &mut uncertain,
        );
        match result {
            Ok(()) => Outcome::Saved,
            Err(e) if uncertain => Outcome::Uncertain(e),
            Err(e) if published => Outcome::PublishedUncertain(e),
            Err(e) => Outcome::NotSaved(e),
        }
    }
    fn publish(
        &mut self,
        bytes: Vec<u8>,
        hook: &mut impl FnMut(Stage) -> Result<(), String>,
        move_file: &mut impl FnMut(&Path, &Path) -> Result<(), String>,
        published: &mut bool,
        uncertain: &mut bool,
    ) -> Result<(), String> {
        if !self.writable {
            return Err(error("Save disabled for this session"));
        }
        self.profile.validate(&bytes)?;
        self.security
            .private(self.anchors.last().unwrap())
            .map_err(error)?;
        let lock = match self.security.create(&self.path.join(self.profile.lock())) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => OpenOptions::new()
                .read(true)
                .write(true)
                .access_mode(GENERIC_READ | GENERIC_WRITE | READ_SECURITY)
                .share_mode(0)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(self.path.join(self.profile.lock()))
                .map_err(error)?,
            Err(e) => return Err(error(e)),
        };
        self.file_ok(&lock, 0)?;
        if self.read()? != self.baseline {
            return Err(error("settings changed on disk; restart to reload"));
        }
        let mut created = None;
        for _ in 0..32 {
            let name = self.path.join(format!(
                "{}{}-{}",
                self.profile.prefix(),
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match self.security.create(&name) {
                Ok(file) => {
                    created = Some((name, file));
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(error(e)),
            }
        }
        let (name, mut stage) = created.ok_or_else(|| error("staging name budget exhausted"))?;
        let stage_id = identity(&info(&stage)?);
        let result = (|| {
            self.file_ok(&stage, self.profile.limit())?;
            stage.write_all(&bytes).map_err(error)?;
            stage.sync_all().map_err(error)?;
            self.file_ok(&stage, self.profile.limit())?;
            // Windows rename requires closing handles that deny delete sharing.
            drop(stage);
            hook(Stage::Prepared)?;
            self.security
                .private(self.anchors.last().unwrap())
                .map_err(error)?;
            let check = OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(&name)
                .map_err(error)?;
            if self.file_ok(&check, self.profile.limit())? != stage_id {
                return Err(error("staging file identity changed"));
            }
            drop(check);
            if self.read()? != self.baseline {
                return Err(error("settings changed before publication"));
            }
            if let Err(e) = move_file(&name, &self.path.join(self.profile.primary())) {
                match self.read() {
                    Ok(Some(current)) if current.identity == stage_id => {
                        *published = true;
                        self.baseline = Some(current);
                    }
                    Ok(current) if current == self.baseline => {}
                    _ => {
                        self.writable = false;
                        *uncertain = true;
                    }
                }
                return Err(e);
            }
            *published = true;
            self.baseline = Some(Snapshot {
                identity: stage_id,
                bytes,
            });
            hook(Stage::Published)?;
            Ok(())
        })();
        // Leave substituted/inaccessible files alone; cleanup only our own unpublished inode.
        if let Ok(file) = OpenOptions::new()
            .access_mode(DELETE | FILE_READ_ATTRIBUTES | READ_SECURITY)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&name)
        {
            let ours = info(&file)
                .map(|m| {
                    identity(&m) == stage_id
                        && m.nNumberOfLinks == 1
                        && m.dwFileAttributes
                            & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)
                            == 0
                })
                .unwrap_or(false);
            if ours {
                let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
                // SAFETY: identity was checked on this same DELETE handle; no pathname is resolved.
                unsafe {
                    SetFileInformationByHandle(
                        file.as_raw_handle(),
                        FileDispositionInfo,
                        (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                        std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
                    );
                }
            }
        }
        result
    }
}
#[cfg(test)]
#[path = "defense_audio_settings_windows_tests.rs"]
pub(crate) mod tests;
