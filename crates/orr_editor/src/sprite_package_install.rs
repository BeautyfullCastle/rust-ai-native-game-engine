//! New-name-only sprite package activation through the existing checked writer.
//! Paths require trusted stable ancestry. These checks are not a hostile-race
//! filesystem sandbox; failed commits may retain unreachable immutable objects.
use crate::sprite_bindings;
use orr_package::{Lock, LockedPackage};
use orr_sprite::SpriteDocument;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, Metadata},
    io::Read,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Summary {
    pub name: String,
    pub version: String,
    pub digest: String,
    pub width: u32,
    pub height: u32,
    pub regions: usize,
    pub clips: usize,
    pub files: Vec<String>,
}

/// Only `prepare` constructs approval state; UI code gets immutable views.
#[derive(Debug)]
pub(crate) struct Plan {
    project: PathBuf,
    source: PathBuf,
    previous: Lock,
    candidate: Lock,
    summary: Summary,
}

impl Plan {
    pub(crate) fn summary(&self) -> &Summary {
        &self.summary
    }
    #[cfg(test)]
    pub(crate) fn project(&self) -> &Path {
        &self.project
    }
    #[cfg(test)]
    pub(crate) fn source(&self) -> &Path {
        &self.source
    }
    #[cfg(test)]
    pub(crate) fn previous(&self) -> &Lock {
        &self.previous
    }
    #[cfg(test)]
    pub(crate) fn candidate(&self) -> &Lock {
        &self.candidate
    }
}

pub(crate) fn prepare(project: &Path, source: &Path) -> Result<Plan, String> {
    validate_path(project)?;
    validate_path(source)?;
    checked_ancestry(project)?;
    checked_ancestry(source)?;
    let opened = sprite_bindings::open_project(project)?;
    let runtime = sprite_bindings::compiled_runtime();
    let package = orr_package::inspect(source, &runtime)
        .map_err(|error| format!("sprite package inspection: {error}"))?;
    if package.manifest.capabilities != BTreeSet::from(["sprite".into()])
        || !package.manifest.dependencies.is_empty()
    {
        return Err(
            "sprite installation requires exactly the sprite capability and no dependencies".into(),
        );
    }
    if package.manifest.files.len() != 3
        || !package.manifest.files.contains("sprites.json")
        || !package.manifest.files.contains("LICENSE.txt")
    {
        return Err(
            "sprite package must declare exactly sprites.json, LICENSE.txt and its atlas image"
                .into(),
        );
    }
    let json = read_verified(source, "sprites.json", 4 * 1024 * 1024, &package)?;
    let document = SpriteDocument::from_json(
        std::str::from_utf8(&json).map_err(|error| format!("sprite document UTF-8: {error}"))?,
    )
    .map_err(|error| format!("sprite document: {error}"))?;
    let atlas = document.atlas();
    let expected_files = BTreeSet::from([
        "sprites.json".to_owned(),
        "LICENSE.txt".to_owned(),
        atlas.image.clone(),
    ]);
    if expected_files.len() != 3 || expected_files != package.manifest.files {
        return Err("sprite package files must match sprites.json, LICENSE.txt and the document atlas image exactly".into());
    }
    let png = read_verified(source, &atlas.image, 16 * 1024 * 1024, &package)?;
    let rgba = orr_sample::project_sprites::decode_atlas(&png, atlas.width, atlas.height)
        .map_err(|error| format!("sprite atlas PNG: {error}"))?;
    let expected_rgba = u64::from(atlas.width)
        .checked_mul(u64::from(atlas.height))
        .and_then(|n| n.checked_mul(4))
        .filter(|n| *n <= 16 * 1024 * 1024)
        .ok_or("sprite atlas decoded byte limit exceeded")?;
    if rgba.len() as u64 != expected_rgba {
        return Err("sprite atlas decoded bytes differ from the document dimensions".into());
    }
    drop(rgba);
    let license = read_verified(source, "LICENSE.txt", 64 * 1024, &package)?;
    if std::str::from_utf8(&license)
        .ok()
        .is_none_or(|text| text.trim().is_empty())
    {
        return Err("sprite package license must be nonempty UTF-8 text".into());
    }
    let preview = opened
        .preview_install(&[source.to_path_buf()])
        .map_err(|error| format!("sprite installation preview: {error}"))?;
    let name = &package.manifest.name;
    if preview.previous.direct.contains_key(name) || preview.previous.packages.contains_key(name) {
        return Err("sprite installation requires a new package name; existing selections cannot be replaced".into());
    }
    let mut expected = preview.previous.clone();
    expected
        .direct
        .insert(name.clone(), package.manifest.version.clone());
    expected.packages.insert(name.clone(), package.clone());
    if preview.candidate != expected {
        return Err(
            "sprite installation preview changed content beyond the one new direct package".into(),
        );
    }
    // Bind typed reads and the preview candidate to one exact inspected package.
    // This is read-only and does not reserve a writer or stage cached objects.
    let rechecked = orr_package::inspect(source, &runtime)
        .map_err(|error| format!("sprite package reinspection: {error}"))?;
    if rechecked != package {
        return Err("sprite source changed during preparation; prepare again".into());
    }
    Ok(Plan {
        project: project.to_path_buf(),
        source: source.to_path_buf(),
        previous: preview.previous,
        candidate: preview.candidate,
        summary: Summary {
            name: name.clone(),
            version: package.manifest.version.clone(),
            digest: package.digest.clone(),
            width: atlas.width,
            height: atlas.height,
            regions: document.regions().len(),
            clips: document.clips().len(),
            files: package.manifest.files.iter().cloned().collect(),
        },
    })
}

pub(crate) fn apply(
    plan: &Plan,
    before_publish: impl FnOnce() -> Result<(), String>,
) -> Result<Lock, String> {
    validate_path(&plan.project)?;
    validate_path(&plan.source)?;
    checked_ancestry(&plan.project)?;
    checked_ancestry(&plan.source)?;
    for file in &plan.summary.files {
        checked_ancestry(&plan.source.join(file))?;
    }
    let opened = sprite_bindings::open_project(&plan.project)?;
    // The core rechecks the full previous/candidate locks, snapshots source
    // bytes and runs the caller's live-context check under its writer guard.
    // Do not clean up unreachable objects or retry a refused transaction.
    opened
        .install_checked(
            std::slice::from_ref(&plan.source),
            &plan.previous,
            &plan.candidate,
            before_publish,
        )
        .map_err(|error| format!("sprite installation: {error}"))
}

fn read_verified(
    source: &Path,
    name: &str,
    limit: u64,
    package: &LockedPackage,
) -> Result<Vec<u8>, String> {
    let expected = package
        .files
        .get(name)
        .ok_or("sprite asset is not declared in the inspected package")?;
    let path = source.join(name);
    validate_path(&path)?;
    checked_ancestry(&path)?;
    let metadata = fs::symlink_metadata(&path).map_err(|error| format!("{name}: {error}"))?;
    if !regular(&metadata) || metadata.len() > limit {
        return Err(format!("{name} must be a bounded regular file"));
    }
    let mut file = File::open(&path).map_err(|error| format!("{name}: {error}"))?;
    let opened = file.metadata().map_err(|error| error.to_string())?;
    if !regular(&opened) || opened.len() > limit {
        return Err(format!("{name} changed or exceeds its byte limit"));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > limit || hash(&bytes) != *expected {
        return Err(format!(
            "sprite source asset changed since inspection: {name}"
        ));
    }
    Ok(bytes)
}

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn validate_path(path: &Path) -> Result<(), String> {
    let text = path
        .to_str()
        .ok_or("sprite installation paths must be Unicode")?;
    if !path.is_absolute()
        || text.len() > 4096
        || path.components().count() > 64
        || text.chars().any(char::is_control)
        || text.split(['/', '\\']).any(|s| s == "." || s == "..")
    {
        return Err("sprite installation paths must be bounded, absolute and normalized".into());
    }
    for component in path.components() {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            return Err("sprite installation paths cannot contain traversal".into());
        }
        if let Component::Prefix(prefix) = component {
            if !matches!(
                prefix.kind(),
                std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
            ) {
                return Err("sprite installation requires local filesystem paths".into());
            }
        }
        if let Component::Normal(name) = component {
            let name = name
                .to_str()
                .ok_or("sprite installation paths must be Unicode")?;
            if name.contains([':', '\\']) || name.ends_with(['.', ' ']) {
                return Err("sprite installation path has a nonportable component".into());
            }
        }
    }
    Ok(())
}

fn checked_ancestry(path: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| format!("{}: {error}", current.display()))?;
        if linked(&metadata) {
            return Err("sprite installation refuses linked or reparse ancestry".into());
        }
    }
    Ok(())
}

fn linked(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_type().is_symlink() || metadata.file_attributes() & 0x0400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}
fn regular(metadata: &Metadata) -> bool {
    metadata.is_file() && !linked(metadata)
}

#[cfg(test)]
#[path = "sprite_package_install/tests.rs"]
mod tests;
