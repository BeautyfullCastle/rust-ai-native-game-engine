//! Explicit local revision changes for unassigned direct package selections.
//!
//! The package core owns resolution, object staging and atomic lock publication.
//! This editor boundary narrows that operation to one unchanged dependency graph
//! and pins the complete saved Room admission without editing its content.
use super::{
    checked_directory, inspect_source,
    removal::{admit_candidate, read_regular, root_identity, RootIdentity, Snapshot},
    room_runtime, MAX_MANIFEST_BYTES,
};
use orr_package::{Lock, LockedPackage, PackagePreview, Project};
use std::path::{Path, PathBuf};

/// Opaque evidence of one observed local revision and saved Room state.
/// The report is read-only: callers cannot replace its approved lock or source.
/// Applying still revalidates under the package writer guard. The editor must
/// separately guard current-project identity and unsaved host changes.
pub struct UpdatePlan {
    root: PathBuf,
    source: PathBuf,
    name: String,
    preview: PackagePreview,
    source_identity: RootIdentity,
    source_manifest: Vec<u8>,
    snapshot: Snapshot,
}

impl UpdatePlan {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn source(&self) -> &Path {
        &self.source
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn preview(&self) -> &PackagePreview {
        &self.preview
    }
}

/// Observe a different exact version of an installed direct package. This does
/// not acquire a writer, stage package objects, or change Room files/history.
pub fn plan_update(root: &Path, source: &Path) -> Result<UpdatePlan, String> {
    let root = checked_directory(root, "Room project")?;
    let source = checked_directory(source, "package source")?;
    let source_identity = root_identity(&source)?;
    let source_manifest = read_regular(&source.join("orr.package.json"), MAX_MANIFEST_BYTES)?;
    let runtime = room_runtime();
    let source_package = inspect_source(&source, &runtime)?;
    let project = Project::open(&root, runtime).map_err(|error| error.to_string())?;
    let previous = project.verify().map_err(|error| error.to_string())?;
    validate_source(&previous, &source_package)?;
    let preview = project
        .preview_install(std::slice::from_ref(&source))
        .map_err(|error| error.to_string())?;
    if preview.previous != previous {
        return Err(
            "active Room package lock changed during update planning; preview again".into(),
        );
    }
    validate_update(&preview.previous, &preview.candidate, &source_package)?;
    // Shared admission covers every saved static/animated model binding and
    // rejects any binding whose package identity would change in the candidate.
    let snapshot = admit_candidate(&root, &preview.previous, &preview.candidate)?;
    let plan = UpdatePlan {
        root,
        source,
        name: source_package.manifest.name.clone(),
        preview,
        source_identity,
        source_manifest,
        snapshot,
    };
    unchanged_source(&plan)?;
    Ok(plan)
}

/// Publish only this immutable preview. Existing source files, cached revisions,
/// saved Room content and host history are not edited or deleted. A rejected
/// callback may leave an unreachable immutable object staged by the core.
/// Post-publication uncertainty retains the published lock; never auto-rollback.
pub fn apply_update(plan: &UpdatePlan) -> Result<Lock, String> {
    unchanged_source(plan)?;
    unchanged_room(plan)?;
    let project = Project::open(&plan.root, room_runtime()).map_err(|error| error.to_string())?;
    let installed = project
        .install_checked(
            std::slice::from_ref(&plan.source),
            &plan.preview.previous,
            &plan.preview.candidate,
            || {
                // Core has reverified the complete active lock, re-read source
                // content, compared the candidate and staged its immutable object.
                // These checks run while its existing writer guard is held.
                unchanged_source(plan)?;
                unchanged_room(plan)
            },
        )
        .map_err(|error| error.to_string())?;

    let after = admit_candidate(&plan.root, &installed, &installed).map_err(|error| {
        format!(
            "package revision was updated at {} but Room re-admission failed; the published lock is retained: {error}",
            plan.root.display()
        )
    })?;
    if installed != plan.preview.candidate || after != plan.snapshot {
        return Err(format!(
            "package revision was updated at {} but the approved lock or saved Room content changed after publication; the published lock is retained",
            plan.root.display()
        ));
    }
    Ok(installed)
}

fn unchanged_source(plan: &UpdatePlan) -> Result<(), String> {
    let source = checked_directory(&plan.source, "package source")?;
    if source != plan.source || root_identity(&source)? != plan.source_identity {
        return Err(
            "package source directory identity changed since update planning; preview again".into(),
        );
    }
    let source_package = inspect_source(&source, &room_runtime())?;
    if source_package.manifest.name != plan.name
        || read_regular(&source.join("orr.package.json"), MAX_MANIFEST_BYTES)?
            != plan.source_manifest
    {
        return Err("package source manifest changed since update planning; preview again".into());
    }
    validate_update(
        &plan.preview.previous,
        &plan.preview.candidate,
        &source_package,
    )?;
    if root_identity(&source)? != plan.source_identity
        || checked_directory(&source, "package source")? != source
    {
        return Err(
            "package source directory changed during update admission; preview again".into(),
        );
    }
    Ok(())
}

fn unchanged_room(plan: &UpdatePlan) -> Result<(), String> {
    let current = admit_candidate(&plan.root, &plan.preview.previous, &plan.preview.candidate)?;
    if current != plan.snapshot {
        return Err(
            "saved Room project identity, scene, model bindings or camera changed since update planning; preview again"
                .into(),
        );
    }
    Ok(())
}

fn validate_source(previous: &Lock, source: &LockedPackage) -> Result<(), String> {
    let name = &source.manifest.name;
    let selected = previous
        .direct
        .get(name)
        .ok_or_else(|| format!("update requires an installed direct package: {name}"))?;
    let old = previous
        .packages
        .get(name)
        .ok_or_else(|| format!("installed direct package is missing: {name}"))?;
    if selected != &old.manifest.version {
        return Err("installed direct selection does not match its package version".into());
    }
    if old.manifest.version == source.manifest.version {
        return Err(
            "update requires a different exact version; same-version replacement is not allowed"
                .into(),
        );
    }
    if old.manifest.dependencies != source.manifest.dependencies {
        return Err("update must preserve the selected package's exact dependencies".into());
    }
    Ok(())
}

fn validate_update(
    previous: &Lock,
    candidate: &Lock,
    source: &LockedPackage,
) -> Result<(), String> {
    validate_source(previous, source)?;
    let name = &source.manifest.name;
    if candidate.schema != previous.schema
        || candidate.direct.keys().ne(previous.direct.keys())
        || candidate.packages.keys().ne(previous.packages.keys())
    {
        return Err(
            "update must preserve all direct selections and installed package names".into(),
        );
    }
    for (old_name, old_version) in &previous.direct {
        if old_name != name && candidate.direct.get(old_name) != Some(old_version) {
            return Err(format!(
                "update changed another direct selection: {old_name}"
            ));
        }
    }
    for (old_name, old_package) in &previous.packages {
        if old_name != name && candidate.packages.get(old_name) != Some(old_package) {
            return Err(format!(
                "update changed another package identity: {old_name}"
            ));
        }
    }
    if candidate.direct.get(name) != Some(&source.manifest.version)
        || candidate.packages.get(name) != Some(source)
    {
        return Err("update preview does not match the inspected local revision".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "update_tests.rs"]
mod tests;
