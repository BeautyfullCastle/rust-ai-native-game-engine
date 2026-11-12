//! One new direct package with explicit local dependency candidates.
//! Observations require trusted quiescent ancestry, not a filesystem sandbox.
use super::removal::{read_regular, root_identity, RootIdentity};
use super::{checked_directory, inspect_source, open_room, room_runtime, MAX_MANIFEST_BYTES};
use orr_package::{Lock, LockedPackage, PackagePreview, Project};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

struct Source {
    path: PathBuf,
    identity: RootIdentity,
    manifest: Vec<u8>,
    package: LockedPackage,
}
pub struct DependencyAddPlan {
    root: PathBuf,
    identity: RootIdentity,
    manifest: Vec<u8>,
    sources: Vec<Source>, // primary first, including all unused candidates
    preview: PackagePreview,
    unused: Vec<String>,
}
impl DependencyAddPlan {
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn preview(&self) -> &PackagePreview {
        &self.preview
    }
    pub fn unused(&self) -> &[String] {
        &self.unused
    }
}
fn unchanged_sources(plan: &DependencyAddPlan) -> Result<(), String> {
    let runtime = room_runtime();
    for source in &plan.sources {
        if checked_directory(&source.path, "package source")? != source.path
            || root_identity(&source.path)? != source.identity
            || read_regular(&source.path.join("orr.package.json"), MAX_MANIFEST_BYTES)?
                != source.manifest
            || inspect_source(&source.path, &runtime)? != source.package
        {
            return Err(
                "Dependency addition source changed since preview (including unused candidates)"
                    .into(),
            );
        }
    }
    Ok(())
}
fn unchanged_room(plan: &DependencyAddPlan) -> Result<(), String> {
    if checked_directory(&plan.root, "Room project")? != plan.root
        || root_identity(&plan.root)? != plan.identity
        || read_regular(&plan.root.join("orr.project.json"), MAX_MANIFEST_BYTES)? != plan.manifest
    {
        return Err("Room project identity or manifest changed since dependency preview".into());
    }
    let (root, lock) = open_room(&plan.root)?;
    if root != plan.root || lock != plan.preview.previous {
        return Err("Room package lock changed since dependency preview".into());
    }
    Ok(())
}
fn validate(plan: &DependencyAddPlan) -> Result<(), String> {
    let previous = &plan.preview.previous;
    let next = &plan.preview.candidate;
    let primary = &plan.sources[0].package;
    if next.schema != previous.schema
        || next.direct.len() != previous.direct.len() + 1
        || next.direct.get(&primary.manifest.name) != Some(&primary.manifest.version)
    {
        return Err("Dependency addition must select exactly one new direct package".into());
    }
    for (name, version) in &previous.direct {
        if next.direct.get(name) != Some(version) {
            return Err("Existing direct selection changed".into());
        }
    }
    for (name, package) in &previous.packages {
        if next.packages.get(name) != Some(package) {
            return Err("Existing package identity changed".into());
        }
    }
    for (name, package) in &next.packages {
        if !previous.packages.contains_key(name)
            && !plan.sources.iter().any(|source| &source.package == package)
        {
            return Err("Unexpected package in dependency addition".into());
        }
    }
    if next.packages.get(&primary.manifest.name) != Some(primary) {
        return Err("Primary package differs from inspected source".into());
    }
    Ok(())
}
pub fn plan_add_with_dependencies(
    root: &Path,
    primary: &Path,
    candidates: &[PathBuf],
) -> Result<DependencyAddPlan, String> {
    if candidates.is_empty() || candidates.len() > 8 {
        return Err("Supply 1..8 explicit dependency directories".into());
    }
    let (root, previous) = open_room(root)?;
    let identity = root_identity(&root)?;
    let manifest = read_regular(&root.join("orr.project.json"), MAX_MANIFEST_BYTES)?;
    let mut paths = BTreeSet::new();
    let mut names = BTreeSet::new();
    let mut sources = Vec::new();
    for path in std::iter::once(primary).chain(candidates.iter().map(PathBuf::as_path)) {
        let path = checked_directory(path, "package source")?;
        if !paths.insert(path.clone()) {
            return Err("Duplicate package source path".into());
        }
        let identity = root_identity(&path)?;
        let manifest = read_regular(&path.join("orr.package.json"), MAX_MANIFEST_BYTES)?;
        let package = inspect_source(&path, &room_runtime())?;
        if !names.insert(package.manifest.name.clone())
            || previous.packages.contains_key(&package.manifest.name)
        {
            return Err("Duplicate or already active package name in dependency addition".into());
        }
        sources.push(Source {
            path,
            identity,
            manifest,
            package,
        });
    }
    let deps: Vec<_> = sources[1..].iter().map(|s| s.path.clone()).collect();
    let preview = Project::open(&root, room_runtime())
        .and_then(|p| {
            p.preview_install_with_dependencies(std::slice::from_ref(&sources[0].path), &deps)
        })
        .map_err(|e| e.to_string())?;
    if preview.previous != previous {
        return Err("Room package lock changed while planning".into());
    }
    let mut unused: Vec<_> = sources[1..]
        .iter()
        .filter(|s| {
            !preview
                .candidate
                .packages
                .contains_key(&s.package.manifest.name)
        })
        .map(|s| s.package.manifest.name.clone())
        .collect();
    unused.sort();
    let plan = DependencyAddPlan {
        root,
        identity,
        manifest,
        sources,
        preview,
        unused,
    };
    validate(&plan)?;
    unchanged_sources(&plan)?;
    unchanged_room(&plan)?;
    Ok(plan)
}
pub fn apply_add_with_dependencies(plan: &DependencyAddPlan) -> Result<Lock, String> {
    validate(plan)?;
    unchanged_sources(plan)?;
    unchanged_room(plan)?;
    let deps: Vec<_> = plan.sources[1..].iter().map(|s| s.path.clone()).collect();
    let installed = Project::open(&plan.root, room_runtime())
        .map_err(|e| e.to_string())?
        .install_checked_with_dependencies(
            std::slice::from_ref(&plan.sources[0].path),
            &deps,
            &plan.preview.previous,
            &plan.preview.candidate,
            || {
                unchanged_sources(plan)?;
                unchanged_room(plan)
            },
        )
        .map_err(|e| e.to_string())?;
    let verified = open_room(&plan.root).map_err(|e| format!("Packages were installed but Room re-admission failed; published lock retained: {e}"))?.1;
    if installed != verified || verified != plan.preview.candidate {
        return Err("Packages were installed but verification differs from approved lock; published lock retained".into());
    }
    Ok(verified)
}
#[cfg(test)]
mod tests;
