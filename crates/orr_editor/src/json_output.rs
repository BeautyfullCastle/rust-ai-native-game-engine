//! Create-only JSON byte publication for trusted, quiescent destination ancestry.
//!
//! Path checks and retained handles detect replacements at observation points,
//! but are not a hostile-filesystem sandbox or an atomic concurrency lock.
//! The caller must exclude concurrent mutation of ancestry and staging files.
//! Existing destinations are never overwritten. Once linked, errors become
//! warnings: the output exists or may exist, and retry is not safe.
type Result<T> = std::result::Result<T, String>;
use orr_fsid::{FileId, PinnedDirectory};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Debug)]
pub(crate) struct Published {
    pub(crate) bytes: usize,
    pub(crate) warning: Option<String>,
}

fn failure(context: &str, error: impl std::fmt::Display) -> String {
    format!("{context}: {error}")
}

struct Parent {
    path: PathBuf,
    ancestry: Vec<PinnedDirectory>,
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
            // A drive or verbatim prefix such as `\\?\C:` is not a filesystem object.
            if matches!(component, std::path::Component::Prefix(_)) {
                continue;
            }
            if current.file_name().is_some_and(|n| {
                n.as_encoded_bytes().eq_ignore_ascii_case(b".orr")
                    || n.as_encoded_bytes().eq_ignore_ascii_case(b"store")
            }) || current
                .extension()
                .is_some_and(|e| e.as_encoded_bytes().eq_ignore_ascii_case(b"orr"))
            {
                return Err("output inside .orr/store ancestry is forbidden".into());
            }
            // One open both checks the type and pins the identity, so a link or a
            // replaced folder cannot slip in between a check and the open.
            let directory = PinnedDirectory::open(&current).map_err(|e| {
                failure(
                    "output ancestry must contain only real directories, not symlinks",
                    e,
                )
            })?;
            ancestry.push(directory);
        }
        if ancestry.is_empty() {
            return Err("output needs a parent directory".into());
        }
        let parent = Self { path, ancestry };
        parent.check()?;
        absent(output)?;
        Ok(parent)
    }

    fn check(&self) -> Result<()> {
        for directory in &self.ancestry {
            directory.verify_unchanged().map_err(|e| {
                failure(
                    "output ancestry changed or stable identity is unsupported",
                    e,
                )
            })?;
        }
        Ok(())
    }

    /// The open handle of the output parent folder, the last pinned ancestor.
    fn handle(&self) -> &File {
        self.ancestry
            .last()
            .map(PinnedDirectory::file)
            .expect("Parent::open keeps at least one ancestor")
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
    identity: FileId,
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
                    let identity = orr_fsid::file_id(&handle).map_err(|e| failure(
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
        let named = orr_fsid::open_file(&self.path).map_err(|e| failure("stage recheck", e))?;
        let held = self
            .handle
            .metadata()
            .map_err(|e| failure("held stage metadata", e))?;
        let named_id = orr_fsid::file_id(&named).map_err(|e| failure("stage recheck", e))?;
        let held_id = orr_fsid::file_id(&self.handle).map_err(|e| failure("held stage", e))?;
        if !held.is_file() || named_id != self.identity || held_id != self.identity {
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
pub(crate) enum Point {
    StageCreated,
    Written,
    Linked,
    BeforeDirectorySync,
    BeforeCleanup,
}

/// Callers must validate and bound the JSON bytes before calling. No JSON
/// parsing or schema validation is performed here. The refusal callback runs before
/// staging and immediately before final identity/readback checks and no-replace
/// publication. It is not a concurrency guarantee. File identity comes from open handles
/// (`orr_fsid`); other targets fail closed before staging.
pub(crate) fn publish_new(
    output: &Path,
    bytes: &[u8],
    check: impl FnMut() -> Result<()>,
) -> Result<Published> {
    publish_with_hook(output, bytes, check, |_, _| Ok(()))
}

pub(crate) fn publish_with_hook(
    output: &Path,
    bytes: &[u8],
    mut check: impl FnMut() -> Result<()>,
    mut hook: impl FnMut(Point, &Path) -> Result<()>,
) -> Result<Published> {
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
            orr_fsid::open_file(output).map_err(|e| failure("published output readback", e))?;
        let published_id =
            orr_fsid::file_id(&published).map_err(|e| failure("published output readback", e))?;
        if published_id != stage.identity {
            return Err("published output identity changed".into());
        }
        stage.verify(bytes)?;
        hook(Point::BeforeDirectorySync, &stage.path)?;
        parent
            .handle()
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
        if let Err(error) = parent.handle().sync_all() {
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn publishes_prepared_json_bytes_without_sprite_schema() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("pattern.json");
        let bytes = br#"{"format":"test_pattern","cells":[1,2,3]}"#;
        let mut calls = 0;
        let result = publish_new(&output, bytes, || {
            calls += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(result.bytes, bytes.len());
        assert!(result.warning.is_none());
        assert_eq!(fs::read(&output).unwrap(), bytes);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        assert!(publish_new(&output, b"{}", || Ok(())).is_err());
        assert_eq!(fs::read(&output).unwrap(), bytes);
    }

    #[test]
    fn final_source_refusal_removes_staging_without_publishing() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("pattern.json");
        let mut calls = 0;
        let result = publish_new(&output, b"{}", || {
            calls += 1;
            if calls == 2 {
                Err("source bytes changed".into())
            } else {
                Ok(())
            }
        });
        assert_eq!(result.unwrap_err(), "source bytes changed");
        assert_eq!(calls, 2);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn a_junction_in_the_output_ancestry_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let target = root.join("target");
        let junction = root.join("junction");
        fs::create_dir(&target).unwrap();
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&target)
            .output()
            .unwrap();
        assert!(made.status.success(), "{made:?}");
        assert!(publish_new(&junction.join("pattern.json"), b"{}", || Ok(())).is_err());
        assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
    }

    #[test]
    fn a_staging_file_replaced_by_another_file_is_refused_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let output = fs::canonicalize(dir.path()).unwrap().join("pattern.json");
        let result = publish_with_hook(
            &output,
            b"{}",
            || Ok(()),
            |point, stage| {
                if point == Point::StageCreated {
                    fs::remove_file(stage).unwrap();
                    fs::write(stage, b"foreign").unwrap();
                }
                Ok(())
            },
        );
        assert!(result.is_err());
        assert!(!output.exists());
        let foreign: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| fs::read(entry.unwrap().path()).unwrap())
            .collect();
        assert_eq!(foreign, vec![b"foreign".to_vec()]);
    }

    #[test]
    fn publishes_into_verbatim_canonical_parent() {
        let dir = tempfile::tempdir().unwrap();
        let output = fs::canonicalize(dir.path()).unwrap().join("pattern.json");
        assert!(output.to_str().unwrap().starts_with(r"\\?\"));
        let result = publish_new(&output, b"{}", || Ok(())).unwrap();
        assert_eq!(result.bytes, 2);
        assert_eq!(fs::read(&output).unwrap(), b"{}");
    }
}
