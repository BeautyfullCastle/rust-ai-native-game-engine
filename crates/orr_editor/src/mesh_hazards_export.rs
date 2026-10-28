//! Explicit MeshHazardsV1 bundles with an optional authored camera.
//! Scene-only v1 remains unchanged; camera-bearing v2 requires a distinct capability.
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
const CAMERA_FILENAME: &str = "game.scene.yaml.mesh-camera.json";
const MAX_CAMERA_BYTES: u64 = crate::mesh_camera::MAX_BYTES as u64;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    camera: Option<BundleFile>,
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
    pub camera_sha256: Option<String>,
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
    pub camera_sha256: Option<String>,
    pub platform: RuntimePlatform,
}

pub(crate) fn export_interactive(request: ExportRequest<'_>) -> Result<Receipt, String> {
    if !request.trusted_runtime {
        return Err("Explicitly trust the local hazard player executable before export".into());
    }
    let camera = capture_camera_source(request.scene)?;
    export_interactive_bound(request, &camera)
}

/// Bind an asynchronous request to camera presence and its exact original source bytes.
pub(crate) fn export_interactive_bound(
    request: ExportRequest<'_>,
    expected_camera: &Option<Vec<u8>>,
) -> Result<Receipt, String> {
    export_with_recheck(request, expected_camera, || {})
}

fn export_with_recheck(
    request: ExportRequest<'_>,
    expected_camera: &Option<Vec<u8>>,
    before_recheck: impl FnOnce(),
) -> Result<Receipt, String> {
    if !request.trusted_runtime {
        return Err("Explicitly trust the local hazard player executable before export".into());
    }
    let scene = io::checked_scene_path(request.scene)?;
    if &capture_camera_source(&scene)? != expected_camera {
        return Err("hazard camera source changed before export admission".into());
    }
    let scene_bytes = io::read_bounded(&scene, MAX_SCENE_BYTES, "hazard scene")?;
    let (checksum, graph_revision) = admit_scene(&scene_bytes)?;
    if checksum != request.expected_checksum {
        return Err("saved hazard scene differs from the expected checksum".into());
    }
    // Rebind only the exported copy. Source bytes remain unchanged and are rechecked.
    let camera_bytes = expected_camera
        .as_deref()
        .map(|bytes| {
            let filename = scene
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("scene leaf must be Unicode")?;
            crate::mesh_camera::Document::decode(bytes, filename, graph_revision)?
                .encode(SCENE_FILENAME, graph_revision)
        })
        .transpose()?;
    let runtime_path =
        io::checked_existing_file(request.runtime, MAX_RUNTIME_BYTES, "hazard runtime")?;
    let runtime_bytes = io::read_bounded(&runtime_path, MAX_RUNTIME_BYTES, "hazard runtime")?;
    let platform = if camera_bytes.is_some() {
        runtime::validate_camera_runtime(&runtime_bytes)?
    } else {
        runtime::validate_runtime(&runtime_bytes)?
    };
    let parent = io::checked_output_parent(request.destination)?;
    let destination = parent.join(
        request
            .destination
            .file_name()
            .ok_or("destination leaf missing")?,
    );
    io::refuse_existing(&destination)?;
    let manifest = Manifest {
        schema: if camera_bytes.is_some() { 2 } else { 1 },
        game_profile: GameProfile::MeshHazardsV1,
        input_profile: InputProfile::MeshClickToMoveV1,
        platform,
        scene: record(SCENE_FILENAME, &scene_bytes),
        runtime: record(platform.runtime_filename(), &runtime_bytes),
        camera: camera_bytes
            .as_deref()
            .map(|bytes| record(CAMERA_FILENAME, bytes)),
        checksum,
        graph_revision,
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
    let stage = tempfile::Builder::new()
        .prefix(".orr-hazards-stage-")
        .tempdir_in(&parent)
        .map_err(|error| error.to_string())?;
    io::write_new_file(&stage.path().join(SCENE_FILENAME), &scene_bytes)?;
    if let Some(bytes) = &camera_bytes {
        io::write_new_file(&stage.path().join(CAMERA_FILENAME), bytes)?;
    }
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
    before_recheck();
    if &capture_camera_source(&scene)? != expected_camera
        || io::read_bounded(&scene, MAX_SCENE_BYTES, "hazard scene recheck")? != scene_bytes
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
        camera_sha256: admitted.camera_sha256,
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
    match manifest.schema {
        1 if !shape.as_object().unwrap().contains_key("camera") => {}
        2 if shape
            .get("camera")
            .is_some_and(serde_json::Value::is_object) => {}
        _ => return Err("hazard schema 1 is scene-only; schema 2 requires a camera record".into()),
    }
    if let Some(camera) = &manifest.camera {
        validate_record(camera, CAMERA_FILENAME, MAX_CAMERA_BYTES)?;
    }
    validate_record(&manifest.scene, SCENE_FILENAME, MAX_SCENE_BYTES)?;
    validate_record(
        &manifest.runtime,
        manifest.platform.runtime_filename(),
        MAX_RUNTIME_BYTES,
    )?;
    let mut expected = BTreeSet::from([
        SCENE_FILENAME.to_owned(),
        MANIFEST_FILENAME.to_owned(),
        manifest.platform.runtime_filename().to_owned(),
    ]);
    if manifest.camera.is_some() {
        expected.insert(CAMERA_FILENAME.to_owned());
    }
    if io::inventory_names(&root, expected.len())? != expected {
        return Err(
            "hazard bundle must contain exactly its scene, manifest, runtime and optional camera"
                .into(),
        );
    }
    let scene = root.join(SCENE_FILENAME);
    let runtime_path = root.join(manifest.platform.runtime_filename());
    let scene_bytes = io::read_bounded(&scene, MAX_SCENE_BYTES, "bundled hazard scene")?;
    let runtime_bytes =
        io::read_bounded(&runtime_path, MAX_RUNTIME_BYTES, "bundled hazard runtime")?;
    verify_record(&manifest.scene, &scene_bytes)?;
    verify_record(&manifest.runtime, &runtime_bytes)?;
    let camera_bytes = manifest
        .camera
        .as_ref()
        .map(|record| {
            let bytes = io::read_bounded(
                &root.join(CAMERA_FILENAME),
                MAX_CAMERA_BYTES,
                "bundled hazard camera",
            )?;
            verify_record(record, &bytes)?;
            crate::mesh_camera::Document::decode(&bytes, SCENE_FILENAME, manifest.graph_revision)?;
            Ok::<_, String>(bytes)
        })
        .transpose()?;
    let platform = if camera_bytes.is_some() {
        runtime::validate_camera_runtime(&runtime_bytes)?
    } else {
        runtime::validate_runtime(&runtime_bytes)?
    };
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
        || camera_bytes
            .as_deref()
            .map(|expected| {
                io::read_bounded(
                    &root.join(CAMERA_FILENAME),
                    MAX_CAMERA_BYTES,
                    "hazard camera recheck",
                )
                .map(|current| current != expected)
            })
            .transpose()?
            .unwrap_or(false)
        || io::inventory_names(&root, expected.len())? != expected
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
        camera_sha256: manifest.camera.map(|record| record.sha256),
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
/// Capture only the exact supported sidecar; symlinks, FIFOs, oversized data and
/// all other adjacent scene attachments fail closed through the bounded IO path.
pub(crate) fn capture_camera_source(scene: &Path) -> Result<Option<Vec<u8>>, String> {
    let scene = io::checked_scene_path(scene)?;
    reject_sidecars(&scene)?;
    let filename = scene
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("scene leaf must be Unicode")?;
    let path = scene.with_file_name(format!("{filename}.mesh-camera.json"));
    match fs::symlink_metadata(&path) {
        Ok(_) => io::read_bounded(&path, MAX_CAMERA_BYTES, "hazard camera source").map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("inspect hazard camera source: {error}")),
    }
}

fn reject_sidecars(scene: &Path) -> Result<(), String> {
    let leaf = scene
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("scene leaf must be Unicode")?;
    let prefix = format!("{leaf}.").to_lowercase();
    let camera = format!("{leaf}.mesh-camera.json");
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
            && entry.file_name() != std::ffi::OsStr::new(&camera)
        {
            return Err("hazard export supports only the exact mesh-camera sidecar; other presentation sidecars and adjacent scene attachments are rejected".into());
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

#[cfg(test)]
pub(crate) fn test_camera_runtime(platform: RuntimePlatform) -> Vec<u8> {
    runtime::valid_test_camera_runtime(platform)
}
