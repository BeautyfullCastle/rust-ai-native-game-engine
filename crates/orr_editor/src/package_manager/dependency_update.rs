//! One existing unused direct package revision with explicit local dependency candidates.
//! Observations require trusted quiescent ancestry, not a filesystem sandbox.
use super::removal::{admit_candidate, read_regular, root_identity, RootIdentity, Snapshot};
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
pub struct DependencyUpdatePlan {
    root: PathBuf,
    identity: RootIdentity,
    manifest: Vec<u8>,
    sources: Vec<Source>, // primary first, including all unused candidates
    preview: PackagePreview,
    unused: Vec<String>,
    snapshot: Snapshot,
}
impl DependencyUpdatePlan {
    pub fn name(&self) -> &str {
        &self.sources[0].package.manifest.name
    }
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
fn unchanged_sources(plan: &DependencyUpdatePlan) -> Result<(), String> {
    let runtime = room_runtime();
    for source in &plan.sources {
        if checked_directory(&source.path, "package source")? != source.path
            || root_identity(&source.path)? != source.identity
            || read_regular(&source.path.join("orr.package.json"), MAX_MANIFEST_BYTES)?
                != source.manifest
            || inspect_source(&source.path, &runtime)? != source.package
        {
            return Err(
                "Dependency update source changed since preview (including unused candidates)"
                    .into(),
            );
        }
    }
    Ok(())
}
fn unchanged_room(plan: &DependencyUpdatePlan) -> Result<(), String> {
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
    if admit_candidate(&plan.root, &plan.preview.previous, &plan.preview.candidate)?
        != plan.snapshot
    {
        return Err("Saved Room content changed since dependency update preview".into());
    }
    Ok(())
}
fn validate_source(previous: &Lock, primary: &LockedPackage) -> Result<(), String> {
    let name = &primary.manifest.name;
    let old = previous
        .packages
        .get(name)
        .ok_or("Update requires an installed direct package")?;
    if previous.direct.get(name) != Some(&old.manifest.version) {
        return Err("Update requires an installed direct package".into());
    }
    if old.manifest.version == primary.manifest.version {
        return Err("Update requires a different exact version".into());
    }
    for (name, version) in &old.manifest.dependencies {
        if primary.manifest.dependencies.get(name) != Some(version) {
            return Err(
                "Dependency update cannot remove or change an existing exact dependency".into(),
            );
        }
    }
    Ok(())
}
fn validate(plan: &DependencyUpdatePlan) -> Result<(), String> {
    let previous = &plan.preview.previous;
    let next = &plan.preview.candidate;
    let primary = &plan.sources[0].package;
    validate_source(previous, primary)?;
    if next.schema != previous.schema
        || next.direct.keys().ne(previous.direct.keys())
        || next.direct.get(&primary.manifest.name) != Some(&primary.manifest.version)
    {
        return Err("Dependency update must preserve all direct selection names".into());
    }
    for (name, version) in &previous.direct {
        if name != &primary.manifest.name && next.direct.get(name) != Some(version) {
            return Err("Existing direct selection changed".into());
        }
    }
    for (name, package) in &previous.packages {
        if name != &primary.manifest.name && next.packages.get(name) != Some(package) {
            return Err("Existing package identity changed".into());
        }
    }
    for (name, package) in &next.packages {
        if !previous.packages.contains_key(name)
            && !plan.sources.iter().any(|source| &source.package == package)
        {
            return Err("Unexpected package in dependency update".into());
        }
    }
    if next.packages.get(&primary.manifest.name) != Some(primary) {
        return Err("Primary package differs from inspected source".into());
    }
    Ok(())
}
pub fn plan_update_with_dependencies(
    root: &Path,
    primary: &Path,
    candidates: &[PathBuf],
) -> Result<DependencyUpdatePlan, String> {
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
            || (!sources.is_empty() && previous.packages.contains_key(&package.manifest.name))
        {
            return Err("Duplicate or already active package name in dependency update".into());
        }
        sources.push(Source {
            path,
            identity,
            manifest,
            package,
        });
    }
    validate_source(&previous, &sources[0].package)?;
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
    let snapshot = admit_candidate(&root, &preview.previous, &preview.candidate)?;
    let plan = DependencyUpdatePlan {
        root,
        identity,
        manifest,
        sources,
        preview,
        unused,
        snapshot,
    };
    validate(&plan)?;
    unchanged_sources(&plan)?;
    unchanged_room(&plan)?;
    Ok(plan)
}
pub fn apply_update_with_dependencies(plan: &DependencyUpdatePlan) -> Result<Lock, String> {
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
    let after = admit_candidate(&plan.root, &verified, &verified).map_err(|e| format!("Packages were installed but saved Room verification failed; published lock retained: {e}"))?;
    if after != plan.snapshot {
        return Err(
            "Packages were installed but saved Room content changed; published lock retained"
                .into(),
        );
    }
    Ok(verified)
}
#[cfg(test)]
#[path = "dependency_update/tests.rs"]
mod tests;
