//! Versioned, presentation-only model bindings for persistent scene GUIDs.
//!
//! This document is an independent sidecar. It never writes model data into
//! the deterministic scene or host. Installed package data is read through
//! `orr_package::Project`; the returned model is immutable and reference counted.
#![allow(clippy::float_arithmetic)]

#[cfg(feature = "animated-models")]
use orr_model::animation::AnimatedModel;
use orr_model::{Error as ModelError, StaticModel};
use orr_package::{LockedPackage, Project, Runtime};
use orr_reflect::Guid;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

const MAX_BINDINGS: usize = 4096;
const MAX_NAME_BYTES: usize = 64;
const MAX_PACKAGE_PATH_BYTES: usize = 240;
const MAX_CLIPS: u32 = 32;
/// The persisted model kind is explicit; a missing animation descriptor never
/// changes the meaning of an asset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    Static,
    Animated,
}

/// Yard-sidecar playback policy. It deliberately stays independent of the
/// Arena animation-preview sidecar and carries no transient playback time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackMode {
    Once,
    Loop,
}

/// Persisted clip selection for an explicitly animated model binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimationDescriptor {
    pub clip_index: u32,
    pub playback: PlaybackMode,
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
    /// Declared package-relative cooked model or source GLB/glTF path.
    pub asset: String,
    /// Verified lock identity of the installed package at assignment time.
    pub package_digest: String,
    /// Verified package-file hash of the declared model/source asset.
    pub source_hash: String,
    pub transform: LocalTransform,
    /// Present exactly for animated bindings. Clip indices are verified against
    /// the immutable loaded asset before assignment and again during resolution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animation: Option<AnimationDescriptor>,
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
        if loaded.kind() != ModelKind::Static {
            return Err("loaded package asset is animated; select the animated model kind".into());
        }
        let binding = Self {
            kind: ModelKind::Static,
            package,
            asset,
            package_digest: loaded.package_digest.clone(),
            source_hash: loaded.source_hash.clone(),
            transform,
            animation: None,
        };
        binding.validate_fields()?;
        Ok(binding)
    }

    /// Build an animated binding from the exact verified asset and clip index.
    pub fn from_animated_asset(
        package: String,
        asset: String,
        loaded: &LoadedAsset,
        animation: AnimationDescriptor,
        transform: LocalTransform,
    ) -> Result<Self, String> {
        if package != loaded.package || asset != loaded.asset {
            return Err(
                "loaded animated asset identity does not match the requested package/path".into(),
            );
        }
        if loaded.kind() != ModelKind::Animated {
            return Err("loaded package asset is static; select the static model kind".into());
        }
        #[cfg(feature = "animated-models")]
        if loaded
            .animated_model()
            .and_then(|model| model.source().clips.get(animation.clip_index as usize))
            .is_none()
        {
            return Err(format!(
                "animation clip index {} does not exist",
                animation.clip_index
            ));
        }
        #[cfg(not(feature = "animated-models"))]
        {
            let _ = (animation, transform);
            Err("animated model bindings require the animated-models feature".into())
        }

        #[cfg(feature = "animated-models")]
        {
            let binding = Self {
                kind: ModelKind::Animated,
                package,
                asset,
                package_digest: loaded.package_digest.clone(),
                source_hash: loaded.source_hash.clone(),
                transform,
                animation: Some(animation),
            };
            binding.validate_fields()?;
            Ok(binding)
        }
    }

    fn validate_fields(&self) -> Result<(), String> {
        validate_package_name(&self.package)?;
        validate_asset_path(&self.asset)?;
        validate_digest(&self.package_digest, "package digest")?;
        validate_digest(&self.source_hash, "source hash")?;
        self.transform.validate()?;
        match (self.kind, self.animation) {
            (ModelKind::Static, None) => Ok(()),
            (ModelKind::Static, Some(_)) => {
                Err("static model binding cannot declare animation playback".into())
            }
            (ModelKind::Animated, None) => {
                Err("animated model binding requires an animation descriptor".into())
            }
            (ModelKind::Animated, Some(animation)) => {
                if animation.clip_index >= MAX_CLIPS {
                    return Err("animation clip index exceeds the supported clip limit".into());
                }
                let _ = animation.playback;
                Ok(())
            }
        }
    }

    /// Never silently retarget a persisted binding to replaced package content.
    pub fn validate(&self, loaded: &LoadedAsset) -> Result<(), String> {
        self.validate_fields()?;
        if self.kind != loaded.kind() {
            return Err("model binding kind does not match the loaded package asset".into());
        }
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
        if let Some(animation) = self.animation {
            #[cfg(feature = "animated-models")]
            if loaded
                .animated_model()
                .and_then(|model| model.source().clips.get(animation.clip_index as usize))
                .is_none()
            {
                return Err(format!(
                    "animation clip index {} no longer exists; explicitly reassign the clip",
                    animation.clip_index
                ));
            }
            #[cfg(not(feature = "animated-models"))]
            {
                let _ = animation;
                return Err("animated model bindings require the animated-models feature".into());
            }
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
        if !matches!(self.version, 1 | 2) {
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
            if self.version == 1 && binding.kind != ModelKind::Static {
                return Err("version-1 model bindings support static models only".into());
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
    model: LoadedModel,
}
#[derive(Clone, Debug)]
enum LoadedModel {
    Static(Arc<StaticModel>),
    #[cfg(feature = "animated-models")]
    Animated(Arc<AnimatedModel>),
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
    pub fn kind(&self) -> ModelKind {
        match &self.model {
            LoadedModel::Static(_) => ModelKind::Static,
            #[cfg(feature = "animated-models")]
            LoadedModel::Animated(_) => ModelKind::Animated,
        }
    }
    pub fn static_model(&self) -> Option<&Arc<StaticModel>> {
        match &self.model {
            LoadedModel::Static(model) => Some(model),
            #[cfg(feature = "animated-models")]
            LoadedModel::Animated(_) => None,
        }
    }
    #[cfg(feature = "animated-models")]
    pub fn animated_model(&self) -> Option<&Arc<AnimatedModel>> {
        match &self.model {
            LoadedModel::Animated(model) => Some(model),
            LoadedModel::Static(_) => None,
        }
    }
}

/// Open a Yard model project with compiled model and optional irradiance capabilities. The
/// Arena preview's `animated_bindings::open_project` intentionally remains
/// animation-only; this loader is the mixed static/skinned Yard path.
pub fn open_project(project_root: &Path) -> Result<Project, String> {
    let mut capabilities = BTreeSet::new();
    capabilities.insert("models".to_owned());
    #[cfg(feature = "terrain")]
    capabilities.insert("terrain_v1".to_owned());
    #[cfg(feature = "irradiance-probes")]
    capabilities.insert("irradiance-probes".to_owned());
    #[cfg(feature = "animated-models")]
    capabilities.insert("animation".to_owned());
    let mut runtime = Runtime::content_only();
    runtime.capabilities = capabilities;
    let project = Project::open(project_root, runtime).map_err(|e| format!("project: {e}"))?;
    project
        .verify()
        .map_err(|e| format!("package verification: {e}"))?;
    Ok(project)
}

/// A declared model candidate in a fully verified installed package. GLB/glTF
/// candidates are imported only when selected; package capability declarations
/// select their explicit model kind. Cooked JSON candidates declare their format.
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
        let supports_static = locked.manifest.capabilities.contains("models");
        #[cfg(feature = "animated-models")]
        let supports_animated = locked.manifest.capabilities.contains("animation");
        #[cfg(not(feature = "animated-models"))]
        let supports_animated = false;
        if !supports_static && !supports_animated {
            continue;
        }
        for asset in locked.files.keys() {
            validate_asset_path(asset)?;
            let extension = Path::new(asset)
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            let kinds = if extension.eq_ignore_ascii_case("glb")
                || extension.eq_ignore_ascii_case("gltf")
            {
                // For a package declaring both capabilities, expose both explicit
                // choices and let the selected loader validate actual content.
                let mut kinds = Vec::new();
                if supports_static {
                    kinds.push(ModelKind::Static);
                }
                if supports_animated {
                    kinds.push(ModelKind::Animated);
                }
                kinds
            } else if extension.eq_ignore_ascii_case("json") {
                let bytes = project
                    .read_asset(package, asset)
                    .map_err(|e| format!("model catalog {asset}: {e}"))?;
                verify_expected_asset(locked, asset, &bytes)?;
                #[derive(Deserialize)]
                struct FormatHeader {
                    format: String,
                }
                match serde_json::from_slice::<FormatHeader>(&bytes)
                    .ok()
                    .map(|h| h.format)
                {
                    Some(format) if format == "orr_static_model" && supports_static => {
                        vec![ModelKind::Static]
                    }
                    #[cfg(feature = "animated-models")]
                    Some(format) if format == "orr_animated_model" && supports_animated => {
                        vec![ModelKind::Animated]
                    }
                    _ => Vec::new(),
                }
            } else {
                Vec::new()
            };
            for kind in kinds {
                if assets.len() == MAX_BINDINGS {
                    return Err("model asset catalog exceeds entry limit".into());
                }
                assets.push(AssetInfo {
                    package: package.clone(),
                    asset: asset.clone(),
                    kind,
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

/// Load and verify one declared static asset. Preserved as the default static API
/// for existing model callers; animated callers must select their kind explicitly.
pub fn load_asset(project_root: &Path, package: &str, asset: &str) -> Result<LoadedAsset, String> {
    load_asset_for_kind(project_root, package, asset, ModelKind::Static)
}

/// Load one explicitly typed model. Yard uses a single project opened with its
/// compiled `models` + `animation` capability set so mixed package projects work;
/// the standalone Arena animation preview keeps its narrower loader policy.
pub fn load_asset_for_kind(
    project_root: &Path,
    package: &str,
    asset: &str,
    kind: ModelKind,
) -> Result<LoadedAsset, String> {
    validate_package_name(package)?;
    validate_asset_path(asset)?;
    let project = open_project(project_root)?;
    load_asset_from_project(&project, package, asset, kind)
}

/// Load one explicitly typed model through an already-admitted project.
/// Capability admission belongs to the caller, independently of unified crate features.
pub fn load_asset_from_project(
    project: &Project,
    package: &str,
    asset: &str,
    kind: ModelKind,
) -> Result<LoadedAsset, String> {
    validate_package_name(package)?;
    validate_asset_path(asset)?;
    match kind {
        ModelKind::Static => load_static_asset_from_project(project, package, asset),
        ModelKind::Animated => {
            #[cfg(feature = "animated-models")]
            {
                let loaded =
                    crate::animated_bindings::load_asset_from_project(project, package, asset)?;
                Ok(LoadedAsset {
                    package: loaded.package().to_owned(),
                    asset: loaded.asset().to_owned(),
                    package_digest: loaded.package_digest().to_owned(),
                    source_hash: loaded.source_hash().to_owned(),
                    model: LoadedModel::Animated(Arc::clone(loaded.model())),
                })
            }
            #[cfg(not(feature = "animated-models"))]
            {
                Err("animated model bindings require the animated-models feature".into())
            }
        }
    }
}

fn load_static_asset_from_project(
    project: &Project,
    package: &str,
    asset: &str,
) -> Result<LoadedAsset, String> {
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
        model: LoadedModel::Static(Arc::new(model)),
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
    let loaded = load_asset_for_kind(project_root, &binding.package, &binding.asset, binding.kind)?;
    binding.validate(&loaded)?;
    Ok(loaded)
}

/// Resolve a persisted binding using an explicitly admitted project.
/// Stale identities are rejected before replacement content is decoded.
pub fn load_binding_from_project(
    project: &Project,
    binding: &Binding,
) -> Result<LoadedAsset, String> {
    binding.validate_fields()?;
    // Diagnose changed identity before importing any replacement bytes, even if
    // the replacement is malformed or no longer a supported model.
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
    let loaded = load_asset_from_project(project, &binding.package, &binding.asset, binding.kind)?;
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
