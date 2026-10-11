//! Optional sprite-clip authoring core with explicit Linux new-package publication.
//! No project-lock activation or simulation writes.
//! The caller admits the installed package before supplying its owned snapshot.
use orr_package::LockedPackage;
use orr_sprite::{Frame, PlaybackMode, SpriteDocument, SpriteSource};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_DOCUMENT_BYTES: usize = 256 * 1024;
pub const MAX_PACKAGE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_HISTORY: usize = 32;

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}
fn validate(source: &SpriteSource) -> Result<(), String> {
    SpriteDocument::new(source.clone()).map_err(|e| e.to_string())?;
    if serde_json::to_vec_pretty(source).map_err(|e| e.to_string())?.len() > MAX_DOCUMENT_BYTES {
        return Err("clip authoring document exceeds 256 KiB".into());
    }
    Ok(())
}
fn portable(path: &str) -> bool {
    !path.is_empty() && !path.contains(['\\', ':'])
        && path.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
}
fn stable_version(version: &str) -> Result<[u32; 3], String> {
    if version.len() > 32 { return Err("stable version exceeds byte limit".into()); }
    let mut parts = version.split('.');
    let mut values = [0; 3];
    for slot in &mut values {
        let part = parts.next().ok_or("use a stable major.minor.patch version")?;
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0')) {
            return Err("use a canonical stable major.minor.patch version".into());
        }
        *slot = part.parse().map_err(|_| "version component exceeds u32")?;
    }
    if parts.next().is_some() { return Err("use a stable major.minor.patch version".into()); }
    Ok(values)
}

/// One existing clip is editable; atlas, regions, IDs and all other clips stay fixed.
/// Successful no-ops and rejected edits preserve both history directions.
pub struct ClipDocument {
    original: SpriteSource,
    current: SpriteSource,
    selected: usize,
    undo: Vec<SpriteSource>,
    redo: Vec<SpriteSource>,
}
impl ClipDocument {
    pub fn open(bytes: &[u8], clip_id: &str) -> Result<Self, String> {
        if bytes.len() > MAX_DOCUMENT_BYTES { return Err("clip document too large".into()); }
        let source: SpriteSource = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        validate(&source)?;
        let selected = source.clips.iter().position(|c| c.id == clip_id)
            .ok_or("select an existing clip")?;
        Ok(Self { original: source.clone(), current: source, selected, undo: Vec::new(), redo: Vec::new() })
    }
    pub fn source(&self) -> &SpriteSource { &self.current }
    pub fn can_undo(&self) -> bool { !self.undo.is_empty() }
    pub fn can_redo(&self) -> bool { !self.redo.is_empty() }
    pub fn replace_frames(&mut self, frames: Vec<Frame>, mode: PlaybackMode) -> Result<bool, String> {
        // Bound caller-owned collections before cloning the document or frames.
        if frames.is_empty() || frames.len() > orr_sprite::MAX_FRAMES_PER_CLIP {
            return Err("clip frame count outside authoring bounds".into());
        }
        let mut next = self.current.clone();
        next.clips[self.selected].frames = frames;
        next.clips[self.selected].mode = mode;
        validate(&next)?;
        if next == self.current { return Ok(false); }
        if self.undo.len() == MAX_HISTORY { self.undo.remove(0); }
        self.undo.push(std::mem::replace(&mut self.current, next));
        self.redo.clear();
        Ok(true)
    }
    pub fn set_duration(&mut self, index: usize, duration_ms: u64) -> Result<bool, String> {
        let clip = &self.current.clips[self.selected];
        let mut frames = clip.frames.clone();
        frames.get_mut(index).ok_or("frame index outside clip")?.duration_ms = duration_ms;
        self.replace_frames(frames, clip.mode)
    }
    pub fn move_frame(&mut self, from: usize, to: usize) -> Result<bool, String> {
        let clip = &self.current.clips[self.selected];
        if from >= clip.frames.len() || to >= clip.frames.len() { return Err("frame index outside clip".into()); }
        let mut frames = clip.frames.clone();
        let frame = frames.remove(from);
        frames.insert(to, frame);
        self.replace_frames(frames, clip.mode)
    }
    pub fn set_mode(&mut self, mode: PlaybackMode) -> Result<bool, String> {
        self.replace_frames(self.current.clips[self.selected].frames.clone(), mode)
    }
    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop() else { return false };
        self.redo.push(std::mem::replace(&mut self.current, previous));
        true
    }
    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else { return false };
        self.undo.push(std::mem::replace(&mut self.current, next));
        true
    }
    pub fn sample(&self, elapsed_ms: u64) -> Result<orr_sprite::Sample, String> {
        let document = SpriteDocument::new(self.current.clone()).map_err(|e| e.to_string())?;
        Ok(document.clip(&self.current.clips[self.selected].id).ok_or("missing selected clip")?.sample(elapsed_ms))
    }
}

/// A mandatory contract for the future filesystem publisher, never a replace mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DestinationPolicy { CreateNewDirectory }

/// Prepared bytes do not activate a lock. Constructing this plan does not write.
/// `publish_new` optionally writes a Linux private stage and uses atomic no-replace
/// publication. Runtime capability/PNG admission remains the caller's authority.
pub struct SaveAsPlan {
    files: BTreeMap<String, Vec<u8>>,
    original_package_digest: String,
}
impl SaveAsPlan {
    pub fn files(&self) -> &BTreeMap<String, Vec<u8>> { &self.files }
    pub fn original_package_digest(&self) -> &str { &self.original_package_digest }
    pub fn destination_policy(&self) -> DestinationPolicy { DestinationPolicy::CreateNewDirectory }
    pub fn prepare(
        document: &ClipDocument,
        package: &LockedPackage,
        snapshot: &BTreeMap<String, Vec<u8>>,
        document_path: &str,
        next_version: &str,
    ) -> Result<Self, String> {
        if stable_version(next_version)? <= stable_version(&package.manifest.version)? {
            return Err("Save As requires a strictly newer stable package version".into());
        }
        if !package.manifest.dependencies.is_empty() {
            return Err("version-one clip tool accepts a dependency-free sprite package".into());
        }
        let image = &document.original.atlas.image;
        if !image.ends_with(".png") { return Err("clip tool requires a PNG atlas".into()); }
        let raw = format!("{}.rgba", image.strip_suffix(".png").ok_or("PNG suffix missing")?);
        let required: BTreeSet<String> = [document_path.to_owned(), image.clone(), "LICENSE.txt".into()].into();
        let mut with_raw = required.clone(); with_raw.insert(raw);
        if required.len() != 3 || (package.manifest.files != required && package.manifest.files != with_raw)
            || !package.manifest.files.iter().all(|p| portable(p) && p != "orr.package.json") {
            return Err("expected only sprite document, PNG, LICENSE.txt and optional same-stem RGBA".into());
        }
        if snapshot.keys().cloned().collect::<BTreeSet<_>>() != package.manifest.files
            || package.files.keys().cloned().collect::<BTreeSet<_>>() != package.manifest.files {
            return Err("snapshot does not match admitted package file inventory".into());
        }
        let mut total = 0usize;
        for (path, bytes) in snapshot {
            total = total.checked_add(bytes.len()).ok_or("package size overflow")?;
            if total > MAX_PACKAGE_BYTES || digest(bytes) != package.files[path] {
                return Err("snapshot exceeds limit or differs from admitted package".into());
            }
        }
        let original: SpriteSource = serde_json::from_slice(&snapshot[document_path]).map_err(|e| e.to_string())?;
        if original != document.original { return Err("document was opened from another package snapshot".into()); }
        validate(&document.current)?;
        let mut files = snapshot.clone();
        files.insert(document_path.into(), serde_json::to_vec_pretty(&document.current).map_err(|e| e.to_string())?);
        let mut manifest = package.manifest.clone();
        manifest.version = next_version.into();
        files.insert("orr.package.json".into(), serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?);
        let size = files.values().try_fold(0usize, |sum, bytes| sum.checked_add(bytes.len())).ok_or("output size overflow")?;
        if size > MAX_PACKAGE_BYTES { return Err("Save As output exceeds package limit".into()); }
        Ok(Self { files, original_package_digest: package.digest.clone() })
    }
}

/// On failure, no caller-owned path is removed. A private incomplete stage may
/// remain for explicit recovery; its path is only a locator if a parent was moved.
/// Automatic cleanup is intentionally absent: never recursively delete a name
/// that another actor could have replaced. A crash can leave a private stage, or
/// an already-published destination with uncertain durability. This is not a
/// claim of protection against malicious concurrent same-user directory mutation.
/// After publication, an fsync error is reported
/// with `published=true`; callers must never retry by deleting the destination.
#[derive(Debug)]
pub struct SaveError {
    pub message: String,
    pub leftover_stage: Option<std::path::PathBuf>,
    pub published: bool,
}
impl SaveAsPlan {
    /// Linux-only atomic no-replace directory publication. Unsupported platforms
    /// fail closed; there is deliberately no replacing-rename fallback.
    pub fn publish_new(&self, destination: &std::path::Path) -> Result<(), SaveError> {
        #[cfg(target_os = "linux")]
        { self.publish_new_with_hook(destination, |_, _| Ok(())) }
        #[cfg(not(target_os = "linux"))]
        { let _ = destination; Err(SaveError { message: "sprite package publication requires Linux atomic no-replace support".into(), leftover_stage: None, published: false }) }
    }
    #[cfg(target_os = "linux")]
    fn publish_new_with_hook(
        &self,
        destination: &std::path::Path,
        hook: impl FnMut(&str, &std::path::Path) -> Result<(), String>,
    ) -> Result<(), SaveError> {
        linux_publish::publish(self, destination, hook)
    }
    #[cfg(all(test, target_os = "linux"))]
    pub fn publish_test_hook(
        &self,
        destination: &std::path::Path,
        hook: impl FnMut(&str, &std::path::Path) -> Result<(), String>,
    ) -> Result<(), SaveError> { self.publish_new_with_hook(destination, hook) }
}

#[cfg(target_os = "linux")]
mod linux_publish {
    use super::{SaveAsPlan, SaveError, MAX_PACKAGE_BYTES};
    use rustix::fs::{self, AtFlags, Mode, OFlags, RenameFlags};
    use std::{collections::BTreeSet, fs::File, io::{Read, Write}, os::fd::OwnedFd,
        path::{Path, PathBuf}, sync::atomic::{AtomicU64, Ordering}, time::{SystemTime, UNIX_EPOCH}};
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    fn message(error: impl std::fmt::Display) -> String { error.to_string() }
    // Sticky trusted directories (notably root-owned /tmp) prevent other users
    // from replacing our owned stage. Every component must itself be trusted.
    fn trusted(fd: &OwnedFd) -> Result<(), String> {
        let stat = fs::fstat(fd).map_err(message)?;
        let uid = rustix::process::geteuid().as_raw();
        if stat.st_uid != 0 && stat.st_uid != uid {
            return Err("destination ancestry has an untrusted owner".into());
        }
        if stat.st_mode & 0o022 != 0 && stat.st_mode & 0o1000 == 0 {
            return Err("destination ancestry is writable without sticky protection".into());
        }
        Ok(())
    }
    fn directory(parent: &OwnedFd, name: &str) -> Result<OwnedFd, String> {
        let fd = fs::openat(parent, name, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty()).map_err(message)?;
        trusted(&fd)?;
        Ok(fd)
    }
    fn identity(fd: &OwnedFd) -> Result<(u64, u64), String> {
        let stat = fs::fstat(fd).map_err(message)?;
        Ok((stat.st_dev, stat.st_ino))
    }
    fn parent(path: &Path) -> Result<(OwnedFd, String, PathBuf), String> {
        let text = path.to_str().ok_or("destination must be UTF-8")?;
        if text.len() > 4096 || !text.starts_with('/') || text.contains(['\0', '\\', ':']) {
            return Err("use a bounded absolute destination path".into());
        }
        let parts: Vec<_> = text[1..].split('/').collect();
        if parts.is_empty() || parts.len() > 64 || parts.iter().any(|p| p.is_empty() || *p == "." || *p == ".." || p.len() > 255) {
            return Err("destination contains empty, traversal or excessive components".into());
        }
        let mut fd = fs::open("/", OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC, Mode::empty()).map_err(message)?;
        trusted(&fd)?;
        for part in &parts[..parts.len()-1] { fd = directory(&fd, part)?; }
        Ok((fd, parts[parts.len()-1].into(), path.parent().ok_or("destination parent missing")?.to_path_buf()))
    }
    fn relative_parent(root: &OwnedFd, path: &str) -> Result<(OwnedFd, String), String> {
        let parts: Vec<_> = path.split('/').collect();
        if parts.len() > 32 || !super::portable(path) || path.contains('\0') || parts.iter().any(|p| p.len() > 255) {
            return Err("package path outside writer bounds".into());
        }
        let mut fd = directory(root, ".")?;
        for part in &parts[..parts.len()-1] { fd = directory(&fd, part)?; }
        Ok((fd, parts[parts.len()-1].into()))
    }
    fn exact_inventory(root: &OwnedFd, files: &std::collections::BTreeMap<String, Vec<u8>>, directories: &BTreeSet<String>) -> Result<(), String> {
        // Enumerate only bounded expected directories, never attacker-selected recursion.
        for path in std::iter::once("").chain(directories.iter().map(String::as_str)) {
            let fd = if path.is_empty() { directory(root, ".")? } else {
                let (parent, name) = relative_parent(root, path)?;
                directory(&parent, &name)?
            };
            let prefix = if path.is_empty() { String::new() } else { format!("{path}/") };
            let expected: BTreeSet<&str> = files.keys().chain(directories.iter()).filter_map(|p| {
                p.strip_prefix(&prefix).filter(|tail| !tail.is_empty() && !tail.contains('/'))
            }).collect();
            let mut seen = BTreeSet::new();
            for entry in fs::Dir::read_from(&fd).map_err(message)? {
                let entry = entry.map_err(message)?;
                let name = entry.file_name().to_str().map_err(message)?;
                if name == "." || name == ".." { continue; }
                if !expected.contains(name) || seen.len() >= expected.len() || !seen.insert(name.to_owned()) {
                    return Err("staged package inventory changed".into());
                }
                let stat = fs::statat(&fd, name, AtFlags::SYMLINK_NOFOLLOW).map_err(message)?;
                let full = format!("{prefix}{name}");
                let expected_type = if directories.contains(&full) { fs::FileType::Directory } else { fs::FileType::RegularFile };
                if fs::FileType::from_raw_mode(stat.st_mode) != expected_type {
                    return Err("staged entry type changed".into());
                }
            }
            if seen.len() != expected.len() { return Err("staged package inventory incomplete".into()); }
        }
        Ok(())
    }
    pub(super) fn publish(
        plan: &SaveAsPlan,
        output: &Path,
        mut hook: impl FnMut(&str, &Path) -> Result<(), String>,
    ) -> Result<(), SaveError> {
        let mut leftover = None;
        let mut published = false;
        let result = (|| -> Result<(), String> {
            let (parent_fd, output_name, parent_path) = parent(output)?;
            let parent_id = identity(&parent_fd)?;
            match fs::statat(&parent_fd, output_name.as_str(), AtFlags::SYMLINK_NOFOLLOW) {
                Err(rustix::io::Errno::NOENT) => (),
                Ok(_) => return Err("destination already exists".into()),
                Err(e) => return Err(message(e)),
            }
            let total = plan.files.values().try_fold(0usize, |n, b| n.checked_add(b.len())).ok_or("package size overflow")?;
            if total > MAX_PACKAGE_BYTES || plan.files.len() > 5 { return Err("package exceeds writer bounds".into()); }
            let mut directories = BTreeSet::new();
            for path in plan.files.keys() {
                if path.len() > 1024 || path.contains('\0') || !super::portable(path) { return Err("invalid package path".into()); }
                let parts: Vec<_> = path.split('/').collect();
                if parts.len() > 32 || parts.iter().any(|p| p.len() > 255) { return Err("package path outside writer bounds".into()); }
                for n in 1..parts.len() { directories.insert(parts[..n].join("/")); }
            }
            let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_err(message)?.as_nanos();
            let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
            let mut stage_name = None;
            for attempt in 0..64 {
                let name = format!(".orr-sprite-clip-{}-{stamp:x}-{serial:x}-{attempt}", std::process::id());
                match fs::mkdirat(&parent_fd, name.as_str(), Mode::from_bits_truncate(0o700)) {
                    Ok(()) => { stage_name = Some(name); break; },
                    Err(rustix::io::Errno::EXIST) => (),
                    Err(e) => return Err(message(e)),
                }
            }
            let stage_name = stage_name.ok_or("could not allocate a fresh private stage")?;
            let stage_path = parent_path.join(&stage_name);
            leftover = Some(stage_path.clone());
            let stage = directory(&parent_fd, &stage_name)?;
            let stage_id = identity(&stage)?;
            hook("stage-created", &stage_path)?;
            for path in &directories {
                let (fd, name) = relative_parent(&stage, path)?;
                fs::mkdirat(&fd, name.as_str(), Mode::from_bits_truncate(0o700)).map_err(message)?;
            }
            for (path, bytes) in &plan.files {
                let (fd, name) = relative_parent(&stage, path)?;
                let raw = fs::openat(&fd, name.as_str(), OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::from_bits_truncate(0o600)).map_err(message)?;
                let mut file = File::from(raw);
                file.write_all(bytes).map_err(message)?;
                file.sync_all().map_err(message)?;
                hook("file-written", &stage_path)?;
            }
            hook("before-publish", &stage_path)?;
            exact_inventory(&stage, &plan.files, &directories)?;
            // Byte-for-byte bounded readback; never trust a partial write as a package.
            for (path, expected) in &plan.files {
                let (fd, name) = relative_parent(&stage, path)?;
                let raw = fs::openat(&fd, name.as_str(), OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC, Mode::empty()).map_err(message)?;
                let stat = fs::fstat(&raw).map_err(message)?;
                if fs::FileType::from_raw_mode(stat.st_mode) != fs::FileType::RegularFile || stat.st_size != expected.len() as i64 {
                    return Err("staged payload is not the expected regular file".into());
                }
                let mut bytes = Vec::new();
                File::from(raw).take((expected.len()+1) as u64).read_to_end(&mut bytes).map_err(message)?;
                if &bytes != expected { return Err("staged package bytes changed".into()); }
            }
            for path in directories.iter().rev() {
                let (fd, name) = relative_parent(&stage, path)?;
                fs::fsync(directory(&fd, &name)?).map_err(message)?;
            }
            fs::fsync(&stage).map_err(message)?;
            let (current_parent, current_name, _) = parent(output)?;
            if identity(&current_parent)? != parent_id || current_name != output_name {
                return Err("destination parent changed".into());
            }
            let named_stage = fs::statat(&parent_fd, stage_name.as_str(), AtFlags::SYMLINK_NOFOLLOW).map_err(message)?;
            if (named_stage.st_dev, named_stage.st_ino) != stage_id { return Err("private stage name changed".into()); }
            fs::renameat_with(&parent_fd, stage_name.as_str(), &parent_fd, output_name.as_str(), RenameFlags::NOREPLACE).map_err(message)?;
            published = true; leftover = None;
            hook("after-publish", output)?;
            fs::fsync(&parent_fd).map_err(message)?;
            Ok(())
        })();
        result.map_err(|message| SaveError { message, leftover_stage: leftover, published })
    }
}

#[cfg(test)]
#[path = "sprite_clip_authoring_tests.rs"]
mod tests;
