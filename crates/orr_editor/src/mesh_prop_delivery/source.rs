//! Immutable saved authoring snapshots. Trusted quiescent ancestry is required.
use super::{
    io::{checked_path, Snapshot},
    manifest::*,
};
use orr_bridge::FrameView;
use orr_remote::mesh_navigation::{mesh_doc_from_yaml, mesh_from_view, PIN_NAME};
use std::{
    fs,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Admission {
    pub checksum: u64,
    pub graph_revision: [u8; 32],
}
pub(crate) fn admit_bytes(bytes: &[u8]) -> Result<Admission, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let document = mesh_doc_from_yaml(text).map_err(|e| e.to_string())?;
    if document
        .scene()
        .singletons
        .iter()
        .any(|(name, _)| name != PIN_NAME && name != "MeshStepStatus")
    {
        return Err("delivery supports only base MeshNavigation; mission, hazard and other authored profiles are unsupported".into());
    }
    let view = mesh_from_view(FrameView::of(document.frame()))?;
    Ok(Admission {
        checksum: document.frame().checksum(),
        graph_revision: view.graph.revision(),
    })
}
pub fn admit(scene: &Path) -> Result<Admission, String> {
    Ok(SavedSource::capture(scene)?.admission)
}
pub struct SavedSource {
    pub(crate) path: PathBuf,
    pub(crate) files: Vec<(String, Snapshot)>,
    pub(crate) admission: Admission,
    pub(crate) absent: Vec<PathBuf>,
}
impl SavedSource {
    pub fn capture(scene: &Path) -> Result<Self, String> {
        let path = checked_path(scene)?;
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("scene basename must be UTF-8")?
            .to_owned();
        scene_name(&name)?;
        let scene = Snapshot::read(&path, MAX_SCENE)?;
        let admission = admit_bytes(&scene.bytes)?;
        let mut files = vec![(name.clone(), scene)];
        let mut absent = Vec::new();
        let parent = path.parent().ok_or("scene parent missing")?;
        for suffix in [
            ".mesh-props.json",
            ".mesh-camera.json",
            ".mesh-lighting.json",
        ] {
            let leaf = format!("{name}{suffix}");
            let p = parent.join(&leaf);
            match fs::symlink_metadata(&p) {
                Ok(_) => {
                    let limit = if suffix == ".mesh-props.json" {
                        MAX_PROPS
                    } else {
                        MAX_SETTINGS
                    };
                    files.push((leaf, Snapshot::read(&p, limit)?));
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::NotFound && suffix != ".mesh-props.json" =>
                {
                    absent.push(p)
                }
                Err(e) => return Err(format!("required/readable authoring sidecar: {e}")),
            }
        }
        for suffix in [".mesh-art.json", ".mesh-animation.json", ".mesh-hud.json"] {
            let p = parent.join(format!("{name}{suffix}"));
            ensure_absent(&p)?;
            absent.push(p);
        }
        let props = crate::mesh_props::load(&path, admission.graph_revision)?
            .ok_or("props sidecar required")?;
        if props.0.props.is_empty() {
            return Err("at least one saved prop is required".into());
        }
        props.0.placements()?;
        check_hash(&files, &format!("{name}.mesh-props.json"), Some(props.1))?;
        let camera = crate::mesh_camera_store::load(&path, admission.graph_revision)?;
        check_hash(
            &files,
            &format!("{name}.mesh-camera.json"),
            camera.map(|(_, hash)| hash),
        )?;
        let lighting = crate::mesh_lighting_store::load(&path, admission.graph_revision)?;
        if let Some((document, _)) = &lighting {
            document.lighting()?;
            document.validate()?;
        }
        check_hash(
            &files,
            &format!("{name}.mesh-lighting.json"),
            lighting.map(|(_, hash)| hash),
        )?;
        let result = Self {
            path,
            files,
            admission,
            absent,
        };
        result.recheck()?;
        Ok(result)
    }
    pub fn scene_path(&self) -> &Path {
        &self.path
    }
    pub fn initial_checksum(&self) -> u64 {
        self.admission.checksum
    }
    pub fn graph_revision(&self) -> [u8; 32] {
        self.admission.graph_revision
    }
    pub fn recheck(&self) -> Result<(), String> {
        for (_, file) in &self.files {
            file.recheck()?;
        }
        for path in &self.absent {
            ensure_absent(path)?;
        }
        Ok(())
    }
}
fn check_hash(
    files: &[(String, Snapshot)],
    name: &str,
    admitted: Option<String>,
) -> Result<(), String> {
    if files
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, f)| hash(&f.bytes))
        != admitted
    {
        return Err("authoring changed during admission".into());
    }
    Ok(())
}
fn ensure_absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
        Ok(_) => Err(format!(
            "unsupported or newly appeared sidecar: {}",
            path.display()
        )),
    }
}
