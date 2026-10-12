//! No-replace publication in an existing, caller-owned trusted directory.
//!
//! Both input and output ancestry must stay stable. These std path checks are
//! not capability-safe against hostile same-user/root replacement: a substituted
//! FIFO can even block between metadata and open. A retained input byte snapshot
//! is not a guarantee against concurrent in-place modification. No permission
//! bits are changed; no package is installed. Hard-link support is required.
use crate::{import_raw16, Error, Recipe, Result};
use orr_terrain::Terrain;
use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static STAGE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Published {
    pub revision: [u8; 32],
    pub bytes: usize,
    /// None means this platform completed parent-directory sync after link and
    /// stage cleanup. It is not a guarantee about storage hardware power loss.
    pub durability_warning: Option<String>,
    /// A published destination is never rolled back after a cleanup failure.
    /// Only our uniquely created stage is eligible for cleanup.
    pub cleanup_warning: Option<String>,
}
impl Published {
    pub fn has_warnings(&self) -> bool {
        self.durability_warning.is_some() || self.cleanup_warning.is_some()
    }
}

fn checked_path(path: &Path, output: bool) -> Result<PathBuf> {
    if path.as_os_str().is_empty() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(Error(
            "empty paths and parent traversal are not allowed".into(),
        ));
    }
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|e| Error(format!("current directory: {e}")))?
            .join(path)
    };
    let mut resolved = PathBuf::new();
    for part in absolute.components() {
        if matches!(part, Component::CurDir) {
            continue;
        }
        if output
            && part
                .as_os_str()
                .to_str()
                .is_some_and(|s| s.eq_ignore_ascii_case(".orr"))
        {
            return Err(Error(
                "output cannot be inside an .orr package store".into(),
            ));
        }
        resolved.push(part.as_os_str());
        if matches!(part, Component::Prefix(_)) {
            continue;
        }
        match fs::symlink_metadata(&resolved) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(Error(
                    "symlink ancestry or destination is not allowed".into(),
                ))
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound && output && resolved == absolute => {}
            Err(e) => return Err(Error(format!("path metadata: {e}"))),
        }
    }
    Ok(resolved)
}

fn same_file(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        // std exposes no portable stable file ID. These are consistency checks,
        // not proof of identity; trusted non-mutating input ancestry is required.
        a.len() == b.len() && a.modified().ok() == b.modified().ok()
    }
}

/// Verify a regular file before open, verify the opened handle again, and retain
/// at most expected+1 bytes. Exact length is required; no pipes or stdin route.
pub fn read_raw16(input: &Path, recipe: &Recipe) -> Result<Vec<u8>> {
    let expected = recipe.validate()?;
    let path = checked_path(input, false)?;
    let before = fs::symlink_metadata(&path).map_err(|e| Error(format!("input metadata: {e}")))?;
    if !before.file_type().is_file() || before.len() != expected as u64 {
        return Err(Error(format!(
            "input must be a regular file of exactly {expected} bytes"
        )));
    }
    let file = File::open(&path).map_err(|e| Error(format!("input open: {e}")))?;
    let opened = file
        .metadata()
        .map_err(|e| Error(format!("opened input metadata: {e}")))?;
    let current = fs::symlink_metadata(&path).map_err(|e| Error(format!("input recheck: {e}")))?;
    if !opened.is_file()
        || !current.file_type().is_file()
        || opened.len() != expected as u64
        || !same_file(&before, &opened)
        || !same_file(&opened, &current)
    {
        return Err(Error(
            "input changed during open; use a trusted stable source directory".into(),
        ));
    }
    let mut limited = file.take(expected as u64 + 1);
    let mut bytes = Vec::with_capacity(expected + 1);
    limited
        .read_to_end(&mut bytes)
        .map_err(|e| Error(format!("input read: {e}")))?;
    let after = limited
        .get_ref()
        .metadata()
        .map_err(|e| Error(format!("input final metadata: {e}")))?;
    if bytes.len() != expected
        || after.len() != expected as u64
        || opened.modified().ok() != after.modified().ok()
    {
        return Err(Error(
            "input length or modification time changed during read".into(),
        ));
    }
    Ok(bytes)
}

fn output_path(output: &Path) -> Result<PathBuf> {
    let name = output.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if name.is_empty()
        || name.len() > 100
        || output
            .extension()
            .is_none_or(|extension| extension != "orrt")
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        || ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
    {
        return Err(Error("output basename must be a portable ASCII .orrt filename (letters, digits, -, _, .; no reserved device names; at most 100 bytes)".into()));
    }
    let path = checked_path(output, true)?;
    let parent = path
        .parent()
        .ok_or_else(|| Error("output must have an existing parent".into()))?;
    if path.file_name().is_none()
        || !fs::metadata(parent)
            .map_err(|e| Error(format!("output parent: {e}")))?
            .is_dir()
    {
        return Err(Error(
            "output parent must be an existing trusted directory".into(),
        ));
    }
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(path),
        Ok(_) => Err(Error(
            "output already exists; no overwrite or --force is supported".into(),
        )),
        Err(e) => Err(Error(format!("output metadata: {e}"))),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Write,
    SyncFile,
    Readback,
    Link,
    Cleanup,
    SyncDirectory,
}

fn directory_sync(parent: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(parent)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = parent;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "parent-directory sync is unavailable on this platform",
        ))
    }
}

fn publish_with(
    output: &Path,
    terrain: &Terrain,
    mut before: impl FnMut(Step, &Path, &Path) -> io::Result<()>,
) -> Result<Published> {
    let destination = output_path(output)?;
    let parent = destination.parent().expect("checked parent");
    let mut created = None;
    for _ in 0..32 {
        let id = STAGE_ID.fetch_add(1, Ordering::Relaxed);
        let stage = parent.join(format!(".orr-raw16-{}-{id}.tmp", std::process::id()));
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&stage)
        {
            Ok(file) => {
                created = Some((stage, file));
                break;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(Error(format!(
                    "output stage creation failed before publication: {e}"
                )))
            }
        }
    }
    let (stage, mut file) = created
        .ok_or_else(|| Error("output stage name budget exhausted before publication".into()))?;
    let bytes = terrain.cook();
    let preparation = (|| -> io::Result<()> {
        before(Step::Write, &stage, &destination)?;
        file.write_all(&bytes)?;
        before(Step::SyncFile, &stage, &destination)?;
        file.sync_all()?;
        before(Step::Readback, &stage, &destination)?;
        file.seek(SeekFrom::Start(0))?;
        let mut readback = Vec::with_capacity(bytes.len() + 1);
        (&mut file)
            .take(bytes.len() as u64 + 1)
            .read_to_end(&mut readback)?;
        if readback != bytes || Terrain::load(&readback).ok().as_ref() != Some(terrain) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "stage readback differs from canonical terrain",
            ));
        }
        Ok(())
    })();
    drop(file);
    let commit = preparation.and_then(|()| {
        before(Step::Link, &stage, &destination)?;
        fs::hard_link(&stage, &destination)
    });
    let cleanup =
        before(Step::Cleanup, &stage, &destination).and_then(|()| fs::remove_file(&stage));
    if let Err(error) = commit {
        let leftover = cleanup
            .err()
            .map(|e| format!("; owned stage {stage:?} cleanup failed: {e}"))
            .unwrap_or_default();
        return Err(Error(format!("not published: {error}; hard-link no-replace publication is required, no replace/copy fallback{leftover}")));
    }
    // Link already committed. Nothing below may remove or rewrite destination.
    let cleanup_warning = cleanup
        .err()
        .map(|e| format!("published, but owned stage {stage:?} cleanup failed: {e}"));
    let durability_warning = before(Step::SyncDirectory, &stage, &destination)
        .and_then(|()| directory_sync(parent))
        .err()
        .map(|e| format!("published, but parent-directory durability is uncertain: {e}"));
    Ok(Published {
        revision: terrain.revision(),
        bytes: bytes.len(),
        durability_warning,
        cleanup_warning,
    })
}

/// Convert a retained RAW16 snapshot and publish one new .orrt. Existing input
/// and destinations (including dangling links/aliases) are never overwritten.
/// Err means no destination was created by this call. Ok can carry post-commit
/// durability/cleanup warnings: never retry to overwrite a published result.
pub fn import_file(input: &Path, output: &Path, recipe: &Recipe) -> Result<Published> {
    recipe.validate()?;
    // Fail fast on input aliases, store paths and existing output before reading.
    output_path(output)?;
    let bytes = read_raw16(input, recipe)?;
    let terrain = import_raw16(&bytes, recipe)?;
    publish_with(output, &terrain, |_, _, _| Ok(()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Terrain {
        Terrain::new(
            "fixture".into(),
            2,
            2,
            [orr_fp::FP::ZERO; 2],
            orr_fp::FP::ONE,
            vec![orr_fp::FP::ZERO; 4],
            vec![false],
        )
        .unwrap()
    }
    fn temp() -> tempfile::TempDir {
        tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap()
    }
    #[test]
    fn failures_before_link_leave_destination_absent_and_remove_owned_stage() {
        for fault in [Step::Write, Step::SyncFile, Step::Readback, Step::Link] {
            let root = temp();
            let target = root.path().join("out.orrt");
            let error = publish_with(&target, &fixture(), |step, stage, _| {
                if step == fault {
                    if step == Step::Write {
                        fs::write(stage, b"partial")?;
                    }
                    return Err(io::Error::other("injected failure"));
                }
                Ok(())
            })
            .unwrap_err();
            assert!(error.0.contains("not published"));
            assert!(!target.exists());
            assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
        }
    }
    #[test]
    fn corrupt_stage_readback_is_not_published() {
        let root = temp();
        let target = root.path().join("out.orrt");
        assert!(publish_with(&target, &fixture(), |step, stage, _| {
            if step == Step::Readback {
                fs::write(stage, b"corrupted")?;
            }
            Ok(())
        })
        .is_err());
        assert!(!target.exists());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
    #[test]
    fn concurrent_destination_creation_is_never_replaced() {
        let root = temp();
        let target = root.path().join("out.orrt");
        assert!(publish_with(&target, &fixture(), |step, _, destination| {
            if step == Step::Link {
                fs::write(destination, b"other writer")?;
            }
            Ok(())
        })
        .is_err());
        assert_eq!(fs::read(&target).unwrap(), b"other writer");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
    #[test]
    fn late_cleanup_failure_keeps_exact_published_bytes_and_one_owned_stage() {
        let root = temp();
        let target = root.path().join("out.orrt");
        let terrain = fixture();
        let outcome = publish_with(&target, &terrain, |step, _, _| {
            if step == Step::Cleanup {
                return Err(io::Error::other("injected cleanup failure"));
            }
            Ok(())
        })
        .unwrap();
        assert!(outcome.cleanup_warning.is_some());
        assert_eq!(fs::read(&target).unwrap(), terrain.cook());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    }
    #[test]
    fn late_directory_sync_failure_keeps_published_result() {
        let root = temp();
        let target = root.path().join("out.orrt");
        let terrain = fixture();
        let outcome = publish_with(&target, &terrain, |step, _, _| {
            if step == Step::SyncDirectory {
                return Err(io::Error::other("injected directory sync failure"));
            }
            Ok(())
        })
        .unwrap();
        assert!(outcome.durability_warning.is_some());
        assert!(outcome.cleanup_warning.is_none());
        assert_eq!(fs::read(target).unwrap(), terrain.cook());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
    #[test]
    fn failed_commit_and_cleanup_report_owned_leftover_without_destination() {
        let root = temp();
        let target = root.path().join("out.orrt");
        let error = publish_with(&target, &fixture(), |step, _, _| {
            if step == Step::Link || step == Step::Cleanup {
                return Err(io::Error::other("injected"));
            }
            Ok(())
        })
        .unwrap_err();
        assert!(error.0.contains("owned stage"));
        assert!(!target.exists());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
}
