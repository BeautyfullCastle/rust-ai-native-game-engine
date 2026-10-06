//! Independent, bounded presentation sidecars for authored/imported irradiance.
//! Scene saves and ERP undo do not modify these files. Package loads return
//! copied, verified values; all edits and saves target a user-owned sidecar.
use orr_render::irradiance::{IrradianceGrid, IrradianceProvenance, MAX_IRRADIANCE_BYTES};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

const MAX_HISTORY: usize = 128;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub version: u32,
    /// The local scene filename adjacent to this independent sidecar.
    pub scene: String,
    pub grid: IrradianceGrid,
}
impl Document {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err("unsupported irradiance sidecar version".into());
        }
        if self.scene.is_empty()
            || self.scene.len() > 255
            || self.scene.contains(['/', '\\', '\0'])
            || matches!(self.scene.as_str(), "." | "..")
            || Path::new(&self.scene).components().count() != 1
        {
            return Err("irradiance scene must be one local filename".into());
        }
        self.grid.validate()
    }
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_IRRADIANCE_BYTES {
            return Err("irradiance sidecar exceeds 256 KiB".into());
        }
        let document: Self =
            serde_json::from_slice(bytes).map_err(|e| format!("irradiance sidecar JSON: {e}"))?;
        document.validate()?;
        Ok(document)
    }
    pub fn to_json(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_IRRADIANCE_BYTES {
            return Err("irradiance sidecar exceeds 256 KiB".into());
        }
        Ok(bytes)
    }
}

pub struct IrradianceBindings {
    pub path: PathBuf,
    document: Document,
    saved: Option<Document>,
    undo: Vec<Document>,
    redo: Vec<Document>,
}
impl IrradianceBindings {
    pub fn sidecar_path(scene: &Path) -> PathBuf {
        let mut name = scene.as_os_str().to_owned();
        name.push(".irradiance.json");
        PathBuf::from(name)
    }
    pub fn create(path: PathBuf, scene: String) -> Result<Self, String> {
        validate_write_target(&path)?;
        if std::fs::symlink_metadata(&path).is_ok() {
            return Err("irradiance sidecar exists; use Open irradiance".into());
        }
        let document = Document {
            version: 1,
            scene,
            grid: IrradianceGrid::default(),
        };
        document.validate()?;
        Ok(Self {
            path,
            document,
            saved: None,
            undo: Vec::new(),
            redo: Vec::new(),
        })
    }
    pub fn open(path: PathBuf) -> Result<Self, String> {
        validate_write_target(&path)?;
        let document = Document::from_json(&read_bounded(&path)?)?;
        Ok(Self {
            path,
            saved: Some(document.clone()),
            document,
            undo: Vec::new(),
            redo: Vec::new(),
        })
    }
    pub fn document(&self) -> &Document {
        &self.document
    }
    pub fn dirty(&self) -> bool {
        self.saved.as_ref() != Some(&self.document) || !self.path.exists()
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn base(&self) -> &Path {
        self.path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
    }
    /// Only caller-verified local paths are authoritative; remote paths never are.
    pub fn matches_scene(&self, scene: &str) -> bool {
        match (
            std::fs::canonicalize(self.base().join(&self.document.scene)),
            std::fs::canonicalize(scene),
        ) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }
    /// Validation precedes history/document changes, including enable toggles.
    pub fn replace_grid(&mut self, grid: IrradianceGrid) -> Result<(), String> {
        grid.validate()?;
        let next = Document {
            grid,
            ..self.document.clone()
        };
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
    pub fn import_json(&mut self, bytes: &[u8]) -> Result<(), String> {
        let mut grid = IrradianceGrid::from_json(bytes)?;
        grid.provenance = IrradianceProvenance::Imported;
        self.replace_grid(grid)
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
    /// Refuses implicit loss of unsaved work; the panel exposes explicit discard.
    pub fn reload(&mut self) -> Result<(), String> {
        if self.dirty() {
            return Err("Save or explicitly discard irradiance changes before Reload".into());
        }
        let candidate = Self::open(self.path.clone())?;
        if candidate.document.scene != self.document.scene {
            return Err("reloaded irradiance names another scene".into());
        }
        *self = candidate;
        Ok(())
    }
    /// Atomic replacement. Failure never clears dirty state or history.
    pub fn save(&mut self) -> Result<(), String> {
        let bytes = self.document.to_json()?;
        validate_write_target(&self.path)?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(self.base()).map_err(|e| e.to_string())?;
        temporary
            .write_all(&bytes)
            .and_then(|_| temporary.as_file().sync_all())
            .map_err(|e| e.to_string())?;
        temporary.persist(&self.path).map_err(|e| e.to_string())?;
        self.saved = Some(self.document.clone());
        Ok(())
    }
}

pub fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("irradiance file must be a regular file, not a link or special file".into());
    }
    if metadata.len() > MAX_IRRADIANCE_BYTES as u64 {
        return Err("irradiance file exceeds 256 KiB".into());
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("irradiance file changed while opening".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_IRRADIANCE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_IRRADIANCE_BYTES {
        return Err("irradiance file exceeds 256 KiB".into());
    }
    Ok(bytes)
}

/// Imported immutable package snapshots and authored package source trees are
/// never writable sidecar destinations. Reject links in every parent component.
fn validate_write_target(path: &Path) -> Result<(), String> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    let parent = absolute
        .parent()
        .ok_or("irradiance sidecar requires a parent directory")?;
    let mut current = PathBuf::new();
    for component in parent.components() {
        match component {
            Component::CurDir => continue,
            Component::ParentDir => {
                current.pop();
            }
            _ => current.push(component.as_os_str()),
        }
        let metadata = std::fs::symlink_metadata(&current).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("irradiance sidecar parents must be real directories".into());
        }
        if current.join("orr.package.json").exists() || current.ends_with(".orr/packages") {
            return Err(
                "irradiance edits must be saved to a user sidecar, never a package tree".into(),
            );
        }
    }
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err("irradiance sidecar must be a regular user file".into())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

pub fn open_project(root: &Path) -> Result<orr_package::Project, String> {
    let mut runtime = orr_package::Runtime::content_only();
    #[cfg(feature = "irradiance-probes")]
    runtime.capabilities.insert("irradiance-probes".into());
    #[cfg(feature = "models")]
    runtime.capabilities.insert("models".into());
    #[cfg(feature = "animated-models")]
    runtime.capabilities.insert("animation".into());
    #[cfg(feature = "sprites")]
    runtime.capabilities.insert("sprite".into());
    let project = orr_package::Project::open(root, runtime)
        .map_err(|e| format!("irradiance project: {e}"))?;
    project
        .verify()
        .map_err(|e| format!("irradiance package verification: {e}"))?;
    Ok(project)
}

pub fn load_package_grid(
    root: &Path,
    package: &str,
    asset: &str,
) -> Result<IrradianceGrid, String> {
    let project = open_project(root)?;
    let lock = project.list().map_err(|e| e.to_string())?;
    let selected = lock
        .packages
        .get(package)
        .ok_or("irradiance package is not installed")?;
    if !selected.manifest.capabilities.contains("irradiance-probes") {
        return Err("selected package must declare the irradiance-probes capability".into());
    }
    let bytes = project
        .read_asset(package, asset)
        .map_err(|e| format!("irradiance package asset: {e}"))?;
    let mut grid = IrradianceGrid::from_json(&bytes)?;
    // The document is a copied import; no writes ever target the package asset.
    grid.provenance = IrradianceProvenance::Imported;
    Ok(grid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_render::irradiance::constant_irradiance;

    #[test]
    fn independent_history_save_reload_and_failed_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let scene = temp.path().join("yard.scene.yaml");
        std::fs::write(&scene, "scene untouched").unwrap();
        let path = IrradianceBindings::sidecar_path(&scene);
        let mut bindings =
            IrradianceBindings::create(path.clone(), "yard.scene.yaml".into()).unwrap();
        assert!(bindings.dirty());
        bindings.save().unwrap();
        assert!(!bindings.dirty());
        let initial = bindings.document().clone();
        let mut grid = initial.grid.clone();
        grid.enabled = true;
        grid.coefficients = vec![constant_irradiance([1.0, 2.0, 3.0]).unwrap(); 8];
        bindings.replace_grid(grid).unwrap();
        assert!(bindings.dirty());
        let accepted = bindings.document().clone();
        assert!(bindings.import_json(b"{\"version\":44}").is_err());
        assert_eq!(bindings.document(), &accepted);
        bindings.undo();
        assert_eq!(bindings.document(), &initial);
        assert!(!bindings.dirty());
        bindings.redo();
        assert_eq!(bindings.document(), &accepted);
        assert!(bindings.dirty());
        assert!(bindings.reload().is_err());
        assert_eq!(bindings.document(), &accepted);
        bindings.save().unwrap();
        assert_eq!(
            IrradianceBindings::open(path.clone()).unwrap().document(),
            &accepted
        );
        std::fs::write(&path, b"invalid replacement").unwrap();
        assert!(bindings.reload().is_err());
        assert_eq!(bindings.document(), &accepted);
        assert_eq!(std::fs::read_to_string(scene).unwrap(), "scene untouched");
    }
    #[test]
    fn scene_identity_remains_attached_to_original_save_target() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old.yaml");
        let new = temp.path().join("new.yaml");
        std::fs::write(&old, "old").unwrap();
        std::fs::write(&new, "new").unwrap();
        let mut bindings =
            IrradianceBindings::create(IrradianceBindings::sidecar_path(&old), "old.yaml".into())
                .unwrap();
        assert!(bindings.matches_scene(old.to_str().unwrap()));
        assert!(!bindings.matches_scene(new.to_str().unwrap()));
        bindings.save().unwrap();
        assert!(!IrradianceBindings::sidecar_path(&new).exists());
    }
    #[test]
    fn package_import_is_capability_checked_verified_and_copied() {
        let temp = tempfile::tempdir().unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/irradiance_demo")
            .canonicalize()
            .unwrap();
        let install = orr_package::Project::open_for_install(
            temp.path(),
            orr_package::Runtime::content_only().engine_version,
        )
        .unwrap();
        let lock = install.install(&[source]).unwrap();
        assert!(
            orr_package::Project::open(temp.path(), orr_package::Runtime::content_only())
                .unwrap()
                .read_asset("sample-irradiance-grid", "authored-room.irradiance.json")
                .is_err()
        );
        let grid = load_package_grid(
            temp.path(),
            "sample-irradiance-grid",
            "authored-room.irradiance.json",
        )
        .unwrap();
        assert_eq!(grid.provenance, IrradianceProvenance::Imported);
        let object = temp
            .path()
            .join(".orr/packages/objects")
            .join(&lock.packages["sample-irradiance-grid"].digest);
        let file = object.join("authored-room.irradiance.json");
        let original = std::fs::read(&file).unwrap();
        assert!(
            IrradianceBindings::create(object.join("bad.irradiance.json"), "yard.yaml".into())
                .is_err()
        );
        let mut bindings = IrradianceBindings::create(
            temp.path().join("user.irradiance.json"),
            "yard.yaml".into(),
        )
        .unwrap();
        bindings.replace_grid(grid).unwrap();
        bindings.save().unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), original);
        std::fs::write(&file, b"tampered").unwrap();
        assert!(load_package_grid(
            temp.path(),
            "sample-irradiance-grid",
            "authored-room.irradiance.json"
        )
        .is_err());
        assert!(bindings.document().grid.enabled);
    }
    #[test]
    fn sidecar_rejects_oversized_unknown_and_wrong_scene_fields() {
        let document = Document {
            version: 1,
            scene: "yard.yaml".into(),
            grid: IrradianceGrid::default(),
        };
        let mut value = serde_json::to_value(&document).unwrap();
        value["extra"] = true.into();
        assert!(Document::from_json(&serde_json::to_vec(&value).unwrap()).is_err());
        assert!(Document::from_json(&vec![b' '; MAX_IRRADIANCE_BYTES + 1]).is_err());
        let mut bad = document;
        bad.scene = "../other.yaml".into();
        assert!(bad.validate().is_err());
    }
}
