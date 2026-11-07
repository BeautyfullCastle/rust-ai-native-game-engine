//! Explicit static-binding revision adoption into a new closed Room project.
//! The original lock and model sidecar are never written. Trusted quiescent
//! ancestry is required; observations are not a hostile-concurrency sandbox.
use serde::Serialize;
use std::{ffi::OsString, path::PathBuf};
pub const USAGE: &str = "orr_room_package_upgrade --project ABS_ROOM --source ABS_PACKAGE --output NEW_ABS_ROOM --acknowledge-model-changes\nLinux x86_64 only. Creates a new closed Room project with explicitly adopted static model bindings; original project is unchanged. Not an arbitrary directory clone or a license determination.";
#[derive(Debug, Serialize)]
pub struct BindingChange {
    pub guid: String,
    pub asset: String,
    pub old_package_digest: String,
    pub new_package_digest: String,
    pub old_source_hash: String,
    pub new_source_hash: String,
}
#[derive(Debug, Serialize)]
pub struct CopyReport {
    pub output: PathBuf,
    pub package: String,
    pub before_version: String,
    pub after_version: String,
    pub bindings: Vec<BindingChange>,
    pub scene_checksum: u64,
}
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use linux::{CopyPlan, apply_copy, plan_copy};
pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<CopyReport, String> {
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    {
        let _ = args;
        Err("Room package upgrade copy currently requires Linux x86_64".into())
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        let args: Vec<_> = args.into_iter().take(8).collect();
        if args.len() != 7 || args.iter().map(|a| a.len()).sum::<usize>() > 16384 {
            return Err(USAGE.into());
        }
        let (mut project, mut source, mut output, mut ack) = (None, None, None, false);
        let mut values = args.into_iter();
        while let Some(flag) = values.next() {
            match flag.to_str() {
                Some("--acknowledge-model-changes") if !ack => ack = true,
                Some("--project" | "--source" | "--output") => {
                    let slot = match flag.to_str() {
                        Some("--project") => &mut project,
                        Some("--source") => &mut source,
                        _ => &mut output,
                    };
                    if slot.is_some() {
                        return Err("duplicate path option".into());
                    }
                    let path = values.next().ok_or("missing path value")?;
                    if path.is_empty()
                        || matches!(
                            path.to_str(),
                            Some(
                                "--project"
                                    | "--source"
                                    | "--output"
                                    | "--acknowledge-model-changes"
                            )
                        )
                    {
                        return Err("missing path value".into());
                    }
                    *slot = Some(PathBuf::from(path));
                }
                _ => return Err("unknown or duplicate option".into()),
            }
        }
        if !ack {
            return Err("explicit acknowledgement of model changes is required".into());
        }
        let plan = plan_copy(
            &project.ok_or("--project required")?,
            &source.ok_or("--source required")?,
        )?;
        apply_copy(&plan, &output.ok_or("--output required")?, true)
    }
}
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod linux {
    use super::*;
    use crate::package_manager::{
        self as manager, UpdateImpactReport,
        removal::{RootIdentity, read_regular, read_room_files, root_identity},
    };
    use orr_model_bindings::model_bindings::{self, Document, ModelKind};
    use orr_package::{Lock, LockedPackage, Project};
    use sha2::{Digest, Sha256};
    use std::{
        collections::BTreeMap,
        fs::{self, File, OpenOptions},
        io::Write,
        path::{Component, Path},
    };
    const MAX_SNAPSHOT: u64 = 256 * 1024 * 1024;
    struct Source {
        package: LockedPackage,
        files: BTreeMap<String, Vec<u8>>,
    }
    pub struct CopyPlan {
        root: PathBuf,
        source: PathBuf,
        impact: UpdateImpactReport,
        previous: Lock,
        candidate: Lock,
        lock_bytes: Vec<u8>,
        sources: BTreeMap<String, Source>,
        room: BTreeMap<PathBuf, Vec<u8>>,
        models_path: PathBuf,
        bindings: Document,
        frame: Vec<u8>,
        checksum: u64,
        package: String,
    }
    fn digest(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
    fn charge(total: &mut u64, bytes: usize) -> Result<(), String> {
        *total = total
            .checked_add(bytes as u64)
            .ok_or("copy snapshot byte overflow")?;
        if *total > MAX_SNAPSHOT {
            return Err("copy declared content exceeds 256 MiB".into());
        }
        Ok(())
    }
    fn same_candidate(old: &Lock, new: &Lock, source: &LockedPackage) -> Result<(), String> {
        let name = &source.manifest.name;
        if old.schema != new.schema
            || old.direct.keys().ne(new.direct.keys())
            || old.packages.keys().ne(new.packages.keys())
            || new.direct.get(name) != Some(&source.manifest.version)
            || new.packages.get(name) != Some(source)
        {
            return Err("copy candidate differs from the reviewed package update".into());
        }
        for (key, value) in &old.packages {
            if key != name && new.packages.get(key) != Some(value) {
                return Err("copy update changed another package identity".into());
            }
        }
        for (key, value) in &old.direct {
            if key != name && new.direct.get(key) != Some(value) {
                return Err("copy update changed another direct selection".into());
            }
        }
        Ok(())
    }
    pub fn plan_copy(root: &Path, source: &Path) -> Result<CopyPlan, String> {
        let root = manager::checked_directory(root, "Room project")?;
        let source = manager::checked_directory(source, "package source")?;
        if root.starts_with(&source) || source.starts_with(&root) {
            return Err("Room and replacement source must not overlap".into());
        }
        // This is an early scope rejection, not Room admission. The strict
        // Project/PreparedProject decoders below still validate accepted inputs.
        let header: serde_json::Value = serde_json::from_slice(&read_regular(
            &root.join("orr.project.json"),
            manager::MAX_MANIFEST_BYTES,
        )?)
        .map_err(|e| e.to_string())?;
        if header.get("schema").and_then(serde_json::Value::as_u64) != Some(2)
            || header.get("progress").is_some()
        {
            return Err(
                "upgrade copies require schema-2 Room projects without progress identity".into(),
            );
        }
        let prepared = manager::open_room_project(&root)?;
        let project = Project::open(&root, manager::room_runtime()).map_err(|e| e.to_string())?;
        if project
            .manifest()
            .is_some_and(|m| m.schema != 2 || m.progress.is_some())
        {
            return Err(
                "upgrade copies support schema-2 Room projects without progress identity".into(),
            );
        }
        let impact = manager::inspect_update_impact(&root, &source)?;
        let replacement = manager::inspect_source(&source, &manager::room_runtime())?;
        let package = replacement.manifest.name.clone();
        let affected: Vec<_> = prepared
            .models()
            .document
            .bindings
            .values()
            .filter(|b| b.package == package)
            .collect();
        if affected.is_empty() {
            return Err("copy requires at least one saved binding to the selected package".into());
        }
        if affected.iter().any(|b| b.kind != ModelKind::Static) {
            return Err(
                "selected animated bindings are not supported by static upgrade copy".into(),
            );
        }
        let previous = project.verify().map_err(|e| e.to_string())?;
        let lock_bytes = read_regular(&root.join("orr.packages.lock.json"), 1024 * 1024)?;
        let preview = project
            .preview_install(std::slice::from_ref(&source))
            .map_err(|e| e.to_string())?;
        if preview.previous != previous {
            return Err("active package lock changed during planning".into());
        }
        same_candidate(&previous, &preview.candidate, &replacement)?;
        let room = read_room_files(&project)?;
        let models_path = prepared
            .models()
            .path
            .strip_prefix(&root)
            .map_err(|_| "Room models outside root")?
            .to_path_buf();
        let bindings: Document = serde_json::from_slice(
            room.get(&root.join(&models_path))
                .ok_or("Room models snapshot missing")?,
        )
        .map_err(|e| e.to_string())?;
        let mut total = 0;
        for bytes in room.values() {
            charge(&mut total, bytes.len())?;
        }
        let mut sources = BTreeMap::new();
        for (name, pkg) in &preview.candidate.packages {
            charge(
                &mut total,
                serde_json::to_vec(&pkg.manifest)
                    .map_err(|e| e.to_string())?
                    .len(),
            )?;
            let mut files = BTreeMap::new();
            for (relative, hash) in &pkg.files {
                let bytes = if name == &package {
                    read_regular(&source.join(relative), 64 * 1024 * 1024)?
                } else {
                    project
                        .read_asset(name, relative)
                        .map_err(|e| e.to_string())?
                };
                if digest(&bytes) != *hash {
                    return Err("package content changed during copy preparation".into());
                }
                charge(&mut total, bytes.len())?;
                files.insert(relative.clone(), bytes);
            }
            sources.insert(
                name.clone(),
                Source {
                    package: pkg.clone(),
                    files,
                },
            );
        }
        let plan = CopyPlan {
            root,
            source,
            impact,
            previous,
            candidate: preview.candidate,
            lock_bytes,
            sources,
            room,
            models_path,
            bindings,
            frame: prepared.scene().frame().to_bytes(),
            checksum: prepared.scene().frame().checksum(),
            package,
        };
        recheck(&plan)?;
        Ok(plan)
    }
    fn recheck(plan: &CopyPlan) -> Result<(), String> {
        plan.impact.validate_current()?;
        let project =
            Project::open(&plan.root, manager::room_runtime()).map_err(|e| e.to_string())?;
        if read_regular(&plan.root.join("orr.packages.lock.json"), 1024 * 1024)? != plan.lock_bytes
            || project.verify().map_err(|e| e.to_string())? != plan.previous
            || read_room_files(&project)? != plan.room
        {
            return Err("original Room changed since copy planning".into());
        }
        Ok(())
    }
    fn destination(
        plan: &CopyPlan,
        path: &Path,
    ) -> Result<(PathBuf, PathBuf, RootIdentity), String> {
        let text = path.to_str().ok_or("copy output must be UTF-8")?;
        if !path.is_absolute()
            || text.len() > 4096
            || path.components().count() > 64
            || text.chars().any(char::is_control)
            || text.contains('\\')
            || text
                .split('/')
                .skip(1)
                .any(|part| part.is_empty() || part == "." || part == "..")
            || path
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err("copy output must be bounded absolute normalized path".into());
        }
        let parent = manager::checked_directory(
            path.parent().ok_or("output parent missing")?,
            "copy output parent",
        )?;
        if parent.starts_with(&plan.root) || parent.starts_with(&plan.source) {
            return Err("copy output and staging must be outside both inputs".into());
        }
        let output = parent.join(path.file_name().ok_or("copy output leaf missing")?);
        if plan.root.starts_with(&output) || plan.source.starts_with(&output) {
            return Err("copy output overlaps inputs".into());
        }
        match fs::symlink_metadata(&output) {
            Ok(_) => return Err("copy output already exists".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        let identity = root_identity(&parent)?;
        Ok((parent, output, identity))
    }
    fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
        fs::create_dir_all(path.parent().ok_or("output file parent missing")?)
            .map_err(|e| e.to_string())?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())
    }
    pub fn apply_copy(
        plan: &CopyPlan,
        output: &Path,
        acknowledge_model_changes: bool,
    ) -> Result<CopyReport, String> {
        if !acknowledge_model_changes {
            return Err("explicit acknowledgement of model changes is required".into());
        }
        recheck(plan)?;
        let (parent, output, parent_identity) = destination(plan, output)?;
        let stage = tempfile::Builder::new()
            .prefix(".orr-room-package-upgrade-")
            .tempdir_in(&parent)
            .map_err(|e| e.to_string())?;
        let staged = stage.path().join("room");
        fs::create_dir(&staged).map_err(|e| e.to_string())?;
        let mut direct = Vec::new();
        let mut dependencies = Vec::new();
        for (name, source) in &plan.sources {
            let path = stage.path().join("sources").join(&source.package.digest);
            write_new(
                &path.join("orr.package.json"),
                &serde_json::to_vec(&source.package.manifest).map_err(|e| e.to_string())?,
            )?;
            for (relative, bytes) in &source.files {
                write_new(&path.join(relative), bytes)?;
            }
            if plan.candidate.direct.contains_key(name) {
                direct.push(path);
            } else {
                dependencies.push(path);
            }
        }
        for (path, bytes) in &plan.room {
            let relative = path
                .strip_prefix(&plan.root)
                .map_err(|_| "Room snapshot outside root")?;
            if relative != plan.models_path {
                write_new(&staged.join(relative), bytes)?;
            }
        }
        let project = Project::open(&staged, manager::room_runtime()).map_err(|e| e.to_string())?;
        let installed = project
            .install_with_dependencies(&direct, &dependencies)
            .map_err(|e| e.to_string())?;
        if installed != plan.candidate {
            return Err("copied installed lock differs from exact approved candidate".into());
        }
        let mut bindings: Document =
            serde_json::from_slice(&serde_json::to_vec(&plan.bindings).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        let mut changes = Vec::new();
        let mut loaded_assets = BTreeMap::new();
        for (guid, binding) in &mut bindings.bindings {
            if binding.package != plan.package {
                continue;
            }
            let loaded = match loaded_assets.entry(binding.asset.clone()) {
                std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(model_bindings::load_asset_from_project(
                        &project,
                        &binding.package,
                        &binding.asset,
                        ModelKind::Static,
                    )?)
                }
            };
            changes.push(BindingChange {
                guid: guid.clone(),
                asset: binding.asset.clone(),
                old_package_digest: binding.package_digest.clone(),
                new_package_digest: loaded.package_digest().into(),
                old_source_hash: binding.source_hash.clone(),
                new_source_hash: loaded.source_hash().into(),
            });
            binding.package_digest = loaded.package_digest().into();
            binding.source_hash = loaded.source_hash().into();
            binding.validate(loaded)?;
        }
        write_new(
            &staged.join(&plan.models_path),
            &serde_json::to_vec_pretty(&bindings).map_err(|e| e.to_string())?,
        )?;
        let prepared = manager::open_room_project(&staged)?;
        if prepared.scene().frame().to_bytes() != plan.frame
            || prepared.scene().frame().checksum() != plan.checksum
        {
            return Err("updated copy changed the admitted gameplay Frame".into());
        }
        if Project::open(&staged, manager::room_runtime())
            .and_then(|p| p.verify())
            .map_err(|e| e.to_string())?
            != plan.candidate
        {
            return Err("updated copy lock changed during Room admission".into());
        }
        recheck(plan)?;
        if root_identity(&parent)? != parent_identity
            || manager::checked_directory(&parent, "copy output parent")? != parent
        {
            return Err("copy output parent changed".into());
        }
        File::open(&staged)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        orr_sample::trusted_runtime::publish_no_replace(&staged, &output)?;
        File::open(&parent).and_then(|f|f.sync_all()).map_err(|e|format!("upgraded copy published at {} but durability uncertain; preserve it and do not retry: {e}",output.display()))?;
        Ok(CopyReport {
            output,
            package: plan.package.clone(),
            before_version: plan.previous.packages[&plan.package]
                .manifest
                .version
                .clone(),
            after_version: plan.candidate.packages[&plan.package]
                .manifest
                .version
                .clone(),
            bindings: changes,
            scene_checksum: plan.checksum,
        })
    }
}
#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
#[path = "referenced_copy/tests.rs"]
mod tests;
