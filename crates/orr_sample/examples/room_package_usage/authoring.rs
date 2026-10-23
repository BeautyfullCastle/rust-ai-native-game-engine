//! Private saved-state query shared by the example and its consumer tests.
//! It never grants mutation authority and assumes a quiescent trusted directory.
use orr_model_bindings::model_bindings::ModelKind;
use orr_package::{PackageExplanation, Project};
use orr_reflect::Guid;
use orr_sample::room_project::{compiled_runtime, PreparedProject};
use serde::Serialize;
use std::{ffi::OsString, path::PathBuf};

pub const USAGE: &str = "room_package_usage --project PATH --package NAME --digest EXACT_DIGEST\nRequires --features room-project (or room-animated-models).\nReads saved Room bindings only; unsaved edits are excluded. Zero bindings is NOT permission or proof that removal is safe.";

#[derive(Debug)]
pub struct Request {
    pub project: PathBuf,
    pub package: String,
    pub digest: String,
}

#[derive(Debug, Serialize)]
pub struct UsageBinding {
    pub guid: String,
    pub name: Option<String>,
    pub asset: String,
    pub kind: ModelKind,
    pub package_digest: String,
    pub source_hash: String,
}

#[derive(Debug, Serialize)]
pub struct UsageReport {
    pub schema: u32,
    pub scope: &'static str,
    pub warning: &'static str,
    pub scene: String,
    pub package: PackageExplanation,
    pub bindings: Vec<UsageBinding>,
}

pub fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Option<Request>, String> {
    // At most six flag/value arguments, plus one to diagnose excess input.
    let args: Vec<_> = args.into_iter().take(7).collect();
    if args.len() == 1 && args[0] == "--help" {
        return Ok(None);
    }
    if args.len() != 6 {
        return Err("exactly --project, --package and --digest are required".into());
    }
    let mut project = None;
    let mut package = None;
    let mut digest = None;
    // The exact six-argument check above guarantees three complete pairs.
    // Keep this shared helper compatible with orr_sample's Rust 1.85 MSRV.
    for pair in args.chunks(2) {
        if pair[1].is_empty() {
            return Err("empty option value".into());
        }
        let slot = match pair[0].to_str() {
            Some("--project") => &mut project,
            Some("--package") => &mut package,
            Some("--digest") => &mut digest,
            _ => return Err("unknown option".into()),
        };
        if slot.replace(pair[1].clone()).is_some() {
            return Err("duplicate option".into());
        }
    }
    Ok(Some(Request {
        project: project.ok_or("missing --project")?.into(),
        package: package
            .ok_or("missing --package")?
            .into_string()
            .map_err(|_| "invalid package name")?,
        digest: digest
            .ok_or("missing --digest")?
            .into_string()
            .map_err(|_| "invalid digest")?,
    }))
}

pub fn inspect_usage(request: &Request) -> Result<UsageReport, String> {
    if request.digest.len() != 64
        || !request
            .digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("digest must be exactly 64 lowercase hexadecimal characters".into());
    }
    let project = Project::open(&request.project, compiled_runtime()).map_err(|e| e.to_string())?;
    // explain validates the bounded name and complete dependency graph, not bytes.
    let explanation = project
        .explain(&request.package)
        .map_err(|e| e.to_string())?;
    let before = project.verify().map_err(|e| e.to_string())?;
    let installed = before
        .packages
        .get(&request.package)
        .ok_or("requested package is not installed")?;
    if installed.digest != request.digest || explanation.digest != request.digest {
        return Err("requested digest does not match the installed package; query again with its exact identity".into());
    }
    if !installed.manifest.capabilities.contains("models")
        && !installed.manifest.capabilities.contains("animation")
    {
        return Err("requested package declares neither models nor animation".into());
    }
    let manifest = project.manifest().ok_or("Room project manifest missing")?;
    let entry = manifest.entry.as_ref().ok_or("Room entry missing")?;
    let model_path = entry.models.as_ref().ok_or("Room models missing")?;
    let prepared = PreparedProject::open(project.root())?;
    if prepared.root() != project.root()
        || prepared.path() != project.root().join(&entry.scene)
        || prepared.models().path != project.root().join(model_path)
    {
        return Err("saved Room entry changed during query; retry".into());
    }
    let mut bindings = Vec::new();
    // Assets are deduplicated, bindings are not: enumerate each saved entity.
    for (guid, binding) in &prepared.models().document.bindings {
        if binding.package != request.package {
            continue;
        }
        if binding.package_digest != request.digest {
            return Err("saved binding package identity changed during query; retry".into());
        }
        let parsed = Guid::parse(guid)?;
        bindings.push(UsageBinding {
            guid: guid.clone(),
            name: prepared.scene().index().name(&parsed).map(str::to_owned),
            asset: binding.asset.clone(),
            kind: binding.kind,
            package_digest: binding.package_digest.clone(),
            source_hash: binding.source_hash.clone(),
        });
    }
    // Reopen: Project caches its manifest, so verify on the old object is insufficient.
    let after = Project::open(project.root(), compiled_runtime()).map_err(|e| e.to_string())?;
    if after.manifest() != Some(manifest)
        || after.verify().map_err(|e| e.to_string())? != before
        || after.explain(&request.package).map_err(|e| e.to_string())? != explanation
    {
        return Err("saved project or installed packages changed during query; retry".into());
    }
    Ok(UsageReport {
        schema: 1,
        scope: "saved-room-model-bindings",
        warning: "Unsaved editor edits are excluded. Zero bindings is not removal permission or proof of safety. This is a quiescent saved-state observation, not an atomic snapshot.",
        scene: entry.scene.clone(),
        package: explanation,
        bindings,
    })
}
