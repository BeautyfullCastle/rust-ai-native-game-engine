//! Scene-owned, optional terrain authoring state. This is presentation data;
//! no terrain edits are sent to the deterministic host or scene serializer.
//!
//! Admission is transactional: a complete core history and its derived mesh,
//! canonical bytes and query are staged before any visible state is replaced.
//! Installed package assets remain read-only until explicitly copied to a new
//! scene-relative file. The package store is never a save destination.

use orr_fp::FP;
use orr_model::StaticModel;
use orr_package::{LockedPackage, Project};
use orr_terrain::{
    Edit, Surface, Terrain, TerrainDocument as CoreDocument, MAX_FILE_BYTES, MAX_SIDE,
};
use orr_terrain_view::{to_static_model, QueryMarker};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerrainSource {
    Local {
        relative_path: String,
    },
    Package {
        project_root: PathBuf,
        package: String,
        asset: String,
        package_digest: String,
        source_hash: String,
    },
}

/// Dimensions and values are validated before allocation. Numeric fields never
/// pass through a floating-point parser or a rounded display string.
#[derive(Clone, Debug)]
pub struct NewTerrain {
    pub asset_id: String,
    pub width: u32,
    pub depth: u32,
    pub origin: [FP; 2],
    pub spacing: FP,
    pub height: FP,
}
impl Default for NewTerrain {
    fn default() -> Self {
        Self {
            asset_id: "terrain.orrt".into(),
            width: 17,
            depth: 17,
            origin: [FP::ZERO; 2],
            spacing: FP::ONE,
            height: FP::ZERO,
        }
    }
}
impl NewTerrain {
    fn build(self) -> Result<Terrain, String> {
        if !(2..=MAX_SIDE).contains(&self.width) || !(2..=MAX_SIDE).contains(&self.depth) {
            return Err(format!(
                "terrain dimensions must be between 2 and {MAX_SIDE}"
            ));
        }
        Terrain::new(
            self.asset_id,
            self.width,
            self.depth,
            self.origin,
            self.spacing,
            vec![self.height; (self.width * self.depth) as usize],
            vec![false; ((self.width - 1) * (self.depth - 1)) as usize],
        )
        .map_err(|e| e.to_string())
    }
}

pub fn parse_fixed(text: &str) -> Result<FP, String> {
    orr_reflect::decimal::parse_fp(text.trim())
        .map_err(|e| format!("invalid fixed-point decimal: {e:?}"))
}
pub fn format_fixed(value: FP) -> String {
    orr_reflect::decimal::fp_to_decimal(value)
}

struct AdmittedTerrain {
    document: CoreDocument,
    source: TerrainSource,
    bytes: Vec<u8>,
    revision: [u8; 32],
    saved_bytes: Option<Vec<u8>>,
    model: Option<Arc<StaticModel>>,
    query: Option<[FP; 2]>,
    surface: Option<Surface>,
    marker: Option<QueryMarker>,
}
impl AdmittedTerrain {
    fn new(terrain: Terrain, source: TerrainSource, saved: bool) -> Result<Self, String> {
        let model = mesh(&terrain, None)?;
        let bytes = terrain.cook();
        let revision = terrain.revision();
        Ok(Self {
            document: CoreDocument::new(terrain),
            source,
            saved_bytes: saved.then(|| bytes.clone()),
            bytes,
            revision,
            model,
            query: None,
            surface: None,
            marker: None,
        })
    }

    fn dirty(&self) -> bool {
        self.saved_bytes.as_ref() != Some(&self.bytes)
    }

    /// Core apply/undo/redo are performed on a clone by the caller. Even a mesh
    /// precision or marker failure cannot alter history, dirty state or the cache.
    fn replace_document(
        &mut self,
        document: CoreDocument,
        render_admission_error: Option<&str>,
    ) -> Result<(), String> {
        if document.terrain() == self.document.terrain() {
            return Ok(());
        }
        let terrain = document.terrain();
        let (surface, marker) = query_result(terrain, self.query);
        let model = mesh(terrain, marker)?;
        check_render_admission(&model, render_admission_error)?;
        let bytes = terrain.cook();
        let revision = terrain.revision();
        self.document = document;
        self.bytes = bytes;
        self.revision = revision;
        self.model = model;
        self.surface = surface;
        self.marker = marker;
        Ok(())
    }
}

/// Exactly one optional terrain associated with one saved local Yard scene.
/// The caller supplies `None` outside local saved YardEdit. A rejected scene
/// change retains the previous association; never render it in a different scene.
#[derive(Default)]
pub struct TerrainSession {
    scene: Option<PathBuf>,
    admitted: Option<AdmittedTerrain>,
    // Supplied by the current composed viewport. Never cooked or put in history.
    render_admission_error: Option<String>,
}
impl TerrainSession {
    /// Refresh before authoring actions using the current viewport's other
    /// participants. Empty all-hole geometry needs no renderer reservation.
    pub fn set_render_admission_error(&mut self, error: Option<String>) {
        self.render_admission_error = error;
    }
    pub fn scene(&self) -> Option<&Path> {
        self.scene.as_deref()
    }
    /// Compare a caller's original path without canonicalizing away symlinks.
    pub fn scene_matches(&self, scene: Option<&Path>) -> bool {
        match scene.map(resolve_scene).transpose() {
            Ok(scene) => scene == self.scene,
            Err(_) => false,
        }
    }
    pub fn terrain(&self) -> Option<&Terrain> {
        self.admitted.as_ref().map(|a| a.document.terrain())
    }
    pub fn source(&self) -> Option<&TerrainSource> {
        self.admitted.as_ref().map(|a| &a.source)
    }
    pub fn bytes(&self) -> Option<&[u8]> {
        self.admitted.as_ref().map(|a| a.bytes.as_slice())
    }
    pub fn revision(&self) -> Option<[u8; 32]> {
        self.admitted.as_ref().map(|a| a.revision)
    }
    pub fn dirty(&self) -> bool {
        self.admitted.as_ref().is_some_and(AdmittedTerrain::dirty)
    }
    pub fn read_only(&self) -> bool {
        matches!(self.source(), Some(TerrainSource::Package { .. }))
    }
    pub fn can_undo(&self) -> bool {
        !self.read_only()
            && self
                .admitted
                .as_ref()
                .is_some_and(|a| a.document.can_undo())
    }
    pub fn can_redo(&self) -> bool {
        !self.read_only()
            && self
                .admitted
                .as_ref()
                .is_some_and(|a| a.document.can_redo())
    }
    pub fn dirty_region(&self) -> Option<orr_terrain::DirtyRegion> {
        self.admitted
            .as_ref()
            .and_then(|a| a.document.dirty_region())
    }
    pub fn model(&self) -> Option<&Arc<StaticModel>> {
        self.admitted.as_ref().and_then(|a| a.model.as_ref())
    }
    pub fn query(&self) -> Option<[FP; 2]> {
        self.admitted.as_ref().and_then(|a| a.query)
    }
    pub fn surface(&self) -> Option<Surface> {
        self.admitted.as_ref().and_then(|a| a.surface)
    }
    pub fn marker(&self) -> Option<QueryMarker> {
        self.admitted.as_ref().and_then(|a| a.marker)
    }

    pub fn set_scene(&mut self, scene: Option<&Path>) -> Result<(), String> {
        let next = scene.map(resolve_scene).transpose()?;
        if next == self.scene {
            return Ok(());
        }
        self.require_clean()?;
        self.scene = next;
        self.admitted = None;
        Ok(())
    }

    pub fn new_local(&mut self, relative: &str, config: NewTerrain) -> Result<(), String> {
        self.require_clean()?;
        let path = self.local_path(relative)?;
        require_missing(&path)?;
        let candidate = AdmittedTerrain::new(
            config.build()?,
            TerrainSource::Local {
                relative_path: relative.into(),
            },
            false,
        )?;
        check_render_admission(&candidate.model, self.render_admission_error.as_deref())?;
        self.admitted = Some(candidate);
        Ok(())
    }

    pub fn open_local(&mut self, relative: &str) -> Result<(), String> {
        self.require_clean()?;
        let path = self.local_path(relative)?;
        let bytes = read_terrain_file(&path)?;
        let terrain = Terrain::load(&bytes).map_err(|e| e.to_string())?;
        let candidate = AdmittedTerrain::new(
            terrain,
            TerrainSource::Local {
                relative_path: relative.into(),
            },
            true,
        )?;
        check_render_admission(&candidate.model, self.render_admission_error.as_deref())?;
        self.admitted = Some(candidate);
        Ok(())
    }

    pub fn open_package(
        &mut self,
        project_root: &Path,
        package: &str,
        asset: &str,
    ) -> Result<(), String> {
        self.require_clean()?;
        self.scene_base()?;
        let candidate = load_package(project_root, package, asset)?;
        check_render_admission(&candidate.model, self.render_admission_error.as_deref())?;
        self.admitted = Some(candidate);
        Ok(())
    }

    /// Copy to a new file, never overwrite an existing scene asset or package.
    /// Reverify the installed identity before copying the admitted canonical bytes.
    pub fn copy_to_scene(&mut self, relative: &str) -> Result<(), String> {
        let path = self.local_path(relative)?;
        require_missing(&path)?;
        let admitted = self.admitted.as_ref().ok_or("open a terrain first")?;
        let TerrainSource::Package {
            project_root,
            package,
            asset,
            ..
        } = &admitted.source
        else {
            return Err("Copy to scene requires a read-only package terrain".into());
        };
        let verified = load_package(project_root, package, asset)?;
        if verified.source != admitted.source || verified.bytes != admitted.bytes {
            return Err(
                "installed terrain changed; reopen the package asset before copying".into(),
            );
        }
        atomic_write(&path, &admitted.bytes, false)?;
        let admitted = self
            .admitted
            .as_mut()
            .expect("admitted terrain checked above");
        admitted.source = TerrainSource::Local {
            relative_path: relative.into(),
        };
        admitted.saved_bytes = Some(admitted.bytes.clone());
        Ok(())
    }

    /// Writes canonical `.orrt` bytes. A failed write leaves the complete document
    /// and saved baseline unchanged. Saving does not erase undo/redo history.
    pub fn save(&mut self) -> Result<(), String> {
        self.require_editable()?;
        let admitted = self.admitted.as_ref().expect("editable terrain");
        let TerrainSource::Local { relative_path } = &admitted.source else {
            unreachable!("editable source")
        };
        let path = self.local_path(relative_path)?;
        // A new document must not clobber a file created since New was selected.
        let replace = admitted.saved_bytes.is_some();
        atomic_write(&path, &admitted.bytes, replace)?;
        let admitted = self.admitted.as_mut().expect("editable terrain");
        admitted.saved_bytes = Some(admitted.bytes.clone());
        Ok(())
    }

    pub fn apply(&mut self, edits: &[Edit]) -> Result<(), String> {
        self.require_editable()?;
        let admitted = self.admitted.as_mut().expect("editable terrain");
        let mut candidate = admitted.document.clone();
        candidate.apply(edits).map_err(|e| e.to_string())?;
        admitted.replace_document(candidate, self.render_admission_error.as_deref())
    }
    pub fn undo(&mut self) -> Result<bool, String> {
        self.require_editable()?;
        let admitted = self.admitted.as_mut().expect("editable terrain");
        let mut candidate = admitted.document.clone();
        let changed = candidate.undo();
        if changed {
            admitted.replace_document(candidate, self.render_admission_error.as_deref())?;
        }
        Ok(changed)
    }
    pub fn redo(&mut self) -> Result<bool, String> {
        self.require_editable()?;
        let admitted = self.admitted.as_mut().expect("editable terrain");
        let mut candidate = admitted.document.clone();
        let changed = candidate.redo();
        if changed {
            admitted.replace_document(candidate, self.render_admission_error.as_deref())?;
        }
        Ok(changed)
    }

    /// Queries do not dirty the asset. Mesh and marker changes are staged together.
    pub fn set_query(&mut self, query: Option<[FP; 2]>) -> Result<(), String> {
        let admitted = self.admitted.as_mut().ok_or("open a terrain first")?;
        if admitted.query == query {
            return Ok(());
        }
        let (surface, marker) = query_result(admitted.document.terrain(), query);
        let model = mesh(admitted.document.terrain(), marker)?;
        check_render_admission(&model, self.render_admission_error.as_deref())?;
        admitted.query = query;
        admitted.surface = surface;
        admitted.marker = marker;
        admitted.model = model;
        Ok(())
    }

    pub fn close(&mut self) -> Result<(), String> {
        self.require_clean()?;
        self.admitted = None;
        Ok(())
    }
    /// Call only for the user's explicit Discard choice. The scene association
    /// remains so a subsequent New/Open can reuse the saved scene's directory.
    pub fn discard(&mut self) {
        self.admitted = None;
    }

    fn require_clean(&self) -> Result<(), String> {
        if self.dirty() {
            Err("terrain has unsaved changes; Save or explicitly Discard before continuing".into())
        } else {
            Ok(())
        }
    }
    fn require_editable(&self) -> Result<(), String> {
        self.scene_base()?;
        match self.source() {
            None => Err("open a terrain first".into()),
            Some(TerrainSource::Package { .. }) => {
                Err("package terrain is read-only; use Copy to scene first".into())
            }
            Some(TerrainSource::Local { .. }) => Ok(()),
        }
    }
    fn scene_base(&self) -> Result<PathBuf, String> {
        let scene = self
            .scene
            .as_ref()
            .ok_or("terrain authoring needs a saved local Yard scene")?;
        let scene = resolve_scene(scene)?;
        Ok(scene
            .parent()
            .expect("resolved scene has parent")
            .to_owned())
    }
    fn local_path(&self, relative: &str) -> Result<PathBuf, String> {
        validate_asset_path(relative)?;
        // Package stores stay immutable even when a project is beneath the scene.
        if relative
            .split('/')
            .any(|part| part.eq_ignore_ascii_case(".orr"))
        {
            return Err("terrain cannot be saved inside a package store".into());
        }
        let base = self.scene_base()?;
        // Refuse traversal before joining, and inspect all existing components.
        let path = base.join(relative);
        real_directory(path.parent().ok_or("terrain path has no parent")?)?;
        check_optional_regular(&path)?;
        Ok(path)
    }
}

fn query_result(
    terrain: &Terrain,
    query: Option<[FP; 2]>,
) -> (Option<Surface>, Option<QueryMarker>) {
    let Some([x, z]) = query else {
        return (None, None);
    };
    (terrain.surface(x, z), QueryMarker::sample(terrain, x, z))
}
fn mesh(
    terrain: &Terrain,
    marker: Option<QueryMarker>,
) -> Result<Option<Arc<StaticModel>>, String> {
    validate_display_vertices(terrain)?;
    to_static_model(terrain, marker.as_slice())
        .map(|model| model.map(Arc::new))
        .map_err(|e| format!("terrain display: {e}"))
}

fn check_render_admission(
    model: &Option<Arc<StaticModel>>,
    error: Option<&str>,
) -> Result<(), String> {
    if let (Some(_), Some(error)) = (model, error) {
        return Err(error.to_owned());
    }
    Ok(())
}

/// The mesh bridge visits only visible triangles. Authoring also validates the
/// hidden vertices, so a hole cannot conceal an unrenderable value until later.
/// An FP raw integer is exactly representable in f32 iff it has at most 24
/// significant binary digits (division by 2^16 changes only the exponent).
fn validate_display_vertices(terrain: &Terrain) -> Result<(), String> {
    let exact = |value: FP| {
        let raw = value.raw().unsigned_abs();
        let significant = u64::BITS - raw.leading_zeros();
        significant <= 24 || raw.trailing_zeros() >= significant - 24
    };
    if !exact(terrain.spacing()) {
        return Err("terrain display requires exactly representable f32 spacing".into());
    }
    for index in 0..terrain.width() * terrain.depth() {
        for value in terrain.vertex_position(index).expect("validated vertex") {
            if value.raw().unsigned_abs() > 1_000_000 * 65_536 || !exact(value) {
                return Err("terrain display requires exact f32 coordinates within +/-1,000,000, including hole vertices".into());
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerrainAsset {
    pub package: String,
    pub asset: String,
}

/// Enumerate only explicitly declared terrain_v1 assets from a verified lock.
/// Discovery does not decode or admit the assets; selection performs full load.
pub fn list_assets(project_root: &Path) -> Result<Vec<TerrainAsset>, String> {
    let project = crate::model_bindings::open_project(project_root)?;
    let lock = project
        .verify()
        .map_err(|e| format!("package verification: {e}"))?;
    let mut result = Vec::new();
    for (package, locked) in &lock.packages {
        if !locked.manifest.capabilities.contains("terrain_v1") {
            continue;
        }
        for asset in locked.files.keys() {
            if Path::new(asset)
                .extension()
                .is_some_and(|extension| extension == "orrt")
            {
                validate_asset_path(asset)?;
                result.push(TerrainAsset {
                    package: package.clone(),
                    asset: asset.clone(),
                });
            }
        }
    }
    let after = project
        .verify()
        .map_err(|e| format!("package verification after listing: {e}"))?;
    if lock != after {
        return Err("installed packages changed while listing terrain; retry".into());
    }
    Ok(result)
}

fn load_package(
    project_root: &Path,
    package: &str,
    asset: &str,
) -> Result<AdmittedTerrain, String> {
    validate_asset_path(asset)?;
    let project_root = real_directory(project_root)?;
    let project = crate::model_bindings::open_project(&project_root)?;
    load_package_from_project(&project, project_root, package, asset)
}
fn load_package_from_project(
    project: &Project,
    project_root: PathBuf,
    package: &str,
    asset: &str,
) -> Result<AdmittedTerrain, String> {
    let before = project
        .verify()
        .map_err(|e| format!("package verification: {e}"))?;
    let locked = before
        .packages
        .get(package)
        .ok_or_else(|| format!("missing terrain package: {package}"))?;
    if !locked.manifest.capabilities.contains("terrain_v1") {
        return Err("package does not declare terrain_v1".into());
    }
    let (package_digest, source_hash) = verified_identity(locked, asset)?;
    let bytes = project
        .read_asset(package, asset)
        .map_err(|e| format!("terrain asset: {e}"))?;
    if bytes.len() > MAX_FILE_BYTES || sha256(&bytes) != source_hash {
        return Err("terrain asset changed during load or exceeds byte limit".into());
    }
    let terrain = Terrain::load(&bytes).map_err(|e| e.to_string())?;
    let candidate = AdmittedTerrain::new(
        terrain,
        TerrainSource::Package {
            project_root,
            package: package.into(),
            asset: asset.into(),
            package_digest: package_digest.clone(),
            source_hash: source_hash.clone(),
        },
        true,
    )?;
    let after = project
        .verify()
        .map_err(|e| format!("package verification after terrain read: {e}"))?;
    let after_locked = after
        .packages
        .get(package)
        .ok_or("terrain package disappeared while loading")?;
    if verified_identity(after_locked, asset)? != (package_digest, source_hash) {
        return Err("installed terrain package changed while loading; retry".into());
    }
    Ok(candidate)
}
fn verified_identity(locked: &LockedPackage, asset: &str) -> Result<(String, String), String> {
    let hash = locked
        .files
        .get(asset)
        .ok_or("terrain asset is not declared by the package")?;
    Ok((locked.digest.clone(), hash.clone()))
}
fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Portable scene-relative file paths, also matching package path rules. A file
/// cannot escape its scene directory, use a symlink, or name a reserved device.
fn validate_asset_path(relative: &str) -> Result<(), String> {
    let path = Path::new(relative);
    if relative.is_empty()
        || relative.len() > 240
        || !relative.is_ascii()
        || relative.contains('\\')
        || relative.contains(':')
        || path.is_absolute()
        || path.extension().is_none_or(|extension| extension != "orrt")
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || relative.split('/').count() > 16
        || relative.split('/').any(|part| {
            let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
            part.is_empty()
                || part == "."
                || part == ".."
                || part.len() > 100
                || part.ends_with('.')
                || !part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
                || ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
                || (stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.as_bytes()[3].is_ascii_digit())
        })
    {
        return Err(
            "terrain path must be a portable scene-relative .orrt path without traversal".into(),
        );
    }
    Ok(())
}

fn real_directory(path: &Path) -> Result<PathBuf, String> {
    let current = if path.is_absolute() {
        PathBuf::new()
    } else {
        std::env::current_dir().map_err(|e| e.to_string())?
    };
    let mut resolved = PathBuf::new();
    let mut components = current.components().chain(path.components()).peekable();
    while let Some(component) = components.next() {
        match component {
            Component::CurDir => continue,
            Component::ParentDir => {
                if !resolved.pop() {
                    return Err("terrain path escapes the filesystem root".into());
                }
            }
            _ => resolved.push(component),
        }
        if matches!(component, Component::Prefix(_))
            && components.peek() == Some(&Component::RootDir)
        {
            continue;
        }
        let metadata = fs::symlink_metadata(&resolved)
            .map_err(|e| format!("terrain directory {}: {e}", resolved.display()))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "terrain path must contain only real directories: {}",
                resolved.display()
            ));
        }
    }
    Ok(resolved)
}
fn resolve_scene(path: &Path) -> Result<PathBuf, String> {
    let name = path
        .file_name()
        .ok_or("save the local scene before opening terrain")?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let resolved = real_directory(parent)?.join(name);
    let metadata = fs::symlink_metadata(&resolved)
        .map_err(|e| format!("saved scene {}: {e}", resolved.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("terrain authoring needs a saved regular scene file, not a symlink".into());
    }
    if resolved
        .components()
        .any(|c| matches!(c, Component::Normal(name) if name.eq_ignore_ascii_case(".orr")))
    {
        return Err("terrain scene cannot live inside a package store".into());
    }
    Ok(resolved)
}
fn check_optional_regular(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(true),
        Ok(_) => Err("terrain must be a regular file, not a symlink or special file".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}
fn require_missing(path: &Path) -> Result<(), String> {
    if check_optional_regular(path)? {
        Err("terrain file already exists; use Open terrain or choose a new path".into())
    } else {
        Ok(())
    }
}
fn read_terrain_file(path: &Path) -> Result<Vec<u8>, String> {
    real_directory(path.parent().ok_or("terrain path has no parent")?)?;
    if !check_optional_regular(path)? {
        return Err("terrain file does not exist".into());
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES as u64 {
        return Err("terrain file is not regular or exceeds its byte limit".into());
    }
    // Recheck the named path after opening; no special files or symlinks are admitted.
    if !check_optional_regular(path)? {
        return Err("terrain file disappeared while opening".into());
    }
    let mut bytes = Vec::new();
    (&file)
        .take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err("terrain byte limit".into());
    }
    real_directory(path.parent().ok_or("terrain path has no parent")?)?;
    if !check_optional_regular(path)? {
        return Err("terrain file disappeared while reading".into());
    }
    // A replaced path is not the same asset that this handle read. Compare inode
    // identity on Unix as well as metadata bounds; writes themselves are atomic.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let named = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if metadata.dev() != named.dev() || metadata.ino() != named.ino() {
            return Err("terrain file was replaced while reading; retry".into());
        }
    }
    Ok(bytes)
}
fn atomic_write(path: &Path, bytes: &[u8], replace: bool) -> Result<(), String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err("terrain byte limit".into());
    }
    let parent = real_directory(path.parent().ok_or("terrain path has no parent")?)?;
    if replace {
        check_optional_regular(path)?;
    } else {
        require_missing(path)?;
    }
    let mut temp = tempfile::NamedTempFile::new_in(&parent).map_err(|e| e.to_string())?;
    temp.write_all(bytes)
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    // Repeat path checks immediately before the atomic same-directory replacement.
    real_directory(&parent)?;
    if replace {
        check_optional_regular(path)?;
        temp.persist(path).map_err(|e| e.to_string())?;
    } else {
        require_missing(path)?;
        temp.persist_noclobber(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_terrain_view::package::{install_and_reload, ASSET_PATH, PACKAGE_NAME};

    fn fixture() -> (tempfile::TempDir, PathBuf, TerrainSession) {
        // Resolve the trusted system temporary root before making test fixtures;
        // user-supplied paths below it must still reject every symlink.
        let root = fs::canonicalize(std::env::temp_dir()).unwrap();
        let temp = tempfile::tempdir_in(root).unwrap();
        let scene = temp.path().join("yard.scene");
        fs::write(&scene, "fixture local saved scene").unwrap();
        let mut session = TerrainSession::default();
        session.set_scene(Some(&scene)).unwrap();
        (temp, scene, session)
    }
    fn config() -> NewTerrain {
        NewTerrain {
            width: 3,
            depth: 3,
            ..NewTerrain::default()
        }
    }
    fn height(value: FP) -> Edit {
        Edit::SetHeight {
            x: 1,
            z: 1,
            height: value,
        }
    }
    struct Snapshot {
        bytes: Vec<u8>,
        source: TerrainSource,
        revision: [u8; 32],
        dirty: bool,
        undo: bool,
        redo: bool,
        region: Option<orr_terrain::DirtyRegion>,
        model: Option<Arc<StaticModel>>,
        query: Option<[FP; 2]>,
        surface: Option<Surface>,
        marker: Option<QueryMarker>,
    }
    impl Snapshot {
        fn of(session: &TerrainSession) -> Self {
            Self {
                bytes: session.bytes().unwrap().to_vec(),
                source: session.source().unwrap().clone(),
                revision: session.revision().unwrap(),
                dirty: session.dirty(),
                undo: session.can_undo(),
                redo: session.can_redo(),
                region: session.dirty_region(),
                model: session.model().cloned(),
                query: session.query(),
                surface: session.surface(),
                marker: session.marker(),
            }
        }
        fn assert_unchanged(&self, session: &TerrainSession) {
            assert_eq!(session.bytes(), Some(self.bytes.as_slice()));
            assert_eq!(session.source(), Some(&self.source));
            assert_eq!(session.revision(), Some(self.revision));
            assert_eq!(session.dirty(), self.dirty);
            assert_eq!(session.can_undo(), self.undo);
            assert_eq!(session.can_redo(), self.redo);
            assert_eq!(session.dirty_region(), self.region);
            assert_eq!(session.query(), self.query);
            assert_eq!(session.surface(), self.surface);
            assert_eq!(session.marker(), self.marker);
            match (&self.model, session.model()) {
                (Some(before), Some(after)) => assert!(Arc::ptr_eq(before, after)),
                (None, None) => {}
                _ => panic!("model cache changed"),
            }
        }
    }

    #[test]
    fn canonical_save_reopen_and_history_return_to_saved_revision() {
        let (temp, _, mut session) = fixture();
        session.new_local("terrain.orrt", config()).unwrap();
        assert!(session.dirty());
        assert!(!temp.path().join("terrain.orrt").exists());
        session
            .apply(&[
                height(FP::from_raw(1)),
                Edit::SetHole {
                    x: 1,
                    z: 0,
                    hole: true,
                },
            ])
            .unwrap();
        let saved = session.bytes().unwrap().to_vec();
        session.save().unwrap();
        assert!(!session.dirty());
        assert_eq!(fs::read(temp.path().join("terrain.orrt")).unwrap(), saved);
        assert_eq!(Terrain::load(&saved).unwrap().cook(), saved);
        let revision = session.revision();
        assert!(session.undo().unwrap());
        assert!(session.dirty());
        assert!(session.redo().unwrap());
        assert_eq!(session.revision(), revision);
        assert!(!session.dirty());
        session.close().unwrap();
        session.open_local("terrain.orrt").unwrap();
        assert_eq!(session.bytes(), Some(saved.as_slice()));
        assert_eq!(session.revision(), revision);
        assert!(!session.can_undo());
        assert!(!session.can_redo());
        assert_eq!(session.terrain().unwrap().heights()[4].raw(), 1);
    }

    #[test]
    fn failed_core_and_mesh_edits_preserve_bytes_dirty_cache_and_redo_history() {
        let (_temp, _scene, mut session) = fixture();
        session.new_local("terrain.orrt", config()).unwrap();
        session.save().unwrap();
        session.set_query(Some([FP::ONE; 2])).unwrap();
        session.apply(&[height(FP::from_int(2))]).unwrap();
        let edited = session.bytes().unwrap().to_vec();
        session.undo().unwrap();
        let before = Snapshot::of(&session);
        assert!(session
            .apply(&[
                height(FP::ONE),
                Edit::SetHole {
                    x: 2,
                    z: 0,
                    hole: true
                }
            ])
            .is_err());
        before.assert_unchanged(&session);
        assert!(session.apply(&[height(FP::from_raw(16_777_217))]).is_err());
        before.assert_unchanged(&session);
        assert!(session.apply(&[height(FP::from_int(1_000_001))]).is_err());
        before.assert_unchanged(&session);
        assert!(session.redo().unwrap());
        assert_eq!(session.bytes(), Some(edited.as_slice()));
        assert_eq!(session.surface().unwrap().height, FP::from_int(2));
        assert_eq!(
            session.marker().unwrap().anchor(),
            [FP::ONE, FP::from_int(2), FP::ONE]
        );
    }

    #[test]
    fn failed_open_new_and_dirty_scene_switch_are_atomic() {
        let (temp, scene, mut session) = fixture();
        session.new_local("terrain.orrt", config()).unwrap();
        session.save().unwrap();
        session.apply(&[height(FP::ONE)]).unwrap();
        session.save().unwrap();
        session.set_query(Some([FP::ONE; 2])).unwrap();
        fs::write(temp.path().join("bad.orrt"), b"not terrain").unwrap();
        let before = Snapshot::of(&session);
        assert!(session.open_local("bad.orrt").is_err());
        before.assert_unchanged(&session);
        assert!(session
            .new_local(
                "new.orrt",
                NewTerrain {
                    width: 130,
                    ..config()
                }
            )
            .is_err());
        before.assert_unchanged(&session);
        let too_precise = Terrain::new(
            "precise.orrt".into(),
            2,
            2,
            [FP::ZERO; 2],
            FP::ONE,
            vec![FP::from_raw(16_777_217); 4],
            vec![false],
        )
        .unwrap();
        fs::write(temp.path().join("precise.orrt"), too_precise.cook()).unwrap();
        assert!(session.open_local("precise.orrt").is_err());
        before.assert_unchanged(&session);
        session.apply(&[height(FP::from_int(2))]).unwrap();
        let dirty = Snapshot::of(&session);
        let other_scene = temp.path().join("other.scene");
        fs::write(&other_scene, "another saved scene").unwrap();
        assert!(session.set_scene(Some(&other_scene)).is_err());
        assert!(session.set_scene(None).is_err());
        assert!(session.close().is_err());
        assert!(session.open_local("bad.orrt").is_err());
        assert!(session.new_local("new.orrt", config()).is_err());
        dirty.assert_unchanged(&session);
        assert_eq!(session.scene(), Some(scene.as_path()));
        assert!(session.scene_matches(Some(&scene)));
        session.discard();
        session.set_scene(Some(&other_scene)).unwrap();
        assert!(session.terrain().is_none());
    }

    #[test]
    fn no_op_retains_cache_and_all_holes_have_no_model_or_query_marker() {
        let (_temp, _scene, mut session) = fixture();
        session
            .new_local(
                "terrain.orrt",
                NewTerrain {
                    width: 2,
                    depth: 2,
                    ..config()
                },
            )
            .unwrap();
        session.set_query(Some([FP::ZERO; 2])).unwrap();
        let before = Snapshot::of(&session);
        session
            .apply(&[Edit::SetHeight {
                x: 0,
                z: 0,
                height: FP::ZERO,
            }])
            .unwrap();
        before.assert_unchanged(&session);
        session
            .apply(&[Edit::SetHole {
                x: 0,
                z: 0,
                hole: true,
            }])
            .unwrap();
        assert!(session.model().is_none());
        assert!(session.marker().is_none());
        assert!(session.surface().is_none());
        let holes = Snapshot::of(&session);
        // Holes must not hide a precision failure from mesh admission.
        assert!(session.apply(&[height(FP::from_raw(16_777_217))]).is_err());
        holes.assert_unchanged(&session);
        assert!(session.undo().unwrap());
        assert!(session.model().is_some());
        assert!(session.marker().is_some());
        assert!(session.redo().unwrap());
        assert!(session.model().is_none());
    }

    #[test]
    fn exact_decimal_and_dimension_limits_are_checked_without_float_rounding() {
        for raw in [0, 1, -1, 65_535, -65_535, 16_777_217, 65_536_000_000] {
            let fp = FP::from_raw(raw);
            assert_eq!(parse_fixed(&format_fixed(fp)).unwrap().raw(), raw);
        }
        assert_eq!(parse_fixed("0.0000152587890625").unwrap().raw(), 1);
        assert!(parse_fixed("nan").is_err());
        assert!(parse_fixed("inf").is_err());
        let (_temp, _scene, mut session) = fixture();
        for (width, depth) in [(0, 2), (1, 2), (2, 1), (130, 2), (2, u32::MAX)] {
            assert!(session
                .new_local(
                    "new.orrt",
                    NewTerrain {
                        width,
                        depth,
                        ..config()
                    }
                )
                .is_err());
            assert!(session.terrain().is_none());
        }
        session
            .new_local(
                "new.orrt",
                NewTerrain {
                    width: 129,
                    depth: 129,
                    height: FP::from_raw(1),
                    ..config()
                },
            )
            .unwrap();
        assert_eq!(session.terrain().unwrap().heights().len(), 129 * 129);
        assert!(session.model().is_some());
    }

    #[test]
    fn failed_query_keeps_prior_marker_and_mesh_without_dirtying_document() {
        let (_temp, _scene, mut session) = fixture();
        session
            .new_local(
                "terrain.orrt",
                NewTerrain {
                    width: 2,
                    depth: 2,
                    spacing: FP::from_int(300),
                    ..config()
                },
            )
            .unwrap();
        session.save().unwrap();
        session.set_query(Some([FP::ONE; 2])).unwrap();
        let before = Snapshot::of(&session);
        assert!(session
            .set_query(Some([FP::from_raw(16_777_217); 2]))
            .is_err());
        before.assert_unchanged(&session);
        session.set_query(Some([FP::from_int(-1); 2])).unwrap();
        assert!(session.surface().is_none());
        assert!(session.marker().is_none());
        assert!(!session.dirty());
    }

    #[test]
    fn failed_undo_with_a_new_query_preserves_the_entire_history() {
        let (_temp, _scene, mut session) = fixture();
        let flat = FP::from_int(256);
        session
            .new_local(
                "terrain.orrt",
                NewTerrain {
                    height: flat,
                    ..config()
                },
            )
            .unwrap();
        session
            .apply(&[height(FP::from_raw(flat.raw() + 2))])
            .unwrap();
        let raised = session.bytes().unwrap().to_vec();
        session.apply(&[height(flat)]).unwrap();
        session.set_query(Some([FP::from_raw(32_768); 2])).unwrap();
        let before = Snapshot::of(&session);
        // The earlier shape has an exactly stored, but non-f32, interpolated
        // midpoint height. The undo must not pop history before marker admission.
        assert!(session.undo().is_err());
        before.assert_unchanged(&session);
        session.set_query(None).unwrap();
        assert!(session.undo().unwrap());
        assert_eq!(session.bytes(), Some(raised.as_slice()));
        assert!(session.redo().unwrap());
    }

    #[test]
    fn viewport_capacity_gate_rejects_visible_candidates_without_mutating_admission() {
        let (_temp, _scene, mut session) = fixture();
        let capacity = Some("viewport has no free model asset slot".to_owned());
        let small = NewTerrain {
            width: 2,
            depth: 2,
            ..config()
        };
        session.set_render_admission_error(capacity.clone());
        assert!(session.new_local("terrain.orrt", small.clone()).is_err());
        assert!(session.terrain().is_none());
        session.set_render_admission_error(None);
        session.new_local("terrain.orrt", small.clone()).unwrap();
        session.save().unwrap();
        session.set_query(Some([FP::ZERO; 2])).unwrap();
        let visible = Snapshot::of(&session);
        session.set_render_admission_error(capacity.clone());
        assert!(session.apply(&[height(FP::ONE)]).is_err());
        assert!(session.set_query(Some([FP::ONE; 2])).is_err());
        assert!(session.open_local("terrain.orrt").is_err());
        assert!(session.new_local("new.orrt", small).is_err());
        visible.assert_unchanged(&session);

        // Removing the final visible cell consumes no renderer slot, and can
        // still be admitted while the rest of the scene is at full capacity.
        session
            .apply(&[Edit::SetHole {
                x: 0,
                z: 0,
                hole: true,
            }])
            .unwrap();
        assert!(session.model().is_none());
        let empty = Snapshot::of(&session);
        assert!(session
            .apply(&[Edit::SetHole {
                x: 0,
                z: 0,
                hole: false
            }])
            .is_err());
        assert!(session.undo().is_err());
        empty.assert_unchanged(&session);
        session.set_query(Some([FP::ONE; 2])).unwrap();
        session.set_render_admission_error(None);
        assert!(session.undo().unwrap());
        assert!(session.model().is_some());
        session.set_render_admission_error(capacity);
        assert!(session.redo().unwrap());
        assert!(session.model().is_none());
    }

    #[test]
    fn package_copy_is_explicit_verified_and_never_writes_installed_objects() {
        let (temp, _, mut session) = fixture();
        let terrain = config().build().unwrap();
        let project = temp.path().join("project");
        let installed =
            install_and_reload(&terrain, &project, &temp.path().join("source"), "1.0.0").unwrap();
        let object = project
            .join(".orr/packages/objects")
            .join(&installed.package_digest)
            .join(ASSET_PATH);
        let installed_bytes = fs::read(&object).unwrap();
        assert_eq!(
            list_assets(&project).unwrap(),
            vec![TerrainAsset {
                package: PACKAGE_NAME.into(),
                asset: ASSET_PATH.into()
            }]
        );
        session
            .open_package(&project, PACKAGE_NAME, ASSET_PATH)
            .unwrap();
        assert!(session.read_only());
        assert!(!session.dirty());
        let before = Snapshot::of(&session);
        assert!(session.apply(&[height(FP::ONE)]).is_err());
        assert!(session.undo().is_err());
        assert!(session.redo().is_err());
        assert!(session.save().is_err());
        before.assert_unchanged(&session);
        fs::write(temp.path().join("exists.orrt"), b"do not overwrite").unwrap();
        assert!(session.copy_to_scene("exists.orrt").is_err());
        assert!(session.copy_to_scene("project/.orr/copy.orrt").is_err());
        before.assert_unchanged(&session);
        session.copy_to_scene("local.orrt").unwrap();
        assert!(!session.read_only());
        assert!(!session.dirty());
        assert_eq!(
            fs::read(temp.path().join("local.orrt")).unwrap(),
            installed_bytes
        );
        session.apply(&[height(FP::ONE)]).unwrap();
        session.save().unwrap();
        assert_ne!(
            fs::read(temp.path().join("local.orrt")).unwrap(),
            installed_bytes
        );
        assert_eq!(fs::read(object).unwrap(), installed_bytes);
    }

    #[test]
    fn compiled_yard_loaders_accept_real_model_and_terrain_packages_together() {
        let (temp, _scene, mut session) = fixture();
        let terrain = config().build().unwrap();
        let root = temp.path().join("mixed-project");
        install_and_reload(
            &terrain,
            &root,
            &temp.path().join("terrain-source"),
            "1.0.0",
        )
        .unwrap();
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        let models = assets.join("imported_scene_demo").canonicalize().unwrap();
        Project::open_for_install(&root, orr_package::Runtime::content_only().engine_version)
            .unwrap()
            .install(&[models])
            .unwrap();
        let mixed = crate::model_bindings::open_project(&root).unwrap();
        assert_eq!(mixed.verify().unwrap().packages.len(), 2);
        crate::model_bindings::load_asset(&root, "sample-imported-scene", "foreground.glb")
            .unwrap();
        session
            .open_package(&root, PACKAGE_NAME, ASSET_PATH)
            .unwrap();
        assert_eq!(session.bytes(), Some(terrain.cook().as_slice()));
        assert!(session.read_only());
        assert_eq!(list_assets(&root).unwrap().len(), 1);

        #[cfg(feature = "irradiance-probes")]
        {
            let probes = assets.join("irradiance_demo").canonicalize().unwrap();
            Project::open_for_install(&root, orr_package::Runtime::content_only().engine_version)
                .unwrap()
                .install(&[probes])
                .unwrap();
            let irradiance_project = crate::irradiance_bindings::open_project(&root).unwrap();
            assert_eq!(irradiance_project.verify().unwrap().packages.len(), 3);
            assert_eq!(
                irradiance_project
                    .read_asset(PACKAGE_NAME, ASSET_PATH)
                    .unwrap(),
                terrain.cook()
            );
            crate::model_bindings::open_project(&root).unwrap();
            session
                .open_package(&root, PACKAGE_NAME, ASSET_PATH)
                .unwrap();
            assert_eq!(session.bytes(), Some(terrain.cook().as_slice()));
        }
    }

    #[test]
    fn package_tamper_rejects_open_and_copy_preserving_previous_admission() {
        let (temp, _, mut session) = fixture();
        let terrain = config().build().unwrap();
        let project = temp.path().join("project");
        let installed =
            install_and_reload(&terrain, &project, &temp.path().join("source"), "1.0.0").unwrap();
        session
            .open_package(&project, PACKAGE_NAME, ASSET_PATH)
            .unwrap();
        session.set_query(Some([FP::ONE; 2])).unwrap();
        let before = Snapshot::of(&session);
        let object = project
            .join(".orr/packages/objects")
            .join(&installed.package_digest)
            .join(ASSET_PATH);
        let mut permissions = fs::metadata(&object).unwrap().permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(permissions.mode() | 0o200);
        }
        #[cfg(windows)]
        permissions.set_readonly(false);
        fs::set_permissions(&object, permissions).unwrap();
        fs::write(&object, b"tampered installed content").unwrap();
        assert!(session
            .open_package(&project, PACKAGE_NAME, ASSET_PATH)
            .is_err());
        before.assert_unchanged(&session);
        assert!(session.copy_to_scene("copy.orrt").is_err());
        before.assert_unchanged(&session);
        assert!(!temp.path().join("copy.orrt").exists());
        assert!(list_assets(&project).is_err());
    }

    #[test]
    fn new_save_refuses_a_file_created_after_admission() {
        let (temp, _, mut session) = fixture();
        session.new_local("terrain.orrt", config()).unwrap();
        let before = Snapshot::of(&session);
        fs::write(temp.path().join("terrain.orrt"), b"concurrent writer").unwrap();
        assert!(session.save().is_err());
        before.assert_unchanged(&session);
        assert_eq!(
            fs::read(temp.path().join("terrain.orrt")).unwrap(),
            b"concurrent writer"
        );
    }

    #[test]
    fn unsafe_paths_missing_scene_and_hidden_nonrenderable_vertices_are_rejected() {
        let (temp, _, mut session) = fixture();
        for relative in [
            "../out.orrt",
            "/out.orrt",
            "a/../out.orrt",
            "a//out.orrt",
            "a\\out.orrt",
            "a:out.orrt",
            "CON.orrt",
            "NUL.orrt",
            "file.json",
            ".orr/file.orrt",
        ] {
            assert!(session.new_local(relative, config()).is_err(), "{relative}");
        }
        let mut no_scene = TerrainSession::default();
        assert!(no_scene.new_local("new.orrt", config()).is_err());
        assert!(no_scene
            .set_scene(Some(&temp.path().join("missing.scene")))
            .is_err());
        for (origin, height) in [
            ([FP::from_raw(16_777_217); 2], FP::ZERO),
            ([FP::ZERO; 2], FP::from_raw(16_777_217)),
        ] {
            let hidden = Terrain::new(
                "hidden.orrt".into(),
                2,
                2,
                origin,
                FP::ONE,
                vec![height; 4],
                vec![true],
            )
            .unwrap();
            fs::write(temp.path().join("hidden.orrt"), hidden.cook()).unwrap();
            assert!(session.open_local("hidden.orrt").is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_files_parents_and_scene_are_rejected_and_failed_save_stays_dirty() {
        use std::os::unix::fs::symlink;
        let (temp, scene, mut session) = fixture();
        session.new_local("terrain.orrt", config()).unwrap();
        session.save().unwrap();
        let canonical = fs::read(temp.path().join("terrain.orrt")).unwrap();
        symlink(&scene, temp.path().join("alias.scene")).unwrap();
        assert!(!session.scene_matches(Some(&temp.path().join("alias.scene"))));
        assert!(session
            .set_scene(Some(&temp.path().join("alias.scene")))
            .is_err());
        symlink(
            temp.path().join("terrain.orrt"),
            temp.path().join("alias.orrt"),
        )
        .unwrap();
        let before = Snapshot::of(&session);
        assert!(session.open_local("alias.orrt").is_err());
        before.assert_unchanged(&session);
        fs::create_dir(temp.path().join("real")).unwrap();
        symlink(temp.path().join("real"), temp.path().join("linked")).unwrap();
        assert!(session.new_local("linked/new.orrt", config()).is_err());
        before.assert_unchanged(&session);
        session.apply(&[height(FP::ONE)]).unwrap();
        fs::rename(
            temp.path().join("terrain.orrt"),
            temp.path().join("preserved.orrt"),
        )
        .unwrap();
        symlink(
            temp.path().join("preserved.orrt"),
            temp.path().join("terrain.orrt"),
        )
        .unwrap();
        let dirty = Snapshot::of(&session);
        assert!(session.save().is_err());
        dirty.assert_unchanged(&session);
        assert_eq!(
            fs::read(temp.path().join("preserved.orrt")).unwrap(),
            canonical
        );
    }
}
