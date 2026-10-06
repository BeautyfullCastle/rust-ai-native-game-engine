//! Versioned, presentation-only static-model bindings for persistent scene GUIDs.
//!
//! This document is an independent sidecar. It never writes model data into
//! the deterministic scene or host. Installed package data is read through
//! `orr_package::Project`; the returned model is immutable and reference counted.
#![allow(clippy::float_arithmetic)]

use orr_model::{Error as ModelError, StaticModel};
use orr_package::{LockedPackage, Project, Runtime};
use orr_reflect::Guid;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

const MAX_BYTES: u64 = 1024 * 1024;
const MAX_BINDINGS: usize = 4096;
const MAX_HISTORY: usize = 128;
const MAX_NAME_BYTES: usize = 64;
const MAX_PACKAGE_PATH_BYTES: usize = 240;
/// Only static models belong to this sidecar. Skeletal playback remains in its
/// independent animation document and is never inferred from a missing clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    Static,
}
impl<'de> Deserialize<'de> for ModelKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let kind = String::deserialize(deserializer)?;
        match kind.as_str() {
            "static" => Ok(Self::Static),
            _ => Err(serde::de::Error::custom(format!(
                "unsupported model binding kind {kind:?}; only static models are supported"
            ))),
        }
    }
}

/// Presentation-local TRS; host Body3 position/rotation are composed separately.
/// Rotation is an xyzw unit quaternion. External scale must remain positive.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalTransform {
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}
impl Default for LocalTransform {
    fn default() -> Self {
        Self {
            translation: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
        }
    }
}
impl LocalTransform {
    pub fn validate(self) -> Result<(), String> {
        if !self
            .translation
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 1.0e6)
        {
            return Err("model translation must be finite and within +/-1000000".into());
        }
        if !self
            .scale
            .iter()
            .all(|v| v.is_finite() && (0.001..=1000.0).contains(v))
        {
            return Err("model scale must be finite, positive, and between 0.001 and 1000".into());
        }
        if !self.rotation.iter().all(|v| v.is_finite()) {
            return Err("model rotation must be a finite normalized xyzw quaternion".into());
        }
        let norm_squared = self.rotation.iter().map(|v| v * v).sum::<f32>();
        if !norm_squared.is_finite() || (norm_squared - 1.0).abs() > 1.0e-4 {
            return Err("model rotation must be a normalized xyzw quaternion".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub kind: ModelKind,
    pub package: String,
    /// Declared package-relative cooked static model or source GLB/glTF path.
    pub asset: String,
    /// Verified lock identity of the installed package at assignment time.
    pub package_digest: String,
    /// Verified package-file hash of the declared model/source asset.
    pub source_hash: String,
    pub transform: LocalTransform,
}
impl Binding {
    pub fn from_asset(
        package: String,
        asset: String,
        loaded: &LoadedAsset,
        transform: LocalTransform,
    ) -> Result<Self, String> {
        if package != loaded.package || asset != loaded.asset {
            return Err(
                "loaded model asset identity does not match the requested package/path".into(),
            );
        }
        let binding = Self {
            kind: ModelKind::Static,
            package,
            asset,
            package_digest: loaded.package_digest.clone(),
            source_hash: loaded.source_hash.clone(),
            transform,
        };
        binding.validate_fields()?;
        Ok(binding)
    }

    fn validate_fields(&self) -> Result<(), String> {
        validate_package_name(&self.package)?;
        validate_asset_path(&self.asset)?;
        validate_digest(&self.package_digest, "package digest")?;
        validate_digest(&self.source_hash, "source hash")?;
        self.transform.validate()
    }

    /// Never silently retarget a persisted binding to replaced package content.
    pub fn validate(&self, loaded: &LoadedAsset) -> Result<(), String> {
        self.validate_fields()?;
        if self.package != loaded.package || self.asset != loaded.asset {
            return Err(
                "model binding refers to a different package asset; explicitly reassign it".into(),
            );
        }
        if self.package_digest != loaded.package_digest {
            return Err(
                "stale model binding: installed package changed; explicitly reassign the model"
                    .into(),
            );
        }
        if self.source_hash != loaded.source_hash {
            return Err(
                "stale model binding: source asset changed; explicitly reassign the model".into(),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub version: u32,
    /// Scene path relative to the sidecar directory; this is only a local-editor hint.
    pub scene: String,
    /// Installed package project path relative to the sidecar directory.
    pub project: String,
    /// Keys are persistent scene GUIDs, never recyclable ECS frame handles.
    #[serde(deserialize_with = "deserialize_unique_bindings")]
    pub bindings: BTreeMap<String, Binding>,
}

fn deserialize_unique_bindings<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, Binding>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{Error, MapAccess, Visitor};
    use std::fmt;

    struct BindingsVisitor;
    impl<'de> Visitor<'de> for BindingsVisitor {
        type Value = BTreeMap<String, Binding>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a map of unique persistent scene GUIDs to model bindings")
        }

        fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut bindings = BTreeMap::new();
            while let Some((guid, binding)) = access.next_entry::<String, Binding>()? {
                if bindings.len() >= MAX_BINDINGS && !bindings.contains_key(&guid) {
                    return Err(A::Error::custom("too many static-model bindings"));
                }
                if bindings.insert(guid.clone(), binding).is_some() {
                    return Err(A::Error::custom(format!(
                        "duplicate model binding GUID: {guid}"
                    )));
                }
            }
            Ok(bindings)
        }
    }
    deserializer.deserialize_map(BindingsVisitor)
}
impl Document {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err("unsupported model binding document version".into());
        }
        if !relative_hint(&self.scene) || !relative_hint(&self.project) {
            return Err("scene and project must be explicit relative local paths".into());
        }
        if self.bindings.len() > MAX_BINDINGS {
            return Err("too many static-model bindings".into());
        }
        for (guid, binding) in &self.bindings {
            let parsed = Guid::parse(guid).map_err(|_| format!("invalid scene GUID: {guid}"))?;
            if parsed.to_string() != *guid {
                return Err(format!("noncanonical scene GUID: {guid}"));
            }
            binding
                .validate_fields()
                .map_err(|error| format!("invalid model binding for {guid}: {error}"))?;
        }
        Ok(())
    }
}

/// Immutable, verified model data suitable for sharing with preview/render code.
#[derive(Clone, Debug)]
pub struct LoadedAsset {
    package: String,
    asset: String,
    package_digest: String,
    source_hash: String,
    model: Arc<StaticModel>,
}
impl LoadedAsset {
    pub fn package(&self) -> &str {
        &self.package
    }
    pub fn asset(&self) -> &str {
        &self.asset
    }
    pub fn package_digest(&self) -> &str {
        &self.package_digest
    }
    pub fn source_hash(&self) -> &str {
        &self.source_hash
    }
    pub fn model(&self) -> &Arc<StaticModel> {
        &self.model
    }
}

/// A validated candidate tied to one exact document revision. Preparing never
/// mutates state; commit cannot overwrite edits made after preparation.
pub struct PreparedAssignment {
    generation: Arc<()>,
    path: PathBuf,
    next: Document,
}

pub struct Bindings {
    pub path: PathBuf,
    document: Document,
    saved: Document,
    undo: Vec<Document>,
    redo: Vec<Document>,
    generation: Arc<()>,
}
impl Bindings {
    pub fn create(path: PathBuf, scene: String, project: String) -> Result<Self, String> {
        if std::fs::symlink_metadata(&path).is_ok() {
            return Err("sidecar exists; use Open bindings".into());
        }
        let document = Document {
            version: 1,
            scene,
            project,
            bindings: BTreeMap::new(),
        };
        document.validate()?;
        Ok(Self {
            path,
            saved: document.clone(),
            document,
            undo: Vec::new(),
            redo: Vec::new(),
            generation: Arc::new(()),
        })
    }

    pub fn open(path: PathBuf) -> Result<Self, String> {
        let initial_metadata = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if initial_metadata.file_type().is_symlink() || !initial_metadata.is_file() {
            return Err(
                "model binding sidecar must be a regular file, not a symlink or special file"
                    .into(),
            );
        }
        if initial_metadata.len() > MAX_BYTES {
            return Err("model binding file exceeds byte limit".into());
        }
        let mut bytes = Vec::new();
        let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
        let opened_metadata = file.metadata().map_err(|e| e.to_string())?;
        if !opened_metadata.is_file() {
            return Err("model binding sidecar changed to a special file while opening".into());
        }
        if opened_metadata.len() > MAX_BYTES {
            return Err("model binding file exceeds byte limit".into());
        }
        file.take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("model binding file exceeds byte limit".into());
        }
        let document: Document = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        document.validate()?;
        Ok(Self {
            path,
            saved: document.clone(),
            document,
            undo: Vec::new(),
            redo: Vec::new(),
            generation: Arc::new(()),
        })
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    pub fn dirty(&self) -> bool {
        self.document != self.saved || !self.path.exists()
    }

    pub fn base(&self) -> &Path {
        self.path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
    }

    /// Only compare the sidecar's local scene hint with a caller-provided local path.
    /// The editor must not pass a path received from a remote host here.
    pub fn matches_scene(&self, scene: &str) -> bool {
        match (
            std::fs::canonicalize(self.base().join(&self.document.scene)),
            std::fs::canonicalize(scene),
        ) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }

    /// Resolve only after matching the sidecar to a trusted local scene path.
    /// A remote host's scene path is never authority for local package reads.
    pub fn project_root(&self) -> Result<PathBuf, String> {
        resolve_project(self.base(), &self.document.project)
    }

    /// Validate a whole selection without mutating document, history, or dirtiness.
    /// The caller remains responsible for confirming GUIDs exist in its local scene.
    pub fn prepare_assignment(
        &self,
        guids: &[Guid],
        binding: &Binding,
        loaded: &LoadedAsset,
    ) -> Result<PreparedAssignment, String> {
        if guids.is_empty() {
            return Err("select at least one scene entity".into());
        }
        binding.validate(loaded)?;
        let mut next = self.document.clone();
        for guid in guids {
            next.bindings.insert(guid.to_string(), binding.clone());
        }
        next.validate()?;
        Ok(PreparedAssignment {
            generation: Arc::clone(&self.generation),
            path: self.path.clone(),
            next,
        })
    }

    pub fn commit_assignment(&mut self, prepared: PreparedAssignment) -> Result<(), String> {
        if !Arc::ptr_eq(&self.generation, &prepared.generation) || self.path != prepared.path {
            return Err(
                "model bindings changed after preparation; prepare the assignment again".into(),
            );
        }
        self.commit(prepared.next);
        Ok(())
    }

    /// Assignment is one validated transaction using immutable verified model data.
    pub fn assign_validated(
        &mut self,
        guids: &[Guid],
        binding: &Binding,
        loaded: &LoadedAsset,
    ) -> Result<(), String> {
        let prepared = self.prepare_assignment(guids, binding, loaded)?;
        self.commit_assignment(prepared)
    }

    fn commit(&mut self, next: Document) {
        if next != self.document {
            if self.undo.len() == MAX_HISTORY {
                self.undo.remove(0);
            }
            self.undo.push(std::mem::replace(&mut self.document, next));
            self.redo.clear();
            self.generation = Arc::new(());
        }
    }

    /// Remove a selection in one transactional undo step.
    pub fn remove(&mut self, guids: &[Guid]) -> Result<(), String> {
        if guids.is_empty() {
            return Err("select at least one scene entity".into());
        }
        let mut next = self.document.clone();
        for guid in guids {
            next.bindings.remove(&guid.to_string());
        }
        self.commit(next);
        Ok(())
    }

    pub fn undo(&mut self) {
        if let Some(previous) = self.undo.pop() {
            self.generation = Arc::new(());
            self.redo
                .push(std::mem::replace(&mut self.document, previous));
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.generation = Arc::new(());
            self.undo.push(std::mem::replace(&mut self.document, next));
        }
    }

    /// Atomic sidecar replacement. State/history become clean only after persist succeeds.
    pub fn save(&mut self) -> Result<(), String> {
        self.document.validate()?;
        let bytes = serde_json::to_vec_pretty(&self.document).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("model binding file exceeds byte limit".into());
        }
        let mut temp = tempfile::NamedTempFile::new_in(self.base()).map_err(|e| e.to_string())?;
        temp.write_all(&bytes)
            .and_then(|_| temp.as_file().sync_all())
            .map_err(|e| e.to_string())?;
        temp.persist(&self.path).map_err(|e| e.to_string())?;
        self.saved = self.document.clone();
        Ok(())
    }
}

/// Open a package project with exactly the models capability this preview consumes.
/// No package-provided path or host path can expand the local project root.
pub fn open_project(project_root: &Path) -> Result<Project, String> {
    let mut capabilities = BTreeSet::new();
    capabilities.insert("models".to_owned());
    let mut runtime = Runtime::content_only();
    runtime.capabilities = capabilities;
    let project = Project::open(project_root, runtime).map_err(|e| format!("project: {e}"))?;
    project
        .verify()
        .map_err(|e| format!("package verification: {e}"))?;
    Ok(project)
}

/// A declared model candidate in a fully verified installed package. GLB/glTF
/// candidates are imported only when selected; unsupported source content remains
/// a visible assignment error. Cooked JSON candidates declare their format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetInfo {
    pub package: String,
    pub asset: String,
    pub kind: ModelKind,
}

pub fn list_assets(project_root: &Path) -> Result<Vec<AssetInfo>, String> {
    let project = open_project(project_root)?;
    let before = project
        .verify()
        .map_err(|e| format!("package verification: {e}"))?;
    let mut assets = Vec::new();
    for (package, locked) in &before.packages {
        if !locked.manifest.capabilities.contains("models") {
            continue;
        }
        for asset in locked.files.keys() {
            validate_asset_path(asset)?;
            let extension = Path::new(asset)
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            let candidate = if extension.eq_ignore_ascii_case("glb")
                || extension.eq_ignore_ascii_case("gltf")
            {
                true
            } else if extension.eq_ignore_ascii_case("json") {
                let bytes = project
                    .read_asset(package, asset)
                    .map_err(|e| format!("model catalog {asset}: {e}"))?;
                verify_expected_asset(locked, asset, &bytes)?;
                #[derive(Deserialize)]
                struct FormatHeader {
                    format: String,
                }
                serde_json::from_slice::<FormatHeader>(&bytes)
                    .is_ok_and(|header| header.format == "orr_static_model")
            } else {
                false
            };
            if candidate {
                if assets.len() == MAX_BINDINGS {
                    return Err("model asset catalog exceeds entry limit".into());
                }
                assets.push(AssetInfo {
                    package: package.clone(),
                    asset: asset.clone(),
                    kind: ModelKind::Static,
                });
            }
        }
    }
    let after = project
        .verify()
        .map_err(|e| format!("package verification after catalog read: {e}"))?;
    if before != after {
        return Err("installed model packages changed while listing; retry".into());
    }
    Ok(assets)
}

/// Load and verify one declared static asset from the currently installed project.
/// Raw GLB/glTF sources are imported with package-scoped resource reads, then cooked
/// and reloaded through the exact immutable runtime validation path before exposure.
pub fn load_asset(project_root: &Path, package: &str, asset: &str) -> Result<LoadedAsset, String> {
    validate_package_name(package)?;
    validate_asset_path(asset)?;
    let project = open_project(project_root)?;
    let before = project
        .verify()
        .map_err(|e| format!("package verification: {e}"))?;
    let locked = before
        .packages
        .get(package)
        .ok_or_else(|| format!("model binding package was removed: {package}"))?
        .clone();
    if !locked.manifest.capabilities.contains("models") {
        return Err(format!(
            "package {package} does not declare the models capability"
        ));
    }
    let (package_digest, source_hash) = verified_identity(&locked, asset)?;
    let bytes = project
        .read_asset(package, asset)
        .map_err(|e| format!("model asset {asset}: {e}"))?;
    verify_expected_asset(&locked, asset, &bytes)?;
    let after = project
        .verify()
        .map_err(|e| format!("package verification after asset read: {e}"))?;
    let after_locked = after
        .packages
        .get(package)
        .ok_or_else(|| format!("model binding package changed during load: {package}"))?;
    let (after_package_digest, after_source_hash) = verified_identity(after_locked, asset)?;
    if package_digest != after_package_digest || source_hash != after_source_hash {
        return Err("installed model package changed while loading; retry".into());
    }

    let model = if let Ok(cooked) = StaticModel::from_bytes(&bytes) {
        cooked
    } else {
        let imported = orr_model::import::import_with_resolver(asset, &bytes, |uri| {
            let resolved = resolve_package_uri(asset, uri).map_err(ModelError::message)?;
            let resource = project
                .read_asset(package, &resolved)
                .map_err(|e| ModelError::message(format!("package resource {resolved}: {e}")))?;
            verify_expected_asset(&locked, &resolved, &resource).map_err(ModelError::message)?;
            Ok(resource)
        })
        .map_err(|e| format!("static model import: {e}"))?;
        // Round-trip through the bounded cooked representation before sharing.
        let cooked = imported
            .to_bytes()
            .map_err(|e| format!("static model cook: {e}"))?;
        StaticModel::from_bytes(&cooked).map_err(|e| format!("cooked static model reload: {e}"))?
    };
    // Also verify after external dependency reads and parsing, so one load cannot
    // combine content from different package-lock generations.
    let final_lock = project
        .verify()
        .map_err(|e| format!("package verification after model load: {e}"))?;
    let final_locked = final_lock
        .packages
        .get(package)
        .ok_or_else(|| format!("model binding package changed during load: {package}"))?;
    let (final_package_digest, final_source_hash) = verified_identity(final_locked, asset)?;
    if package_digest != final_package_digest || source_hash != final_source_hash {
        return Err("installed model package changed while loading; retry".into());
    }
    Ok(LoadedAsset {
        package: package.to_owned(),
        asset: asset.to_owned(),
        package_digest,
        source_hash,
        model: Arc::new(model),
    })
}

/// Resolve a persisted assignment. A missing package/file stays a visible error, while
/// changed content returns an explicit stale-binding diagnostic requiring reassignment.
pub fn load_binding(project_root: &Path, binding: &Binding) -> Result<LoadedAsset, String> {
    binding.validate_fields()?;
    // Diagnose changed identity before importing any replacement bytes, even if
    // the replacement is malformed or no longer a supported model.
    let project = open_project(project_root)?;
    let lock = project
        .verify()
        .map_err(|e| format!("package verification: {e}"))?;
    let locked = lock
        .packages
        .get(&binding.package)
        .ok_or_else(|| format!("model binding package was removed: {}", binding.package))?;
    let (package_digest, source_hash) = verified_identity(locked, &binding.asset)?;
    if package_digest != binding.package_digest {
        return Err(
            "stale model binding: installed package changed; explicitly reassign the model".into(),
        );
    }
    if source_hash != binding.source_hash {
        return Err(
            "stale model binding: source asset changed; explicitly reassign the model".into(),
        );
    }
    let loaded = load_asset(project_root, &binding.package, &binding.asset)?;
    binding.validate(&loaded)?;
    Ok(loaded)
}

/// Resolve a portable project-root path relative to the sidecar without following symlinks.
pub fn resolve_project(base: &Path, relative_project: &str) -> Result<PathBuf, String> {
    if !relative_hint(relative_project) {
        return Err("project path must be relative".into());
    }
    let current = if base.is_absolute() {
        PathBuf::new()
    } else {
        std::env::current_dir().map_err(|e| e.to_string())?
    };
    let mut resolved = PathBuf::new();
    // Do not join onto a Windows verbatim path: push/join normalizes ParentDir
    // before we can reject root escapes or symlinks along the original path.
    // Parse portable relative separators before appending individual components.
    let mut components = current
        .components()
        .chain(base.components())
        .chain(Path::new(relative_project).components())
        .peekable();
    while let Some(component) = components.next() {
        match component {
            Component::CurDir => continue,
            Component::ParentDir => {
                if !resolved.pop() {
                    return Err("project path escapes filesystem root".into());
                }
            }
            _ => resolved.push(component),
        }
        // Canonical Windows paths include a verbatim drive prefix. Its bare
        // prefix is a device, not a directory; check it together with RootDir.
        // A prefix-only UNC root still needs its own metadata check.
        if matches!(component, Component::Prefix(_))
            && components.peek() == Some(&Component::RootDir)
        {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&resolved)
            .map_err(|e| format!("project directory {}: {e}", resolved.display()))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "project path must contain only real directories: {}",
                resolved.display()
            ));
        }
    }
    Ok(resolved)
}

fn verified_identity(locked: &LockedPackage, asset: &str) -> Result<(String, String), String> {
    let source_hash = locked
        .files
        .get(asset)
        .ok_or_else(|| format!("model asset was removed or is not declared: {asset}"))?;
    validate_digest(&locked.digest, "package digest")?;
    validate_digest(source_hash, "source hash")?;
    Ok((locked.digest.clone(), source_hash.clone()))
}

fn verify_expected_asset(locked: &LockedPackage, asset: &str, bytes: &[u8]) -> Result<(), String> {
    let expected = locked
        .files
        .get(asset)
        .ok_or_else(|| format!("model package asset is not in the captured lock: {asset}"))?;
    validate_digest(expected, "captured package asset hash")?;
    verify_expected_hash(expected, bytes).map_err(|()| {
        format!(
            "model package asset changed during load or does not match the captured lock: {asset}"
        )
    })
}

fn verify_expected_hash(expected: &str, bytes: &[u8]) -> Result<(), ()> {
    if sha256_hex(bytes) == expected {
        Ok(())
    } else {
        Err(())
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn validate_package_name(package: &str) -> Result<(), String> {
    if package.is_empty()
        || package.len() > MAX_NAME_BYTES
        || !package
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'-' | b'_'))
    {
        return Err("invalid installed package name".into());
    }
    Ok(())
}

fn validate_asset_path(asset: &str) -> Result<(), String> {
    if asset.is_empty() || asset.len() > MAX_PACKAGE_PATH_BYTES || !asset.is_ascii() {
        return Err("invalid package asset path".into());
    }
    let path = Path::new(asset);
    let parts = asset.split('/').collect::<Vec<_>>();
    let valid_parts = parts.iter().all(|part| {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        !part.is_empty()
            && *part != "."
            && *part != ".."
            && !part.ends_with('.')
            && part.len() <= 100
            && part
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
            && !["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
            && !(stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit())
    });
    if path.is_absolute()
        || asset.contains('\\')
        || asset.contains(':')
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        || parts.len() > 16
        || !valid_parts
    {
        return Err("package asset path must be portable and contain no traversal".into());
    }
    Ok(())
}

fn validate_digest(digest: &str, label: &str) -> Result<(), String> {
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(format!("invalid {label}"));
    }
    Ok(())
}

fn relative_hint(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && path.split('/').count() <= 64
        && !path.contains('\\')
        && !path.contains(':')
        && !path.contains('\0')
        && !Path::new(path).is_absolute()
}

/// `uri` is already normalized by the model importer. Resolve it inside the
/// installed package namespace, rejecting any attempt to escape that namespace.
fn resolve_package_uri(asset: &str, uri: &str) -> Result<String, String> {
    validate_asset_path(asset)?;
    if uri.is_empty() || uri.contains('\\') || uri.contains(':') || uri.contains('\0') {
        return Err("invalid model dependency URI".into());
    }
    let mut parts: Vec<&str> = asset.split('/').collect();
    parts.pop(); // the source path is a file
    for component in Path::new(uri).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if parts.pop().is_none() {
                    return Err("model dependency escapes its package".into());
                }
            }
            Component::Normal(part) => {
                let part = part
                    .to_str()
                    .ok_or_else(|| "model dependency path is not UTF-8".to_owned())?;
                parts.push(part);
            }
            _ => return Err("model dependency must remain package-relative".into()),
        }
    }
    let resolved = parts.join("/");
    validate_asset_path(&resolved)?;
    Ok(resolved)
}

#[cfg(test)]
mod integrity_tests {
    use super::{sha256_hex, verify_expected_hash};

    #[test]
    fn captured_lock_hash_accepts_only_the_exact_bytes() {
        let locked_bytes = b"fixture source bytes from the captured lock";
        let expected = sha256_hex(locked_bytes);
        assert_eq!(verify_expected_hash(&expected, locked_bytes), Ok(()));
        assert_eq!(
            verify_expected_hash(&expected, b"replacement bytes from another lock generation"),
            Err(())
        );
    }
}
