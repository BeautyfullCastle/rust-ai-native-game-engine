//! Create-only sprite JSON publication for trusted, quiescent destination ancestry.
//!
//! Path checks and retained handles detect replacements at observation points,
//! but are not a hostile-filesystem sandbox or an atomic concurrency lock.
//! The caller must exclude concurrent mutation of ancestry and staging files.
//! Existing destinations are never overwritten. Once linked, errors become
//! warnings: the output exists or may exist, and retry is not safe.
use orr_sprite::{SpriteDocument, MAX_JSON_BYTES};
type Result<T> = std::result::Result<T, String>;
use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Debug)]
pub(super) struct Published {
    pub(super) bytes: usize,
    pub(super) warning: Option<String>,
}

fn failure(context: &str, error: impl std::fmt::Display) -> String {
    format!("{context}: {error}")
}

fn same_file(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        false // Fail closed: std does not expose a portable stable file identity.
    }
}

struct Parent {
    path: PathBuf,
    handle: File,
    ancestry: Vec<(PathBuf, Metadata)>,
}
impl Parent {
    fn open(output: &Path) -> Result<Self> {
        if !output.is_absolute()
            || output.as_os_str().len() > 4096
            || output.extension().is_none_or(|e| e != "json")
            || output
                .as_os_str()
                .as_encoded_bytes()
                .split(|b| *b == b'/')
                .any(|part| part == b"." || part == b"..")
        {
            return Err("output must be an explicit absolute .json path, at most 4096 bytes, without traversal".into());
        }
        let path = output
            .parent()
            .ok_or_else(|| "output needs a parent directory".to_owned())?
            .to_owned();
        let mut current = PathBuf::new();
        let mut ancestry = Vec::new();
        for component in path.components() {
            current.push(component.as_os_str());
            if current.file_name().is_some_and(|n| {
                n.as_encoded_bytes().eq_ignore_ascii_case(b".orr")
                    || n.as_encoded_bytes().eq_ignore_ascii_case(b"store")
            }) || current
                .extension()
                .is_some_and(|e| e.as_encoded_bytes().eq_ignore_ascii_case(b"orr"))
            {
                return Err("output inside .orr/store ancestry is forbidden".into());
            }
            let metadata =
                fs::symlink_metadata(&current).map_err(|e| failure("output ancestry", e))?;
            if !metadata.file_type().is_dir() {
                return Err(
                    "output ancestry must contain only real directories, not symlinks".into(),
                );
            }
            ancestry.push((current.clone(), metadata));
        }
        let handle = File::open(&path).map_err(|e| failure("open output parent", e))?;
        let parent = Self {
            path,
            handle,
            ancestry,
        };
        parent.check()?;
        absent(output)?;
        Ok(parent)
    }

    fn check(&self) -> Result<()> {
        for (path, before) in &self.ancestry {
            let now =
                fs::symlink_metadata(path).map_err(|e| failure("output ancestry recheck", e))?;
            if !now.file_type().is_dir() || !same_file(before, &now) {
                return Err("output ancestry changed or stable identity is unsupported".into());
            }
        }
        let now = self
            .handle
            .metadata()
            .map_err(|e| failure("held parent metadata", e))?;
        if !self
            .ancestry
            .last()
            .is_some_and(|(_, before)| same_file(before, &now))
        {
            return Err("held output parent changed".into());
        }
        Ok(())
    }
}

fn absent(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(failure("output destination metadata", e)),
        Ok(_) => Err("output destination already exists; never overwritten".into()),
    }
}

struct Stage {
    path: PathBuf,
    handle: File,
    identity: Metadata,
}
impl Stage {
    fn create(parent: &Parent) -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..128 {
            let path = parent.path.join(format!(
                ".sprite-timing-{}-{}.tmp",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(handle) => {
                    let identity = handle.metadata().map_err(|e| failure(
                        &format!("stage identity unavailable; owned staging path may remain at {} (not removed without verified identity)", path.display()), e))?;
                    return Ok(Self {
                        path,
                        handle,
                        identity,
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(failure("exclusive output staging", e)),
            }
        }
        Err("exclusive output staging names exhausted".into())
    }

    fn check(&self, parent: &Parent) -> Result<()> {
        parent.check()?;
        let path = fs::symlink_metadata(&self.path).map_err(|e| failure("stage recheck", e))?;
        let held = self
            .handle
            .metadata()
            .map_err(|e| failure("held stage metadata", e))?;
        if !path.file_type().is_file()
            || !held.is_file()
            || !same_file(&self.identity, &path)
            || !same_file(&self.identity, &held)
        {
            return Err("staging file replaced; foreign path will not be removed".into());
        }
        Ok(())
    }

    fn verify(&mut self, expected: &[u8]) -> Result<()> {
        self.handle
            .seek(SeekFrom::Start(0))
            .map_err(|e| failure("stage readback seek", e))?;
        let mut actual = Vec::with_capacity(expected.len() + 1);
        (&mut self.handle)
            .take(expected.len() as u64 + 1)
            .read_to_end(&mut actual)
            .map_err(|e| failure("stage readback", e))?;
        if actual != expected {
            return Err("staging exact byte readback mismatch".into());
        }
        Ok(())
    }

    fn cleanup(&self, parent: &Parent) -> Result<()> {
        self.check(parent)?;
        fs::remove_file(&self.path).map_err(|e| failure("remove owned staging link", e))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Point {
    StageCreated,
    Written,
    Linked,
    BeforeDirectorySync,
    BeforeCleanup,
}

/// All conversion completes before staging. The refusal callback runs before
/// staging and immediately before final identity/readback checks and no-replace
/// publication. It is not a concurrency guarantee. Only Unix stable file IDs are
/// supported; other targets fail closed before staging.
pub(super) fn publish_new(
    output: &Path,
    document: &SpriteDocument,
    check: impl FnMut() -> Result<()>,
) -> Result<Published> {
    publish(output, document, check, |_, _| Ok(()))
}

fn publish(
    output: &Path,
    document: &SpriteDocument,
    mut check: impl FnMut() -> Result<()>,
    mut hook: impl FnMut(Point, &Path) -> Result<()>,
) -> Result<Published> {
    let json = document
        .to_json()
        .map_err(|e| failure("serialize sprite", e))?;
    if json.len() > MAX_JSON_BYTES {
        return Err("sprite JSON exceeds byte limit".into());
    }
    let reloaded =
        SpriteDocument::from_json(&json).map_err(|e| failure("validate serialized sprite", e))?;
    if reloaded
        .to_json()
        .map_err(|e| failure("canonical sprite", e))?
        != json
    {
        return Err("sprite canonical reload mismatch".into());
    }
    let bytes = json.as_bytes();
    let parent = Parent::open(output)?;
    check()?;
    parent.check()?;
    absent(output)?;
    let mut stage = Stage::create(&parent)?;
    let prepared = (|| {
        hook(Point::StageCreated, &stage.path)?;
        stage
            .handle
            .write_all(bytes)
            .map_err(|e| failure("stage write", e))?;
        hook(Point::Written, &stage.path)?;
        stage
            .handle
            .sync_all()
            .map_err(|e| failure("stage sync", e))?;
        stage.check(&parent)?;
        stage.verify(bytes)?;
        check()?;
        stage.check(&parent)?;
        stage.verify(bytes)?;
        absent(output)?;
        stage.check(&parent)?;
        fs::hard_link(&stage.path, output).map_err(|e| failure("no-replace publication", e))
    })();
    if let Err(mut error) = prepared {
        if let Err(cleanup) = stage.cleanup(&parent) {
            error.push_str(&format!("; staging cleanup refused/failed: {cleanup}"));
        }
        return Err(error);
    }
    let mut warnings = Vec::new();
    let confirmed = (|| {
        hook(Point::Linked, &stage.path)?;
        stage.check(&parent)?;
        let published =
            fs::symlink_metadata(output).map_err(|e| failure("published output readback", e))?;
        if !published.file_type().is_file() || !same_file(&stage.identity, &published) {
            return Err("published output identity changed".into());
        }
        stage.verify(bytes)?;
        hook(Point::BeforeDirectorySync, &stage.path)?;
        parent
            .handle
            .sync_all()
            .map_err(|e| failure("output parent sync", e))
    })();
    if let Err(error) = confirmed {
        warnings.push(format!(
            "output exists or may exist; do not retry blindly: {error}"
        ));
    }
    let cleanup = hook(Point::BeforeCleanup, &stage.path).and_then(|()| stage.cleanup(&parent));
    let cleanup_warning = cleanup
        .err()
        .map(|e| format!("output was published; staging cleanup refused/failed: {e}"));
    if cleanup_warning.is_none() {
        if let Err(error) = parent.handle.sync_all() {
            warnings.push(format!(
                "output was published; staging cleanup durability is uncertain: {error}"
            ));
        }
    }
    if let Some(cleanup) = cleanup_warning {
        warnings.push(format!(
            "{cleanup}; output exists or may exist; do not retry blindly"
        ));
    }
    Ok(Published {
        bytes: bytes.len(),
        warning: (!warnings.is_empty()).then(|| warnings.join("; ")),
    })
}

#[cfg(test)]
mod tests;
