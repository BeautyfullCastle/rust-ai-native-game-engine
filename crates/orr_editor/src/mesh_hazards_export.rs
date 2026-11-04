//! Explicit MeshHazardsV1 bundles with optional authored camera, static art, lighting and HUD.
//! Schemas 1–5 remain unchanged; static unshaded props require schema 6.
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
const ART_FILENAME: &str = "game.scene.yaml.mesh-art.json";
const MAX_ART_BYTES: u64 = orr_model::MAX_FILE_BYTES as u64;
const CAMERA_FILENAME: &str = "game.scene.yaml.mesh-camera.json";
const MAX_CAMERA_BYTES: u64 = crate::mesh_camera::MAX_BYTES as u64;
const LIGHTING_FILENAME: &str = "game.scene.yaml.mesh-lighting.json";
const MAX_LIGHTING_BYTES: u64 = crate::mesh_lighting::MAX_BYTES as u64;
const HUD_FILENAME: &str = "game.scene.yaml.mesh-hud.json";
const MAX_HUD_BYTES: u64 = crate::mesh_hud::MAX_BYTES as u64;
const PROPS_FILENAME: &str = "game.scene.yaml.mesh-props.json";
const MAX_PROPS_BYTES: u64 = crate::mesh_props::MAX_SIDECAR_BYTES;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    art: Option<BundleFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lighting: Option<BundleFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hud: Option<BundleFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    props: Option<BundleFile>,
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
    pub art_sha256: Option<String>,
    pub lighting_sha256: Option<String>,
    pub hud_sha256: Option<String>,
    pub props_sha256: Option<String>,
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
    pub art_sha256: Option<String>,
    pub lighting_sha256: Option<String>,
    pub hud_sha256: Option<String>,
    pub props_sha256: Option<String>,
    pub platform: RuntimePlatform,
}

#[cfg(test)]
pub(crate) fn export_interactive(request: ExportRequest<'_>) -> Result<Receipt, String> {
    if !request.trusted_runtime {
        return Err("Explicitly trust the local hazard player executable before export".into());
    }
    let sources = capture_sources(request.scene)?;
    export_interactive_sources(request, &sources)
}

/// Bind an asynchronous request to camera presence and its exact original source bytes.
#[cfg(test)]
pub(crate) fn export_interactive_bound(
    request: ExportRequest<'_>,
    expected_camera: &Option<Vec<u8>>,
) -> Result<Receipt, String> {
    export_interactive_sources(
        request,
        &SourceAttachments {
            camera: expected_camera.clone(),
            art: None,
            lighting: None,
            hud: None,
            props: None,
        },
    )
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SourceAttachments {
    pub camera: Option<Vec<u8>>,
    pub art: Option<Vec<u8>>,
    pub lighting: Option<Vec<u8>>,
    pub hud: Option<Vec<u8>>,
    pub props: Option<Vec<u8>>,
}

pub(crate) fn export_interactive_sources(
    request: ExportRequest<'_>,
    expected_sources: &SourceAttachments,
) -> Result<Receipt, String> {
    export_sources_with_recheck(request, expected_sources, || {})
}

#[cfg(test)]
fn export_with_recheck(
    request: ExportRequest<'_>,
    expected_camera: &Option<Vec<u8>>,
    before_recheck: impl FnOnce(),
) -> Result<Receipt, String> {
    export_sources_with_recheck(
        request,
        &SourceAttachments {
            camera: expected_camera.clone(),
            art: None,
            lighting: None,
            hud: None,
            props: None,
        },
        before_recheck,
    )
}

fn export_sources_with_recheck(
    request: ExportRequest<'_>,
    expected_sources: &SourceAttachments,
    before_recheck: impl FnOnce(),
) -> Result<Receipt, String> {
    if !request.trusted_runtime {
        return Err("Explicitly trust the local hazard player executable before export".into());
    }
    let scene = io::checked_scene_path(request.scene)?;
    if &capture_sources(&scene)? != expected_sources {
        return Err("hazard attachment sources changed before export admission".into());
    }
    let scene_bytes = io::read_bounded(&scene, MAX_SCENE_BYTES, "hazard scene")?;
    let (checksum, graph_revision) = admit_scene(&scene_bytes)?;
    if checksum != request.expected_checksum {
        return Err("saved hazard scene differs from the expected checksum".into());
    }
    // Rebind only the exported copy. Source bytes remain unchanged and are rechecked.
    let camera_bytes = expected_sources
        .camera
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
    let art_bytes = expected_sources
        .art
        .as_deref()
        .map(|bytes| {
            let filename = scene
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("scene leaf must be Unicode")?;
            crate::mesh_art::Snapshot::decode_embedded(bytes, filename, graph_revision)?
                .encode_embedded(SCENE_FILENAME)
        })
        .transpose()?;
    let lighting_bytes = expected_sources
        .lighting
        .as_deref()
        .map(|bytes| {
            let filename = scene
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("scene leaf must be Unicode")?;
            crate::mesh_lighting::Document::decode(bytes, filename, graph_revision)?
                .encode(SCENE_FILENAME, graph_revision)
        })
        .transpose()?;
    let hud_bytes = expected_sources
        .hud
        .as_deref()
        .map(|bytes| {
            let filename = scene
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("scene leaf must be Unicode")?;
            crate::mesh_hud::Document::decode(bytes, filename, graph_revision)?
                .encode(SCENE_FILENAME, graph_revision)
        })
        .transpose()?;
    let props_bytes = expected_sources
        .props
        .as_deref()
        .map(|bytes| {
            let filename = scene
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("scene leaf must be Unicode")?;
            decode_static_props(bytes, filename, graph_revision)?
                .encode(SCENE_FILENAME, graph_revision)
        })
        .transpose()?;
    let runtime_path =
        io::checked_existing_file(request.runtime, MAX_RUNTIME_BYTES, "hazard runtime")?;
    let runtime_bytes = io::read_bounded(&runtime_path, MAX_RUNTIME_BYTES, "hazard runtime")?;
    let platform = if props_bytes.is_some() {
        runtime::validate_props_runtime(
            &runtime_bytes,
            camera_bytes.is_some(),
            art_bytes.is_some(),
            lighting_bytes.is_some(),
            hud_bytes.is_some(),
        )?
    } else if hud_bytes.is_some() {
        runtime::validate_hud_runtime(
            &runtime_bytes,
            camera_bytes.is_some(),
            art_bytes.is_some(),
            lighting_bytes.is_some(),
        )?
    } else if lighting_bytes.is_some() {
        runtime::validate_lighting_runtime(
            &runtime_bytes,
            camera_bytes.is_some(),
            art_bytes.is_some(),
        )?
    } else if art_bytes.is_some() {
        runtime::validate_art_runtime(&runtime_bytes, camera_bytes.is_some())?
    } else if camera_bytes.is_some() {
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
        schema: if props_bytes.is_some() {
            6
        } else if hud_bytes.is_some() {
            5
        } else if lighting_bytes.is_some() {
            4
        } else if art_bytes.is_some() {
            3
        } else if camera_bytes.is_some() {
            2
        } else {
            1
        },
        game_profile: GameProfile::MeshHazardsV1,
        input_profile: InputProfile::MeshClickToMoveV1,
        platform,
        scene: record(SCENE_FILENAME, &scene_bytes),
        runtime: record(platform.runtime_filename(), &runtime_bytes),
        camera: camera_bytes
            .as_deref()
            .map(|bytes| record(CAMERA_FILENAME, bytes)),
        art: art_bytes
            .as_deref()
            .map(|bytes| record(ART_FILENAME, bytes)),
        lighting: lighting_bytes
            .as_deref()
            .map(|bytes| record(LIGHTING_FILENAME, bytes)),
        hud: hud_bytes
            .as_deref()
            .map(|bytes| record(HUD_FILENAME, bytes)),
        props: props_bytes
            .as_deref()
            .map(|bytes| record(PROPS_FILENAME, bytes)),
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
    if let Some(bytes) = &art_bytes {
        io::write_new_file(&stage.path().join(ART_FILENAME), bytes)?;
    }
    if let Some(bytes) = &lighting_bytes {
        io::write_new_file(&stage.path().join(LIGHTING_FILENAME), bytes)?;
    }
    if let Some(bytes) = &hud_bytes {
        io::write_new_file(&stage.path().join(HUD_FILENAME), bytes)?;
    }
    if let Some(bytes) = &props_bytes {
        io::write_new_file(&stage.path().join(PROPS_FILENAME), bytes)?;
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
    if &capture_sources(&scene)? != expected_sources
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
        art_sha256: admitted.art_sha256,
        lighting_sha256: admitted.lighting_sha256,
        hud_sha256: admitted.hud_sha256,
        props_sha256: admitted.props_sha256,
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
    let has_camera = shape.as_object().unwrap().contains_key("camera");
    let has_art = shape.as_object().unwrap().contains_key("art");
    let has_hud = shape.as_object().unwrap().contains_key("hud");
    let hud_object = shape.get("hud").is_some_and(serde_json::Value::is_object);
    let has_lighting = shape.as_object().unwrap().contains_key("lighting");
    let camera_object = shape
        .get("camera")
        .is_some_and(serde_json::Value::is_object);
    let art_object = shape.get("art").is_some_and(serde_json::Value::is_object);
    let lighting_object = shape
        .get("lighting")
        .is_some_and(serde_json::Value::is_object);
    let has_props = shape.as_object().unwrap().contains_key("props");
    let props_object = shape.get("props").is_some_and(serde_json::Value::is_object);
    match manifest.schema {
        1 if !has_props && !has_camera && !has_art && !has_lighting && !has_hud => {}
        2 if !has_props && camera_object && !has_art && !has_lighting && !has_hud => {}
        3 if !has_props && art_object && (!has_camera || camera_object) && !has_lighting && !has_hud => {}
        4 if !has_props && lighting_object && (!has_camera || camera_object) && (!has_art || art_object) && !has_hud => {}
        5 if !has_props && hud_object && (!has_camera || camera_object) && (!has_art || art_object) && (!has_lighting || lighting_object) => {}
        6 if props_object && (!has_camera || camera_object) && (!has_art || art_object) && (!has_lighting || lighting_object) && (!has_hud || hud_object) => {}
        _ => return Err("hazard schema 1 is scene-only; schema 2 requires camera only; schema 3 requires art and optional camera; schema 4 requires lighting and optional camera/art; schema 5 requires HUD and optional camera/art/lighting; schema 6 requires static unshaded props and optional camera/art/lighting/HUD".into()),
    }
    if let Some(props) = &manifest.props {
        validate_record(props, PROPS_FILENAME, MAX_PROPS_BYTES)?;
    }
    if let Some(hud) = &manifest.hud {
        validate_record(hud, HUD_FILENAME, MAX_HUD_BYTES)?;
    }
    if let Some(lighting) = &manifest.lighting {
        validate_record(lighting, LIGHTING_FILENAME, MAX_LIGHTING_BYTES)?;
    }
    if let Some(art) = &manifest.art {
        validate_record(art, ART_FILENAME, MAX_ART_BYTES)?;
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
    if manifest.props.is_some() {
        expected.insert(PROPS_FILENAME.to_owned());
    }
    if manifest.camera.is_some() {
        expected.insert(CAMERA_FILENAME.to_owned());
    }
    if manifest.art.is_some() {
        expected.insert(ART_FILENAME.to_owned());
    }
    if manifest.hud.is_some() {
        expected.insert(HUD_FILENAME.to_owned());
    }
    if manifest.lighting.is_some() {
        expected.insert(LIGHTING_FILENAME.to_owned());
    }
    if io::inventory_names(&root, expected.len())? != expected {
        return Err(
            "hazard bundle must contain exactly its scene, manifest, runtime and declared camera/art/lighting/HUD/props"
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
    let art_bytes = manifest
        .art
        .as_ref()
        .map(|record| {
            let bytes = io::read_bounded(
                &root.join(ART_FILENAME),
                MAX_ART_BYTES,
                "bundled hazard art",
            )?;
            verify_record(record, &bytes)?;
            crate::mesh_art::Snapshot::decode_embedded(
                &bytes,
                SCENE_FILENAME,
                manifest.graph_revision,
            )?;
            Ok::<_, String>(bytes)
        })
        .transpose()?;
    let lighting_bytes = manifest
        .lighting
        .as_ref()
        .map(|record| {
            let bytes = io::read_bounded(
                &root.join(LIGHTING_FILENAME),
                MAX_LIGHTING_BYTES,
                "bundled hazard lighting",
            )?;
            verify_record(record, &bytes)?;
            crate::mesh_lighting::Document::decode(
                &bytes,
                SCENE_FILENAME,
                manifest.graph_revision,
            )?;
            Ok::<_, String>(bytes)
        })
        .transpose()?;
    let hud_bytes = manifest
        .hud
        .as_ref()
        .map(|record| {
            let bytes = io::read_bounded(
                &root.join(HUD_FILENAME),
                MAX_HUD_BYTES,
                "bundled hazard hud",
            )?;
            verify_record(record, &bytes)?;
            crate::mesh_hud::Document::decode(&bytes, SCENE_FILENAME, manifest.graph_revision)?;
            Ok::<_, String>(bytes)
        })
        .transpose()?;
    let props_bytes = manifest
        .props
        .as_ref()
        .map(|record| {
            let bytes = io::read_bounded(
                &root.join(PROPS_FILENAME),
                MAX_PROPS_BYTES,
                "bundled hazard props",
            )?;
            verify_record(record, &bytes)?;
            decode_static_props(&bytes, SCENE_FILENAME, manifest.graph_revision)?;
            Ok::<_, String>(bytes)
        })
        .transpose()?;
    let platform = if props_bytes.is_some() {
        runtime::validate_props_runtime(
            &runtime_bytes,
            camera_bytes.is_some(),
            art_bytes.is_some(),
            lighting_bytes.is_some(),
            hud_bytes.is_some(),
        )?
    } else if hud_bytes.is_some() {
        runtime::validate_hud_runtime(
            &runtime_bytes,
            camera_bytes.is_some(),
            art_bytes.is_some(),
            lighting_bytes.is_some(),
        )?
    } else if lighting_bytes.is_some() {
        runtime::validate_lighting_runtime(
            &runtime_bytes,
            camera_bytes.is_some(),
            art_bytes.is_some(),
        )?
    } else if art_bytes.is_some() {
        runtime::validate_art_runtime(&runtime_bytes, camera_bytes.is_some())?
    } else if camera_bytes.is_some() {
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
        || art_bytes
            .as_deref()
            .map(|expected| {
                io::read_bounded(
                    &root.join(ART_FILENAME),
                    MAX_ART_BYTES,
                    "hazard art recheck",
                )
                .map(|current| current != expected)
            })
            .transpose()?
            .unwrap_or(false)
        || lighting_bytes
            .as_deref()
            .map(|expected| {
                io::read_bounded(
                    &root.join(LIGHTING_FILENAME),
                    MAX_LIGHTING_BYTES,
                    "hazard lighting recheck",
                )
                .map(|current| current != expected)
            })
            .transpose()?
            .unwrap_or(false)
        || hud_bytes
            .as_deref()
            .map(|expected| {
                io::read_bounded(
                    &root.join(HUD_FILENAME),
                    MAX_HUD_BYTES,
                    "hazard hud recheck",
                )
                .map(|current| current != expected)
            })
            .transpose()?
            .unwrap_or(false)
        || props_bytes
            .as_deref()
            .map(|expected| {
                io::read_bounded(
                    &root.join(PROPS_FILENAME),
                    MAX_PROPS_BYTES,
                    "hazard props recheck",
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
        art_sha256: manifest.art.map(|record| record.sha256),
        lighting_sha256: manifest.lighting.map(|record| record.sha256),
        hud_sha256: manifest.hud.map(|record| record.sha256),
        props_sha256: manifest.props.map(|record| record.sha256),
        platform,
    })
}

/// Bundles currently have no shading capability: never silently strip authored shading.
fn decode_static_props(
    bytes: &[u8],
    filename: &str,
    revision: [u8; 32],
) -> Result<crate::mesh_props::Document, String> {
    let document = crate::mesh_props::Document::decode(bytes, filename, revision)?;
    #[cfg(feature = "mesh-prop-shading")]
    if document.props.iter().any(|prop| prop.shading.is_some()) {
        return Err("hazard bundles support only static unshaded props".into());
    }
    Ok(document)
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
#[cfg(test)]
pub(crate) fn capture_camera_source(scene: &Path) -> Result<Option<Vec<u8>>, String> {
    let sources = capture_sources(scene)?;
    if sources.art.is_some()
        || sources.lighting.is_some()
        || sources.hud.is_some()
        || sources.props.is_some()
    {
        return Err(
            "camera-only export cannot omit art, lighting, HUD or props attachments".into(),
        );
    }
    Ok(sources.camera)
}

/// Capture exact supported attachment names and bytes, including absence.
pub(crate) fn capture_sources(scene: &Path) -> Result<SourceAttachments, String> {
    let scene = io::checked_scene_path(scene)?;
    reject_sidecars(&scene)?;
    let filename = scene
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("scene leaf must be Unicode")?;
    let capture = |suffix: &str, limit, label| {
        let path = scene.with_file_name(format!("{filename}.{suffix}"));
        match fs::symlink_metadata(&path) {
            Ok(_) => io::read_bounded(&path, limit, label).map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("inspect {label}: {error}")),
        }
    };
    Ok(SourceAttachments {
        camera: capture("mesh-camera.json", MAX_CAMERA_BYTES, "hazard camera source")?,
        art: capture("mesh-art.json", MAX_ART_BYTES, "hazard art source")?,
        lighting: capture(
            "mesh-lighting.json",
            MAX_LIGHTING_BYTES,
            "hazard lighting source",
        )?,
        hud: capture("mesh-hud.json", MAX_HUD_BYTES, "hazard HUD source")?,
        props: capture("mesh-props.json", MAX_PROPS_BYTES, "hazard props source")?,
    })
}

fn reject_sidecars(scene: &Path) -> Result<(), String> {
    let leaf = scene
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("scene leaf must be Unicode")?;
    let prefix = format!("{leaf}.").to_lowercase();
    let camera = format!("{leaf}.mesh-camera.json");
    let art = format!("{leaf}.mesh-art.json");
    let props = format!("{leaf}.mesh-props.json");
    let hud = format!("{leaf}.mesh-hud.json");
    let lighting = format!("{leaf}.mesh-lighting.json");
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
            && entry.file_name() != std::ffi::OsStr::new(&art)
            && entry.file_name() != std::ffi::OsStr::new(&lighting)
            && entry.file_name() != std::ffi::OsStr::new(&hud)
            && entry.file_name() != std::ffi::OsStr::new(&props)
        {
            return Err("hazard export supports only the exact mesh-camera, mesh-art, mesh-lighting, mesh-hud and mesh-props sidecars; other presentation sidecars and adjacent scene attachments are rejected".into());
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

#[cfg(test)]
pub(crate) fn test_art_runtime(platform: RuntimePlatform) -> Vec<u8> {
    tests::art_tests::art_runtime(platform, true)
}

#[cfg(test)]
pub(crate) fn test_lighting_runtime(platform: RuntimePlatform, camera: bool, art: bool) -> Vec<u8> {
    tests::lighting_tests::lighting_runtime(platform, camera, art)
}

#[cfg(test)]
pub(crate) fn test_hud_runtime(
    platform: RuntimePlatform,
    camera: bool,
    art: bool,
    lighting: bool,
) -> Vec<u8> {
    tests::hud_tests::hud_runtime(platform, camera, art, lighting)
}

#[cfg(test)]
pub(crate) fn test_props_runtime(platform: RuntimePlatform) -> Vec<u8> {
    tests::props_tests::props_runtime(platform, false, false, false, false)
}
