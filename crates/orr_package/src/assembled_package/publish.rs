//! Linux no-replace package publication with owned stage cleanup.
//! Requires trusted stable ancestry; held descriptors are not a hostile-user sandbox.
use super::{Metadata, PackageKind, Report};
use crate::authoring_io::{directory, error, read_at, recheck_directory, same, DIR_FLAGS};
use rustix::fs::{self as unix, AtFlags, Mode, OFlags, RenameFlags};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::Write,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
#[cfg(test)]
const MODEL: &str = "model.model.json";
const LICENSE: &str = "LICENSE.txt";
const MANIFEST: &str = "orr.package.json";
static SERIAL: AtomicU64 = AtomicU64::new(0);
fn absolute(path: &Path) -> Result<PathBuf, String> {
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir().map_err(error)?.join(path)
    };
    if path.as_os_str().as_encoded_bytes().len() > 4096
        || path.components().count() > 128
        || path.components().any(|p| matches!(p, Component::ParentDir))
        || path.components().any(|p| p.as_os_str() == ".orr")
        || path.as_os_str().as_encoded_bytes().contains(&0)
    {
        return Err("unsafe or oversized path; traversal and .orr are refused".into());
    }
    Ok(path)
}
fn absent(parent: &File, name: &std::ffi::OsStr) -> Result<(), String> {
    match unix::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Err(rustix::io::Errno::NOENT) => Ok(()),
        Ok(_) => Err("output already exists; choose a new directory".into()),
        Err(e) => Err(error(e)),
    }
}
pub(super) fn snapshot(path: &Path, max: usize) -> Result<Vec<u8>, String> {
    if max > 64 * 1024 * 1024 {
        return Err("snapshot bound exceeds 64 MiB".into());
    }
    let path = absolute(path)?;
    let parent_path = path.parent().ok_or("input parent missing")?;
    let name = path.file_name().ok_or("input filename missing")?;
    let parent = directory(parent_path)?;
    let bytes = read_at(&parent, name, max)?;
    recheck_directory(parent_path, &parent)?;
    Ok(bytes)
}
fn write_at(directory: &File, name: &str, bytes: &[u8]) -> Result<(), String> {
    let mut file = File::from(
        unix::openat(
            directory,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR | Mode::RGRP | Mode::ROTH,
        )
        .map_err(error)?,
    );
    file.write_all(bytes).map_err(error)?;
    file.sync_all().map_err(error)
}
fn cleanup(
    parent_path: &Path,
    parent: &File,
    name: &str,
    stage: &File,
    names: [&str; 3],
) -> Result<(), String> {
    recheck_directory(parent_path, parent)?;
    let current = File::from(unix::openat(parent, name, DIR_FLAGS, Mode::empty()).map_err(error)?);
    if !same(
        &current.metadata().map_err(error)?,
        &stage.metadata().map_err(error)?,
    ) {
        return Err("stage identity changed; refusing cleanup".into());
    }
    for name in names {
        match unix::unlinkat(stage, name, AtFlags::empty()) {
            Ok(()) | Err(rustix::io::Errno::NOENT) => {}
            Err(e) => return Err(error(e)),
        }
    }
    unix::unlinkat(parent, name, AtFlags::REMOVEDIR).map_err(error)
}
pub(super) fn preflight_output(output: &Path) -> Result<(), String> {
    let output = absolute(output)?;
    let parent_path = output.parent().ok_or("output parent missing")?;
    let parent = directory(parent_path)?;
    absent(
        &parent,
        output.file_name().ok_or("output filename missing")?,
    )?;
    recheck_directory(parent_path, &parent)
}
pub(super) fn publish(
    output: &Path,
    model: &[u8],
    license: &[u8],
    metadata: &Metadata,
) -> Result<Report, String> {
    publish_kind(output, model, license, metadata, PackageKind::Static)
}
pub(super) fn publish_kind(
    output: &Path,
    model: &[u8],
    license: &[u8],
    metadata: &Metadata,
    kind: PackageKind,
) -> Result<Report, String> {
    publish_with_kind(output, model, license, metadata, kind, |_, _| Ok(()))
}
#[cfg(test)]
fn publish_with(
    output: &Path,
    model: &[u8],
    license: &[u8],
    metadata: &Metadata,
    checkpoint: impl FnMut(&str, &Path) -> Result<(), String>,
) -> Result<Report, String> {
    publish_with_kind(
        output,
        model,
        license,
        metadata,
        PackageKind::Static,
        checkpoint,
    )
}
fn publish_with_kind(
    output: &Path,
    model: &[u8],
    license: &[u8],
    metadata: &Metadata,
    kind: PackageKind,
    mut checkpoint: impl FnMut(&str, &Path) -> Result<(), String>,
) -> Result<Report, String> {
    let model_name = kind.model_name();
    let names = [model_name, LICENSE, MANIFEST];
    if model.len() > 64 * 1024 * 1024 || license.len() > 1024 * 1024 {
        return Err("model/license exceeds package bound".into());
    }
    let output = absolute(output)?;
    let parent_path = output.parent().ok_or("output parent missing")?;
    let target = output.file_name().ok_or("output filename missing")?;
    let parent = directory(parent_path)?;
    absent(&parent, target)?;
    let mut allocated = None;
    for _ in 0..128 {
        let name = format!(
            ".orr-assembled-stage-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        );
        checkpoint("stage-candidate", &parent_path.join(&name))?;
        match unix::mkdirat(&parent, name.as_str(), Mode::RUSR | Mode::WUSR | Mode::XUSR) {
            Ok(()) => {
                allocated = Some(name);
                break;
            }
            Err(rustix::io::Errno::EXIST) => continue,
            Err(e) => return Err(error(e)),
        }
    }
    let name = allocated.ok_or("could not allocate private stage")?;
    let stage_path = parent_path.join(&name);
    let stage = File::from(
        unix::openat(&parent, name.as_str(), DIR_FLAGS, Mode::empty()).map_err(|e| {
            format!(
                "created stage could not be opened; inspect {}: {e}",
                stage_path.display()
            )
        })?,
    );
    let mut published = false;
    let result = (|| -> Result<(), String> {
        checkpoint("stage-created", &stage_path)?;
        recheck_directory(&stage_path, &stage)?;
        write_at(&stage, model_name, model)?;
        write_at(&stage, LICENSE, license)?;
        recheck_directory(parent_path, &parent)?;
        recheck_directory(&stage_path, &stage)?;
        crate::source_authoring::create(&crate::source_authoring::Options {
            source: stage_path.clone(),
            name: metadata.name.clone(),
            version: metadata.version.clone(),
            engine: metadata.engine.clone(),
            capabilities: kind.capabilities().into_iter().collect(),
            dependencies: vec![],
            files: vec![model_name.into(), LICENSE.into()],
        })?;
        checkpoint("manifest-created", &stage_path)?;
        recheck_directory(&stage_path, &stage)?;
        let actual = fs::read_dir(&stage_path)
            .map_err(error)?
            .map(|entry| entry.map(|e| e.file_name()).map_err(error))
            .collect::<Result<BTreeSet<_>, _>>()?;
        let expected = names
            .into_iter()
            .map(std::ffi::OsString::from)
            .collect::<BTreeSet<_>>();
        if actual != expected {
            return Err("unexpected staged package inventory".into());
        }
        if read_at(&stage, model_name.as_ref(), 64 * 1024 * 1024)? != model
            || read_at(&stage, LICENSE.as_ref(), 1024 * 1024)? != license
        {
            return Err("staged model/license changed".into());
        }
        let manifest = read_at(&stage, MANIFEST.as_ref(), 1024 * 1024)?;
        let decoded: crate::Manifest = serde_json::from_slice(&manifest).map_err(error)?;
        // Bind admission to the request, not merely to a self-consistent staged file.
        // Source authoring preserves these strings; admission enforces canonical exact
        // versions and parses engine requirements without rewriting their spelling.
        let expected_manifest = crate::Manifest {
            schema: 1,
            name: metadata.name.clone(),
            version: metadata.version.clone(),
            engine: metadata.engine.clone(),
            capabilities: kind.capabilities(),
            dependencies: Default::default(),
            files: [model_name.to_owned(), LICENSE.to_owned()]
                .into_iter()
                .collect(),
        };
        if decoded != expected_manifest {
            return Err("staged package manifest differs from requested metadata".into());
        }
        let inspected =
            crate::inspect_for_install(&stage_path, crate::Runtime::content_only().engine_version)
                .map_err(error)?;
        if inspected.manifest != decoded {
            return Err("staged package inspection mismatch".into());
        }
        for name in names {
            File::from(
                unix::openat(
                    &stage,
                    name,
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(error)?,
            )
            .sync_all()
            .map_err(error)?;
        }
        stage.sync_all().map_err(error)?;
        checkpoint("before-rename", &stage_path)?;
        recheck_directory(parent_path, &parent)?;
        recheck_directory(&stage_path, &stage)?;
        if read_at(&stage, model_name.as_ref(), 64 * 1024 * 1024)? != model
            || read_at(&stage, LICENSE.as_ref(), 1024 * 1024)? != license
            || read_at(&stage, MANIFEST.as_ref(), 1024 * 1024)? != manifest
        {
            return Err("staged bytes changed before publication".into());
        }
        let final_inventory = fs::read_dir(&stage_path)
            .map_err(error)?
            .map(|entry| entry.map(|e| e.file_name()).map_err(error))
            .collect::<Result<BTreeSet<_>, _>>()?;
        if final_inventory != expected {
            return Err("staged inventory changed before publication".into());
        }
        unix::renameat_with(
            &parent,
            name.as_str(),
            &parent,
            target,
            RenameFlags::NOREPLACE,
        )
        .map_err(error)?;
        published = true;
        checkpoint("after-rename", &output)?;
        recheck_directory(parent_path, &parent)?;
        recheck_directory(&output, &stage)?;
        parent.sync_all().map_err(error)?;
        Ok(())
    })();
    match result {
        Ok(()) => Ok(Report {
            output,
            durability_warning: None,
        }),
        Err(e) if published => Ok(Report {
            output,
            durability_warning: Some(format!("published; durability uncertain: {e}")),
        }),
        Err(e) => match cleanup(parent_path, &parent, &name, &stage, names) {
            Ok(()) => Err(e),
            Err(cleanup) => Err(format!(
                "{e}; cleanup incomplete: {cleanup}; inspect {}",
                stage_path.display()
            )),
        },
    }
}
#[cfg(test)]
#[path = "publish_tests.rs"]
mod tests;
