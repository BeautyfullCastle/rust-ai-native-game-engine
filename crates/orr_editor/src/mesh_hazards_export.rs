//! Explicit scene-only MeshHazardsV1 bundles. Never upgrades legacy admission.
use orr_bridge::FrameView;
use orr_remote::mesh_navigation::{hazards, mesh_from_view};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

mod io;
mod runtime;
#[cfg(test)]
mod tests;

pub(crate) const MANIFEST_FILENAME: &str = "orr.mesh-hazards-bundle.json";
const SCENE_FILENAME: &str = "game.scene.yaml";
const MAX_SCENE_BYTES: u64 = 1024 * 1024;
const MAX_RUNTIME_BYTES: u64 = 256 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum RuntimePlatform {
    WindowsX86_64,
    LinuxX86_64,
}
impl RuntimePlatform {
    pub(crate) fn runtime_filename(self) -> &'static str {
        match self {
            Self::WindowsX86_64 => "orr_mesh_hazards_player.exe",
            Self::LinuxX86_64 => "orr_mesh_hazards_player",
        }
    }
    pub(crate) fn is_current(self) -> bool {
        match self {
            Self::WindowsX86_64 => cfg!(all(windows, target_arch = "x86_64")),
            Self::LinuxX86_64 => cfg!(all(target_os = "linux", target_arch = "x86_64")),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum GameProfile {
    MeshHazardsV1,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum InputProfile {
    MeshClickToMoveV1,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BundleFile {
    name: String,
    bytes: u64,
    sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    game_profile: GameProfile,
    input_profile: InputProfile,
    platform: RuntimePlatform,
    scene: BundleFile,
    runtime: BundleFile,
    checksum: u64,
    graph_revision: [u8; 32],
}
pub(crate) struct ExportRequest<'a> {
    pub scene: &'a Path,
    pub expected_checksum: u64,
    pub destination: &'a Path,
    pub runtime: &'a Path,
    pub trusted_runtime: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreparedBundle {
    pub root: PathBuf,
    pub scene: PathBuf,
    pub runtime: PathBuf,
    pub checksum: u64,
    pub graph_revision: [u8; 32],
    pub runtime_sha256: String,
    pub platform: RuntimePlatform,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Receipt {
    pub root: PathBuf,
    pub scene: PathBuf,
    pub runtime: PathBuf,
    pub manifest: PathBuf,
    pub checksum: u64,
    pub graph_revision: [u8; 32],
    pub scene_sha256: String,
    pub runtime_sha256: String,
    pub platform: RuntimePlatform,
}

pub(crate) fn export_interactive(request: ExportRequest<'_>) -> Result<Receipt, String> {
    if !request.trusted_runtime {
        return Err("Explicitly trust the local hazard player executable before export".into());
    }
    let scene = io::checked_scene_path(request.scene)?;
    reject_sidecars(&scene)?;
    let scene_bytes = io::read_bounded(&scene, MAX_SCENE_BYTES, "hazard scene")?;
    let (checksum, graph_revision) = admit_scene(&scene_bytes)?;
    if checksum != request.expected_checksum {
        return Err("saved hazard scene differs from the expected checksum".into());
    }
    let runtime_path =
        io::checked_existing_file(request.runtime, MAX_RUNTIME_BYTES, "hazard runtime")?;
    let runtime_bytes = io::read_bounded(&runtime_path, MAX_RUNTIME_BYTES, "hazard runtime")?;
    let platform = runtime::validate_runtime(&runtime_bytes)?;
    let parent = io::checked_output_parent(request.destination)?;
    let destination = parent.join(
        request
            .destination
            .file_name()
            .ok_or("destination leaf missing")?,
    );
    io::refuse_existing(&destination)?;
    let manifest = Manifest {
        schema: 1,
        game_profile: GameProfile::MeshHazardsV1,
        input_profile: InputProfile::MeshClickToMoveV1,
        platform,
        scene: record(SCENE_FILENAME, &scene_bytes),
        runtime: record(platform.runtime_filename(), &runtime_bytes),
        checksum,
        graph_revision,
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
    let stage = tempfile::Builder::new()
        .prefix(".orr-hazards-stage-")
        .tempdir_in(&parent)
        .map_err(|error| error.to_string())?;
    io::write_new_file(&stage.path().join(SCENE_FILENAME), &scene_bytes)?;
    let staged_runtime = stage.path().join(platform.runtime_filename());
    io::write_new_file(&staged_runtime, &runtime_bytes)?;
    #[cfg(unix)]
    if platform == RuntimePlatform::LinuxX86_64 {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&staged_runtime, fs::Permissions::from_mode(0o755))
            .map_err(|error| error.to_string())?;
    }
    io::write_new_file(&stage.path().join(MANIFEST_FILENAME), &manifest_bytes)?;
    open_bundle(stage.path())?;
    reject_sidecars(&scene)?;
    if io::read_bounded(&scene, MAX_SCENE_BYTES, "hazard scene recheck")? != scene_bytes
        || io::read_bounded(&runtime_path, MAX_RUNTIME_BYTES, "hazard runtime recheck")?
            != runtime_bytes
        || io::checked_output_parent(request.destination)? != parent
    {
        return Err("hazard export inputs changed before publication".into());
    }
    io::publish_stage(stage, &destination)?;
    // Publication is non-replacing; a post-publication error never deletes user-visible output.
    let admitted = open_bundle(&destination)?;
    Ok(Receipt {
        root: destination.clone(),
        scene: admitted.scene,
        runtime: admitted.runtime,
        manifest: destination.join(MANIFEST_FILENAME),
        checksum,
        graph_revision,
        scene_sha256: manifest.scene.sha256,
        runtime_sha256: manifest.runtime.sha256,
        platform,
    })
}

pub(crate) fn open_bundle(root: &Path) -> Result<PreparedBundle, String> {
    let root = io::checked_directory(root)?;
    let manifest_path = root.join(MANIFEST_FILENAME);
    let manifest_bytes = io::read_bounded(&manifest_path, MAX_MANIFEST_BYTES, "hazard manifest")?;
    let shape: serde_json::Value =
        serde_json::from_slice(&manifest_bytes).map_err(|error| error.to_string())?;
    if !shape.is_object()
        || !shape.get("scene").is_some_and(serde_json::Value::is_object)
        || !shape
            .get("runtime")
            .is_some_and(serde_json::Value::is_object)
    {
        return Err("hazard manifest and file records must be JSON objects".into());
    }
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("invalid hazard manifest: {error}"))?;
    if manifest.schema != 1 {
        return Err("unsupported hazard manifest schema".into());
    }
    validate_record(&manifest.scene, SCENE_FILENAME, MAX_SCENE_BYTES)?;
    validate_record(
        &manifest.runtime,
        manifest.platform.runtime_filename(),
        MAX_RUNTIME_BYTES,
    )?;
    let expected = BTreeSet::from([
        SCENE_FILENAME.to_owned(),
        MANIFEST_FILENAME.to_owned(),
        manifest.platform.runtime_filename().to_owned(),
    ]);
    if io::inventory_names(&root, 3)? != expected {
        return Err("hazard bundle must contain exactly scene, manifest and runtime".into());
    }
    let scene = root.join(SCENE_FILENAME);
    let runtime_path = root.join(manifest.platform.runtime_filename());
    let scene_bytes = io::read_bounded(&scene, MAX_SCENE_BYTES, "bundled hazard scene")?;
    let runtime_bytes =
        io::read_bounded(&runtime_path, MAX_RUNTIME_BYTES, "bundled hazard runtime")?;
    verify_record(&manifest.scene, &scene_bytes)?;
    verify_record(&manifest.runtime, &runtime_bytes)?;
    let platform = runtime::validate_runtime(&runtime_bytes)?;
    if platform != manifest.platform {
        return Err("hazard runtime platform does not match manifest".into());
    }
    let (checksum, graph_revision) = admit_scene(&scene_bytes)?;
    if checksum != manifest.checksum || graph_revision != manifest.graph_revision {
        return Err("hazard scene admission differs from manifest".into());
    }
    if io::read_bounded(
        &manifest_path,
        MAX_MANIFEST_BYTES,
        "hazard manifest recheck",
    )? != manifest_bytes
        || io::read_bounded(&scene, MAX_SCENE_BYTES, "hazard scene recheck")? != scene_bytes
        || io::read_bounded(&runtime_path, MAX_RUNTIME_BYTES, "hazard runtime recheck")?
            != runtime_bytes
        || io::inventory_names(&root, 3)? != expected
    {
        return Err("hazard bundle changed during admission".into());
    }
    Ok(PreparedBundle {
        root,
        scene,
        runtime: runtime_path,
        checksum,
        graph_revision,
        runtime_sha256: manifest.runtime.sha256,
        platform,
    })
}

fn admit_scene(bytes: &[u8]) -> Result<(u64, [u8; 32]), String> {
    let yaml = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
    let document = hazards::hazards_doc_from_yaml(yaml)
        .map_err(|error| format!("hazard scene admission: {error}"))?;
    let frame = FrameView::of(document.frame());
    let mesh = mesh_from_view(frame)?;
    if mesh.pin.is_empty() {
        return Err("hazard bundle requires an admitted embedded mesh".into());
    }
    hazards::from_view(frame)?.ok_or("hazard bundle requires enabled hazards")?;
    Ok((document.frame().checksum(), mesh.graph.revision()))
}
fn reject_sidecars(scene: &Path) -> Result<(), String> {
    let leaf = scene
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("scene leaf must be Unicode")?;
    let prefix = format!("{leaf}.").to_lowercase();
    for (index, entry) in fs::read_dir(scene.parent().ok_or("scene parent missing")?)
        .map_err(|error| error.to_string())?
        .enumerate()
    {
        if index >= 4096 {
            return Err("scene directory exceeds attachment inspection limit".into());
        }
        let entry = entry.map_err(|error| error.to_string())?;
        if entry
            .file_name()
            .to_string_lossy()
            .to_lowercase()
            .starts_with(&prefix)
        {
            return Err("scene-only hazard export rejects all presentation sidecars and adjacent scene attachments".into());
        }
    }
    Ok(())
}
fn record(name: &str, bytes: &[u8]) -> BundleFile {
    BundleFile {
        name: name.into(),
        bytes: bytes.len() as u64,
        sha256: digest(bytes),
    }
}
fn validate_record(record: &BundleFile, name: &str, limit: u64) -> Result<(), String> {
    if record.name != name
        || record.bytes == 0
        || record.bytes > limit
        || record.sha256.len() != 64
        || !record
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("invalid hazard bundle file record".into());
    }
    Ok(())
}
fn verify_record(record: &BundleFile, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() as u64 != record.bytes || digest(bytes) != record.sha256 {
        return Err("hazard bundle file length or digest mismatch".into());
    }
    Ok(())
}
/// Hash a local runtime through the same bounded, no-follow checked-open path as export.
/// In particular, a regular leaf replaced by a FIFO must never block the worker.
pub(crate) fn runtime_file_digest(path: &Path) -> Result<String, String> {
    let bytes = io::read_bounded(path, MAX_RUNTIME_BYTES, "hazard runtime digest")?;
    Ok(digest(&bytes))
}

pub(crate) fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
