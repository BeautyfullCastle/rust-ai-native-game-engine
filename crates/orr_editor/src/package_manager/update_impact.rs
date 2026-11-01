//! Read-only observations of referenced assets in a different package revision.
//! This report is not an update plan and cannot authorize any package write.
use super::removal::{admit_candidate, read_regular, root_identity, RootIdentity, Snapshot};
use super::{checked_directory, inspect_source, MAX_MANIFEST_BYTES};
use orr_model_bindings::model_bindings::ModelKind;
use orr_package::{Lock, LockedPackage, Project, Runtime};
use orr_sample::room_project::PreparedProject;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetChange {
    Missing,
    BytesChanged,
    BytesUnchanged,
}
#[derive(Debug)]
pub struct BindingImpact {
    pub guid: String,
    pub asset: String,
    pub kind: ModelKind,
    pub change: AssetChange,
}
/// Immutable observation only. Candidate model decoding and compatibility are unchecked.
pub struct UpdateImpactReport {
    root: PathBuf,
    source: PathBuf,
    before: Lock,
    candidate: LockedPackage,
    source_identity: RootIdentity,
    source_manifest: Vec<u8>,
    snapshot: Snapshot,
    bindings: Vec<BindingImpact>,
}
impl UpdateImpactReport {
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn source(&self) -> &Path {
        &self.source
    }
    pub fn name(&self) -> &str {
        &self.candidate.manifest.name
    }
    pub fn before_version(&self) -> &str {
        &self.before.packages[self.name()].manifest.version
    }
    pub fn after_version(&self) -> &str {
        &self.candidate.manifest.version
    }
    pub fn bindings(&self) -> &[BindingImpact] {
        &self.bindings
    }
    /// Recheck on a worker before displaying a queued result, not on each UI frame.
    /// This is a point-in-time snapshot, not a filesystem watcher or apply token.
    pub fn validate_current(&self) -> Result<(), String> {
        if checked_directory(&self.root, "Room project")? != self.root
            || checked_directory(&self.source, "package source")? != self.source
            || root_identity(&self.source)? != self.source_identity
        {
            return Err("project or source identity changed; inspect impact again".into());
        }
        let current = Project::open(&self.root, impact_runtime())
            .map_err(|e| e.to_string())?
            .verify()
            .map_err(|e| e.to_string())?;
        if current != self.before
            || read_regular(&self.source.join("orr.package.json"), MAX_MANIFEST_BYTES)?
                != self.source_manifest
            || inspect_source(&self.source, &impact_runtime())? != self.candidate
            || admit_candidate(&self.root, &self.before, &self.before)? != self.snapshot
            || root_identity(&self.source)? != self.source_identity
        {
            return Err(
                "saved project, bindings, lock or source changed; inspect impact again".into(),
            );
        }
        Ok(())
    }
}
// Use editor-local capability flags: dependency feature unification must not admit
// animated content in an editor compiled without its animated-model UI/runtime.
fn impact_runtime() -> Runtime {
    let mut runtime = Runtime::content_only();
    runtime.capabilities.insert("models".into());
    #[cfg(feature = "room-animated-models")]
    runtime.capabilities.insert("animation".into());
    runtime
}

pub fn inspect_update_impact(root: &Path, source: &Path) -> Result<UpdateImpactReport, String> {
    let root = checked_directory(root, "Room project")?;
    let source = checked_directory(source, "package source")?;
    let source_identity = root_identity(&source)?;
    let source_manifest = read_regular(&source.join("orr.package.json"), MAX_MANIFEST_BYTES)?;
    let candidate = inspect_source(&source, &impact_runtime())?;
    let project = Project::open(&root, impact_runtime()).map_err(|e| e.to_string())?;
    let before = project.verify().map_err(|e| e.to_string())?;
    validate_source(&before, &candidate)?;
    let preview = project
        .preview_install(std::slice::from_ref(&source))
        .map_err(|e| e.to_string())?;
    if preview.previous != before {
        return Err("package lock changed during inspection".into());
    }
    validate_update(&before, &preview.candidate, &candidate)?;
    // Admit the CURRENT Room only. Never admit the replacement as safe to apply.
    let snapshot = admit_candidate(&root, &before, &before)?;
    let prepared = PreparedProject::open(&root)?;
    let mut bindings = Vec::new();
    for (guid, binding) in &prepared.models().document.bindings {
        #[cfg(not(feature = "room-animated-models"))]
        if binding.kind == ModelKind::Animated {
            return Err("animated bindings require room-animated-models".into());
        }
        if binding.package != candidate.manifest.name {
            continue;
        }
        let change = match candidate.files.get(&binding.asset) {
            None => AssetChange::Missing,
            Some(hash) if hash == &binding.source_hash => AssetChange::BytesUnchanged,
            Some(_) => AssetChange::BytesChanged,
        };
        bindings.push(BindingImpact {
            guid: guid.clone(),
            asset: binding.asset.clone(),
            kind: binding.kind,
            change,
        });
    }
    let report = UpdateImpactReport {
        root,
        source,
        before,
        candidate,
        source_identity,
        source_manifest,
        snapshot,
        bindings,
    };
    report.validate_current()?;
    Ok(report)
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
#[path = "update_impact_tests.rs"]
mod tests;
