//! Fixed twelve-file, descriptor-relative visible publication; no recursive cleanup.
use super::{Report, CUES};
use crate::authoring_io::{directory, error, read_at, recheck_directory, same, DIR_FLAGS};
use rustix::fs::{self as unix, AtFlags, Mode, OFlags};
use std::{
    collections::BTreeSet,
    ffi::OsStr,
    fs::{self, File},
    io::Write,
    path::Path,
};
fn absent(parent: &File, name: &OsStr) -> Result<(), String> {
    match unix::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Err(rustix::io::Errno::NOENT) => Ok(()),
        Ok(_) => Err("output already exists; choose a new directory".into()),
        Err(e) => Err(error(e)),
    }
}
pub(super) fn preflight(output: &Path) -> Result<(), String> {
    let p = output.parent().ok_or("output parent missing")?;
    let fd = directory(p)?;
    absent(&fd, output.file_name().ok_or("output filename missing")?)?;
    recheck_directory(p, &fd)
}
pub(super) fn publish(output: &Path, files: &[(String, Vec<u8>)]) -> Result<Report, String> {
    publish_with(output, files, |_, _, _| Ok(()))
}
fn publish_with(
    output: &Path,
    files: &[(String, Vec<u8>)],
    mut checkpoint: impl FnMut(&str, usize, &Path) -> Result<(), String>,
) -> Result<Report, String> {
    let expected: BTreeSet<_> = CUES
        .iter()
        .map(|cue| format!("{cue}.pcm"))
        .chain(["bank.json".into(), "LICENSE.txt".into()])
        .collect();
    let actual: BTreeSet<_> = files.iter().map(|(name, _)| name.clone()).collect();
    if files.len() != 12
        || actual != expected
        || files.iter().any(|(name, bytes)| {
            bytes.len()
                > if name == "bank.json" {
                    128 * 1024
                } else if name == "LICENSE.txt" {
                    64 * 1024
                } else {
                    96008
                }
        })
    {
        return Err("invalid fixed soundbank file set/bounds".into());
    }
    let parent_path = output.parent().ok_or("output parent missing")?;
    let name = output.file_name().ok_or("output filename missing")?;
    let parent = directory(parent_path)?;
    absent(&parent, name)?;
    checkpoint("before-create", 0, output)?;
    recheck_directory(parent_path, &parent)?;
    unix::mkdirat(
        &parent,
        name,
        Mode::RUSR | Mode::WUSR | Mode::XUSR | Mode::RGRP | Mode::XGRP | Mode::ROTH | Mode::XOTH,
    )
    .map_err(error)?;
    let result = (|| -> Result<File, String> {
        let dir = File::from(unix::openat(&parent, name, DIR_FLAGS, Mode::empty()).map_err(error)?);
        for (index, (file, bytes)) in files.iter().enumerate() {
            let mut out = File::from(
                unix::openat(
                    &dir,
                    file.as_str(),
                    OFlags::WRONLY
                        | OFlags::CREATE
                        | OFlags::EXCL
                        | OFlags::NOFOLLOW
                        | OFlags::CLOEXEC,
                    Mode::RUSR | Mode::WUSR | Mode::RGRP | Mode::ROTH,
                )
                .map_err(error)?,
            );
            out.write_all(bytes).map_err(error)?;
            out.sync_all().map_err(error)?;
            checkpoint("file-written", index, output)?;
        }
        checkpoint("before-verify", 0, output)?;
        recheck_directory(parent_path, &parent)?;
        let current =
            File::from(unix::openat(&parent, name, DIR_FLAGS, Mode::empty()).map_err(error)?);
        if !same(
            &current.metadata().map_err(error)?,
            &dir.metadata().map_err(error)?,
        ) {
            return Err("output directory identity changed".into());
        }
        // Trusted/quiescent ancestry is required, but even an injected foreign name
        // is not deleted or accepted as this bank's completed file inventory.
        let observed: BTreeSet<_> = fs::read_dir(output)
            .map_err(error)?
            .map(|entry| entry.map(|e| e.file_name()))
            .collect::<Result<_, _>>()
            .map_err(error)?;
        let expected_os: BTreeSet<_> = expected.iter().map(std::ffi::OsString::from).collect();
        if observed != expected_os {
            return Err("output file inventory changed".into());
        }
        for (file, bytes) in files {
            if read_at(&dir, OsStr::new(file), bytes.len())? != *bytes {
                return Err("output bytes changed".into());
            }
        }
        recheck_directory(output, &dir)?;
        Ok(dir)
    })();
    let dir=match result{Ok(v)=>v,Err(e)=>return Err(format!("{e}; claimed partial output may remain: inspect manually, no retry/overwrite; no files were removed"))};
    // All twelve bytes are verified. A later sync error cannot promise absence.
    let sync = (|| -> Result<(), String> {
        checkpoint("before-directory-sync", 0, output)?;
        dir.sync_all().map_err(error)?;
        checkpoint("before-parent-sync", 0, output)?;
        parent.sync_all().map_err(error)
    })();
    Ok(Report {
        output: output.to_owned(),
        durability_warning: sync.err(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn files() -> Vec<(String, Vec<u8>)> {
        CUES.iter()
            .map(|s| (format!("{s}.pcm"), vec![1]))
            .chain([
                ("bank.json".into(), vec![2]),
                ("LICENSE.txt".into(), vec![3]),
            ])
            .collect()
    }
    #[test]
    fn fixed_inventory_and_no_overwrite() {
        let t = tempfile::tempdir().unwrap();
        let out = t.path().join("bank");
        let mut f = files();
        f[0].0 = "foreign".into();
        assert!(publish(&out, &f).is_err());
        assert!(!out.exists());
        let f = files();
        assert!(publish(&out, &f).unwrap().durability_warning.is_none());
        assert!(publish(&out, &f).is_err());
        assert_eq!(fs::read(out.join("bank.json")).unwrap(), vec![2]);
    }
    #[test]
    fn failure_preserves_partial_and_foreign_files() {
        let t = tempfile::tempdir().unwrap();
        let out = t.path().join("bank");
        let e = publish_with(&out, &files(), |phase, _, p| {
            if phase == "file-written" {
                fs::write(p.join("foreign"), b"keep").map_err(error)?;
                return Err("injected write failure".into());
            }
            Ok(())
        })
        .unwrap_err();
        assert!(e.contains("partial output"));
        assert_eq!(fs::read(out.join("foreign")).unwrap(), b"keep");
        assert_eq!(fs::read(out.join("attack-light.pcm")).unwrap(), vec![1]);
        assert!(publish(&out, &files()).is_err());
    }
    #[test]
    fn completed_sync_failure_is_uncertain_not_absent() {
        for point in ["before-directory-sync", "before-parent-sync"] {
            let t = tempfile::tempdir().unwrap();
            let out = t.path().join("bank");
            let r = publish_with(&out, &files(), |p, _, _| {
                if p == point {
                    Err("injected sync failure".into())
                } else {
                    Ok(())
                }
            })
            .unwrap();
            assert!(r.durability_warning.is_some());
            assert_eq!(fs::read_dir(&out).unwrap().count(), 12);
            assert!(publish(&out, &files()).is_err());
        }
    }
    #[test]
    fn injected_replacement_is_preserved() {
        let t = tempfile::tempdir().unwrap();
        let out = t.path().join("bank");
        let old = t.path().join("claimed-old");
        let e = publish_with(&out, &files(), |phase, _, p| {
            if phase == "before-verify" {
                fs::rename(p, &old).map_err(error)?;
                fs::create_dir(p).map_err(error)?;
                fs::write(p.join("foreign"), b"keep").map_err(error)?;
            }
            Ok(())
        })
        .unwrap_err();
        assert!(e.contains("identity changed"));
        assert_eq!(fs::read(out.join("foreign")).unwrap(), b"keep");
        assert_eq!(fs::read_dir(old).unwrap().count(), 12);
    }
}
