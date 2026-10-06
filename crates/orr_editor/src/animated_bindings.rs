//! Versioned, presentation-only skeletal-animation bindings for persistent scene GUIDs.
//!
//! This document is an independent sidecar. It never writes animation state into
//! the deterministic scene or host. Installed package data is read through
//! `orr_package::Project`; the returned model is immutable and reference counted.
use orr_model::{animation::AnimatedModel, Error as ModelError};
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
const MIN_PLAYBACK_SPEED: f32 = 0.05;
const MAX_PLAYBACK_SPEED: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackMode {
    Once,
    Loop,
}

/// Persisted preview configuration. Playback time/state are deliberately transient.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaybackSettings {
    pub mode: PlaybackMode,
    /// Multiplier applied to preview delta time; never written to the scene.
    pub speed: f32,
}
impl Default for PlaybackSettings {
    fn default() -> Self {
        Self {
            mode: PlaybackMode::Loop,
            speed: 1.0,
        }
    }
}
impl PlaybackSettings {
    pub fn validate(self) -> Result<(), String> {
        if !self.speed.is_finite()
            || !(MIN_PLAYBACK_SPEED..=MAX_PLAYBACK_SPEED).contains(&self.speed)
        {
            return Err(format!(
                "playback speed must be finite and between {MIN_PLAYBACK_SPEED} and {MAX_PLAYBACK_SPEED}"
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub package: String,
    /// Declared package-relative path to either a cooked animated model or source GLB/glTF.
    pub asset: String,
    /// Verified lock identity of the installed package at assignment time.
    pub package_digest: String,
    /// Verified package-file hash of the declared model/source asset.
    pub source_hash: String,
    /// Clip slot is the stable identity within the verified source, not its display name.
    pub clip_index: u32,
    pub playback: PlaybackSettings,
}
impl Binding {
    pub fn from_asset(
        package: String,
        asset: String,
        loaded: &LoadedAsset,
        clip_index: u32,
        playback: PlaybackSettings,
    ) -> Result<Self, String> {
        if package != loaded.package || asset != loaded.asset {
            return Err(
                "loaded animation asset identity does not match the requested package/path".into(),
            );
        }
        playback.validate()?;
        if loaded
            .model
            .source()
            .clips
            .get(clip_index as usize)
            .is_none()
        {
            return Err(format!("animation clip index {clip_index} does not exist"));
        }
        let binding = Self {
            package,
            asset,
            package_digest: loaded.package_digest.clone(),
            source_hash: loaded.source_hash.clone(),
            clip_index,
            playback,
        };
        binding.validate_fields()?;
        Ok(binding)
    }

    fn validate_fields(&self) -> Result<(), String> {
        validate_package_name(&self.package)?;
        validate_asset_path(&self.asset)?;
        validate_digest(&self.package_digest, "package digest")?;
        validate_digest(&self.source_hash, "source hash")?;
        if self.clip_index as usize >= orr_model::animation::MAX_CLIPS {
            return Err("animation clip index exceeds the supported clip limit".into());
        }
        self.playback.validate()
    }

    /// Rejects removed/replaced package content and invalid clip slots. Reassignment is
    /// explicit: an old clip index is never silently applied to changed source bytes.
    pub fn validate(&self, loaded: &LoadedAsset) -> Result<(), String> {
        self.validate_fields()?;
        if self.package != loaded.package || self.asset != loaded.asset {
            return Err(
                "animation binding refers to a different package asset; explicitly reassign it"
                    .into(),
            );
        }
        if self.package_digest != loaded.package_digest {
            return Err(
                "stale animation binding: installed package changed; explicitly reassign the clip"
                    .into(),
            );
        }
        if self.source_hash != loaded.source_hash {
            return Err(
                "stale animation binding: source asset changed; explicitly reassign the clip"
                    .into(),
            );
        }
        if loaded
            .model
            .source()
            .clips
            .get(self.clip_index as usize)
            .is_none()
        {
            return Err(format!(
                "animation clip index {} no longer exists; explicitly reassign the clip",
                self.clip_index
            ));
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
            formatter.write_str("a map of unique persistent scene GUIDs to animation bindings")
        }

        fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut bindings = BTreeMap::new();
            while let Some((guid, binding)) = access.next_entry::<String, Binding>()? {
                if bindings.len() >= MAX_BINDINGS && !bindings.contains_key(&guid) {
                    return Err(A::Error::custom("too many animated-model bindings"));
                }
                if bindings.insert(guid.clone(), binding).is_some() {
                    return Err(A::Error::custom(format!(
                        "duplicate animation binding GUID: {guid}"
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
            return Err("unsupported animated binding document version".into());
        }
        if !relative_hint(&self.scene) || !relative_hint(&self.project) {
            return Err("scene and project must be explicit relative local paths".into());
        }
        if self.bindings.len() > MAX_BINDINGS {
            return Err("too many animated-model bindings".into());
        }
        for (guid, binding) in &self.bindings {
            let parsed = Guid::parse(guid).map_err(|_| format!("invalid scene GUID: {guid}"))?;
            if parsed.to_string() != *guid {
                return Err(format!("noncanonical scene GUID: {guid}"));
            }
            binding
                .validate_fields()
                .map_err(|error| format!("invalid animation binding for {guid}: {error}"))?;
        }
        Ok(())
    }
}

/// One imported model's clip list, retaining duplicate/empty names as display data only.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipInfo {
    pub index: u32,
    pub name: String,
    pub duration: f32,
}

/// Immutable, verified model data suitable for sharing with preview/render code.
#[derive(Clone, Debug)]
pub struct LoadedAsset {
    package: String,
    asset: String,
    package_digest: String,
    source_hash: String,
    model: Arc<AnimatedModel>,
    clips: Vec<ClipInfo>,
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

    pub fn model(&self) -> &Arc<AnimatedModel> {
        &self.model
    }

    pub fn clips(&self) -> &[ClipInfo] {
        &self.clips
    }

    fn new(
        package: &str,
        asset: &str,
        package_digest: String,
        source_hash: String,
        model: AnimatedModel,
    ) -> Result<Self, String> {
        let clips = model
            .source()
            .clips
            .iter()
            .enumerate()
            .map(|(index, clip)| {
                let duration = clip.duration();
                if !duration.is_finite() || duration < 0.0 {
                    return Err(format!("animation clip {index} has invalid duration"));
                }
                Ok(ClipInfo {
                    index: index as u32,
                    name: clip.name.clone(),
                    duration,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            package: package.to_owned(),
            asset: asset.to_owned(),
            package_digest,
            source_hash,
            model: Arc::new(model),
            clips,
        })
    }
}

pub struct Bindings {
    pub path: PathBuf,
    document: Document,
    saved: Document,
    undo: Vec<Document>,
    redo: Vec<Document>,
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
        })
    }

    pub fn open(path: PathBuf) -> Result<Self, String> {
        let initial_metadata = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if initial_metadata.file_type().is_symlink() || !initial_metadata.is_file() {
            return Err(
                "animated binding sidecar must be a regular file, not a symlink or special file"
                    .into(),
            );
        }
        if initial_metadata.len() > MAX_BYTES {
            return Err("animated binding file exceeds byte limit".into());
        }
        let mut bytes = Vec::new();
        let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
        let opened_metadata = file.metadata().map_err(|e| e.to_string())?;
        if !opened_metadata.is_file() {
            return Err("animated binding sidecar changed to a special file while opening".into());
        }
        if opened_metadata.len() > MAX_BYTES {
            return Err("animated binding file exceeds byte limit".into());
        }
        file.take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("animated binding file exceeds byte limit".into());
        }
        let document: Document = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        document.validate()?;
        Ok(Self {
            path,
            saved: document.clone(),
            document,
            undo: Vec::new(),
            redo: Vec::new(),
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

    /// Assign or remove a selection in one transaction. Assignment requires the exact
    /// immutable verified asset used to build the binding, preventing arbitrary clip slots.
    pub fn assign_validated(
        &mut self,
        guids: &[Guid],
        binding: &Binding,
        loaded: &LoadedAsset,
    ) -> Result<(), String> {
        if guids.is_empty() {
            return Err("select at least one scene entity".into());
        }
        binding.validate(loaded)?;
        let mut next = self.document.clone();
        for guid in guids {
            next.bindings.insert(guid.to_string(), binding.clone());
        }
        next.validate()?;
        if next != self.document {
            if self.undo.len() == MAX_HISTORY {
                self.undo.remove(0);
            }
            self.undo.push(std::mem::replace(&mut self.document, next));
            self.redo.clear();
        }
        Ok(())
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
        if next != self.document {
            if self.undo.len() == MAX_HISTORY {
                self.undo.remove(0);
            }
            self.undo.push(std::mem::replace(&mut self.document, next));
            self.redo.clear();
        }
        Ok(())
    }

    pub fn undo(&mut self) {
        if let Some(previous) = self.undo.pop() {
            self.redo
                .push(std::mem::replace(&mut self.document, previous));
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.document, next));
        }
    }

    /// Atomic sidecar replacement. State/history become clean only after persist succeeds.
    pub fn save(&mut self) -> Result<(), String> {
        self.document.validate()?;
        let bytes = serde_json::to_vec_pretty(&self.document).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("animated binding file exceeds byte limit".into());
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

/// Open a package project with exactly the animation capability this preview consumes.
/// No package-provided path or host path can expand the local project root.
pub fn open_project(project_root: &Path) -> Result<Project, String> {
    let mut capabilities = BTreeSet::new();
    capabilities.insert("animation".to_owned());
    let mut runtime = Runtime::content_only();
    runtime.capabilities = capabilities;
    let project = Project::open(project_root, runtime).map_err(|e| format!("project: {e}"))?;
    project
        .verify()
        .map_err(|e| format!("package verification: {e}"))?;
    Ok(project)
}

/// Load and verify one declared animated asset from the currently installed project.
/// Raw GLB/glTF sources are imported with package-scoped resource reads, then cooked
/// and reloaded through the exact immutable runtime validation path before exposure.
pub fn load_asset(project_root: &Path, package: &str, asset: &str) -> Result<LoadedAsset, String> {
    validate_package_name(package)?;
    validate_asset_path(asset)?;
    let project = open_project(project_root)?;
    load_asset_from_project(&project, package, asset)
}

/// Load an animated asset through an already-open project. Yard uses this entry
/// with its deliberately small `models` + `animation` capability union so it can
/// mix static and skeletal packages in one project. The Arena preview continues
/// to call `load_asset`, whose project policy remains animation-only.
pub(crate) fn load_asset_from_project(
    project: &Project,
    package: &str,
    asset: &str,
) -> Result<LoadedAsset, String> {
    validate_package_name(package)?;
    validate_asset_path(asset)?;
    let before = project
        .verify()
        .map_err(|e| format!("package verification: {e}"))?;
    let locked = before
        .packages
        .get(package)
        .ok_or_else(|| format!("animation binding package was removed: {package}"))?
        .clone();
    if !locked.manifest.capabilities.contains("animation") {
        return Err(format!(
            "package {package} does not declare the animation capability"
        ));
    }
    let (package_digest, source_hash) = verified_identity(&locked, asset)?;
    let bytes = project
        .read_asset(package, asset)
        .map_err(|e| format!("animation asset {asset}: {e}"))?;
    verify_expected_asset(&locked, asset, &bytes)?;
    let after = project
        .verify()
        .map_err(|e| format!("package verification after asset read: {e}"))?;
    let after_locked = after
        .packages
        .get(package)
        .ok_or_else(|| format!("animation binding package changed during load: {package}"))?;
    let (after_package_digest, after_source_hash) = verified_identity(after_locked, asset)?;
    if package_digest != after_package_digest || source_hash != after_source_hash {
        return Err("installed animation package changed while loading; retry".into());
    }

    let model = if let Ok(cooked) = AnimatedModel::from_bytes(&bytes) {
        cooked
    } else {
        let imported = orr_model::animation_import::import_with_resolver(asset, &bytes, |uri| {
            let resolved = resolve_package_uri(asset, uri).map_err(ModelError::message)?;
            let resource = project
                .read_asset(package, &resolved)
                .map_err(|e| ModelError::message(format!("package resource {resolved}: {e}")))?;
            verify_expected_asset(&locked, &resolved, &resource).map_err(ModelError::message)?;
            Ok(resource)
        })
        .map_err(|e| format!("animated model import: {e}"))?;
        // Round-trip through the bounded cooked representation before sharing.
        let cooked = imported
            .to_bytes()
            .map_err(|e| format!("animated model cook: {e}"))?;
        AnimatedModel::from_bytes(&cooked)
            .map_err(|e| format!("cooked animated model reload: {e}"))?
    };
    // Also verify after external dependency reads and parsing, so one load cannot
    // combine content from different package-lock generations.
    let final_lock = project
        .verify()
        .map_err(|e| format!("package verification after model load: {e}"))?;
    let final_locked = final_lock
        .packages
        .get(package)
        .ok_or_else(|| format!("animation binding package changed during load: {package}"))?;
    let (final_package_digest, final_source_hash) = verified_identity(final_locked, asset)?;
    if package_digest != final_package_digest || source_hash != final_source_hash {
        return Err("installed animation package changed while loading; retry".into());
    }
    LoadedAsset::new(package, asset, package_digest, source_hash, model)
}

/// Resolve a persisted assignment. A missing package/file stays a visible error, while
/// changed content returns an explicit stale-binding diagnostic requiring reassignment.
pub fn load_binding(project_root: &Path, binding: &Binding) -> Result<LoadedAsset, String> {
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
        .ok_or_else(|| format!("animation asset was removed or is not declared: {asset}"))?;
    validate_digest(&locked.digest, "package digest")?;
    validate_digest(source_hash, "source hash")?;
    Ok((locked.digest.clone(), source_hash.clone()))
}

fn verify_expected_asset(locked: &LockedPackage, asset: &str, bytes: &[u8]) -> Result<(), String> {
    let expected = locked
        .files
        .get(asset)
        .ok_or_else(|| format!("animation package asset is not in the captured lock: {asset}"))?;
    validate_digest(expected, "captured package asset hash")?;
    verify_expected_hash(expected, bytes).map_err(|()| {
        format!("animation package asset changed during load or does not match the captured lock: {asset}")
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
