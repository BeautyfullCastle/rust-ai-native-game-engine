use crate::mesh_art;
use orr_bridge::FrameView;
use orr_remote::mesh_navigation::{mesh_doc_from_yaml, mesh_from_view};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

#[cfg(test)]
#[path = "mesh_export/tests.rs"]
mod tests;

pub(crate) const MANIFEST_FILENAME: &str = "orr.mesh-bundle.json";
const RUNTIME_FILENAME: &str = "orr_mesh_player.exe";
const MAX_PATH_BYTES: usize = 4096;
const MAX_PATH_COMPONENTS: usize = 64;
const MAX_SCENE_BYTES: u64 = 1024 * 1024;
const MAX_SCENE_LEAF_BYTES: usize = 180;
const MAX_RUNTIME_BYTES: u64 = 256 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 16 * 1024;
const SIDECAR_SUFFIX: &str = ".mesh-art.json";
const PROPS_SUFFIX: &str = ".mesh-props.json";
const CAMERA_SUFFIX: &str = ".mesh-camera.json";
const ANIMATION_SUFFIX: &str = ".mesh-animation.json";
const MAX_ANIMATION_BYTES: u64 = 32 * 1024 * 1024;
const LIGHTING_SUFFIX: &str = ".mesh-lighting.json";
const HUD_SUFFIX: &str = ".mesh-hud.json";
const MAX_HUD_BYTES: u64 = 16 * 1024;
const RUNTIME_MARKER: &[u8] = b"ORR_MESH_PLAYER_V1";
const PROPS_RUNTIME_MARKER: &[u8] = b"ORR_MESH_PROPS_V1";
const ROTATION_RUNTIME_MARKER: &[u8] = b"ORR_MESH_ROTATION_V1";
const CAMERA_RUNTIME_MARKER: &[u8] = b"ORR_MESH_CAMERA_V1";
const MATERIAL_RUNTIME_MARKER: &[u8] = b"ORR_MESH_MATERIAL_V1";
const ANIMATION_RUNTIME_MARKER: &[u8] = b"ORR_MESH_ANIMATION_V1";
const SURFACE_RUNTIME_MARKER: &[u8] = b"ORR_MESH_SURFACE_V1";
const LIGHTING_RUNTIME_MARKER: &[u8] = b"ORR_MESH_LIGHTING_V1";
const INPUT_RUNTIME_MARKER: &[u8] = b"ORR_MESH_INPUT_V1";
const FACING_RUNTIME_MARKER: &[u8] = b"ORR_MESH_FACING_V1";
const CHARACTER_RUNTIME_MARKER: &[u8] = b"ORR_MESH_CHARACTER_V1";
const HUD_RUNTIME_MARKER: &[u8] = b"ORR_MESH_HUD_V1";
const MISSION_RUNTIME_MARKER: &[u8] = b"ORR_MESH_MISSION_V1";
const MATERIAL_SLOTS_RUNTIME_MARKER: &[u8] = b"ORR_MESH_MATERIAL_SLOTS_V1";
const DENSE_PROPS_RUNTIME_MARKER: &[u8] = b"ORR_MESH_DENSE_PROPS_V1";
const RUNTIME_SECTION: &[u8; 8] = b".orrmesh";
const PROPS_RUNTIME_SECTION: &[u8; 8] = b".orrprop";
const ROTATION_RUNTIME_SECTION: &[u8; 8] = b".orrrot\0";
const CAMERA_RUNTIME_SECTION: &[u8; 8] = b".orrcam\0";
const MATERIAL_RUNTIME_SECTION: &[u8; 8] = b".orrmat\0";
const ANIMATION_RUNTIME_SECTION: &[u8; 8] = b".orranim";
const SURFACE_RUNTIME_SECTION: &[u8; 8] = b".orrsurf";
const LIGHTING_RUNTIME_SECTION: &[u8; 8] = b".orrlit\0";
const INPUT_RUNTIME_SECTION: &[u8; 8] = b".orrctrl";
const FACING_RUNTIME_SECTION: &[u8; 8] = b".orrface";
const CHARACTER_RUNTIME_SECTION: &[u8; 8] = b".orrchar";
const HUD_RUNTIME_SECTION: &[u8; 8] = b".orrhud\0";
const MISSION_RUNTIME_SECTION: &[u8; 8] = b".orrmis\0";
const MATERIAL_SLOTS_RUNTIME_SECTION: &[u8; 8] = b".orrmsl\0";
const DENSE_PROPS_RUNTIME_SECTION: &[u8; 8] = b".orrdns\0";
const LEGACY_PROP_LIMIT: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum InputProfile {
    MeshClickToMoveV1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum GameProfile {
    MeshMissionV1,
}
#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Receipt {
    pub root: PathBuf,
    pub scene: PathBuf,
    pub runtime: PathBuf,
    pub manifest: PathBuf,
    pub checksum: u64,
    pub graph_revision: [u8; 32],
    pub scene_sha256: String,
    pub sidecar_sha256: Option<String>,
    pub props_sha256: Option<String>,
    pub camera_sha256: Option<String>,
    pub animation_sha256: Option<String>,
    pub lighting_sha256: Option<String>,
    pub hud_sha256: Option<String>,
    pub runtime_sha256: String,
    pub input_profile: Option<InputProfile>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreparedBundle {
    pub scene: PathBuf,
    pub checksum: u64,
    pub graph_revision: [u8; 32],
    pub runtime_sha256: String,
    pub input_profile: Option<InputProfile>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BundleManifest {
    schema: u32,
    scene: BundleFile,
    sidecar: Option<BundleFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    props: Option<BundleFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    camera: Option<BundleFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    animation: Option<BundleFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lighting: Option<BundleFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hud: Option<BundleFile>,
    runtime: BundleFile,
    checksum: u64,
    graph_revision: [u8; 32],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input_profile: Option<InputProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    game_profile: Option<GameProfile>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BundleFile {
    name: String,
    bytes: u64,
    sha256: String,
}

#[cfg(test)]
pub(crate) fn export(
    scene: &Path,
    expected_checksum: u64,
    destination: &Path,
    runtime: &Path,
) -> Result<Receipt, String> {
    export_with_profile(scene, expected_checksum, destination, runtime, None)
}

pub(crate) fn export_interactive(
    scene: &Path,
    expected_checksum: u64,
    destination: &Path,
    runtime: &Path,
) -> Result<Receipt, String> {
    export_with_profile(
        scene,
        expected_checksum,
        destination,
        runtime,
        Some(InputProfile::MeshClickToMoveV1),
    )
}

fn export_with_profile(
    scene: &Path,
    expected_checksum: u64,
    destination: &Path,
    runtime: &Path,
    input_profile: Option<InputProfile>,
) -> Result<Receipt, String> {
    let game_profile = compiled_game_profile();
    let scene_path = checked_scene_path(scene)?;
    let scene_bytes = read_bounded(&scene_path, MAX_SCENE_BYTES, "mesh scene")?;
    let (checksum, graph_revision) = admit_scene_bytes(&scene_bytes)?;
    if checksum != expected_checksum {
        return Err("mesh scene checksum changed before export".into());
    }
    let sidecar_path = mesh_art::sidecar_path(&scene_path)?;
    let mut requires_rotation = false;
    let mut requires_material = false;
    let mut requires_material_slots = false;
    let mut requires_surface = false;
    let mut requires_facing = false;
    let mut requires_character = false;
    let mut requires_dense_props = false;
    let sidecar_bytes = match read_optional_bounded(
        &sidecar_path,
        orr_model::MAX_FILE_BYTES as u64,
        "mesh art sidecar",
    )? {
        Some(bytes) => {
            let Some((art, admitted_hash)) = mesh_art::load(&scene_path, graph_revision)? else {
                return Err("mesh art sidecar could not be admitted".into());
            };
            requires_rotation |= art.rotation() != [0.0, 0.0, 0.0, 1.0]
                || serialized_rotation_present(&bytes, false)?;
            requires_material |=
                art.tint() != [1.0, 1.0, 1.0, 1.0] || serialized_material_present(&bytes, false)?;
            requires_material_slots |= !art.material_slots().is_empty()
                || serialized_material_slots_present(&bytes, false)?;
            requires_surface |=
                art.surface().is_some() || serialized_surface_present(&bytes, false)?;
            requires_facing |= serialized_facing_present(&bytes, false)?;
            if admitted_hash != digest(&bytes) {
                return Err("mesh art sidecar changed during admission".into());
            }
            if read_bounded(
                &sidecar_path,
                orr_model::MAX_FILE_BYTES as u64,
                "mesh art sidecar recheck",
            )? != bytes
            {
                return Err("mesh art sidecar changed during admission".into());
            }
            Some(bytes)
        }
        None => None,
    };
    let props_path = crate::mesh_props::sidecar_path(&scene_path)?;
    let props_bytes =
        match read_optional_bounded(&props_path, 16 * 1024 * 1024, "mesh props sidecar")? {
            Some(bytes) => {
                let Some((document, admitted_hash)) =
                    crate::mesh_props::load(&scene_path, graph_revision)?
                else {
                    return Err("mesh props sidecar could not be admitted".into());
                };
                requires_dense_props = document.props.len() > LEGACY_PROP_LIMIT;
                requires_rotation |= document
                    .props
                    .iter()
                    .any(|prop| prop.art.rotation() != [0.0, 0.0, 0.0, 1.0])
                    || serialized_rotation_present(&bytes, true)?;
                requires_material |= document
                    .props
                    .iter()
                    .any(|prop| prop.art.tint() != [1.0, 1.0, 1.0, 1.0])
                    || serialized_material_present(&bytes, true)?;
                requires_material_slots |= document
                    .props
                    .iter()
                    .any(|prop| !prop.art.material_slots().is_empty())
                    || serialized_material_slots_present(&bytes, true)?;
                requires_surface |= document
                    .props
                    .iter()
                    .any(|prop| prop.art.surface().is_some())
                    || serialized_surface_present(&bytes, true)?;
                if admitted_hash != digest(&bytes)
                    || read_bounded(&props_path, 16 * 1024 * 1024, "mesh props sidecar recheck")?
                        != bytes
                {
                    return Err("mesh props sidecar changed during admission".into());
                }
                Some(bytes)
            }
            None => None,
        };
    let camera_path = crate::mesh_camera_store::sidecar_path(&scene_path)?;
    let camera_bytes = match read_optional_bounded(
        &camera_path,
        crate::mesh_camera::MAX_BYTES as u64,
        "mesh camera sidecar",
    )? {
        Some(bytes) => {
            let Some((camera, admitted_hash)) =
                crate::mesh_camera_store::load(&scene_path, graph_revision)?
            else {
                return Err("mesh camera sidecar could not be admitted".into());
            };
            requires_character |=
                camera.follow_offset.is_some() || serialized_camera_follow_present(&bytes)?;
            if admitted_hash != digest(&bytes)
                || read_bounded(
                    &camera_path,
                    crate::mesh_camera::MAX_BYTES as u64,
                    "mesh camera sidecar recheck",
                )? != bytes
            {
                return Err("mesh camera sidecar changed during admission".into());
            }
            Some(bytes)
        }
        None => None,
    };
    let animation_path = animation_sidecar_path(&scene_path)?;
    #[cfg(feature = "mesh-animated-models")]
    let animation_bytes: Option<Vec<u8>> = match read_optional_bounded(
        &animation_path,
        MAX_ANIMATION_BYTES,
        "mesh animation sidecar",
    )? {
        Some(bytes) => {
            let Some((animation, admitted_hash)) =
                crate::mesh_animation_store::load(&scene_path, graph_revision)?
            else {
                return Err("mesh animation sidecar could not be admitted".into());
            };
            requires_character |= animation.settings().idle_clip.is_some()
                || serialized_animation_idle_present(&bytes)?;
            if admitted_hash != digest(&bytes)
                || read_bounded(
                    &animation_path,
                    MAX_ANIMATION_BYTES,
                    "mesh animation sidecar recheck",
                )? != bytes
            {
                return Err("mesh animation sidecar changed during admission".into());
            }
            Some(bytes)
        }
        None => None,
    };
    #[cfg(not(feature = "mesh-animated-models"))]
    let animation_bytes: Option<Vec<u8>> = match read_optional_bounded(
        &animation_path,
        MAX_ANIMATION_BYTES,
        "mesh animation sidecar",
    )? {
        Some(_) => {
            return Err(
                "mesh animation sidecar is present; rebuild with mesh-animated-models support"
                    .into(),
            )
        }
        None => None,
    };
    let lighting_path = crate::mesh_lighting_store::sidecar_path(&scene_path)?;
    let lighting_bytes = match read_optional_bounded(
        &lighting_path,
        crate::mesh_lighting::MAX_BYTES as u64,
        "mesh lighting sidecar",
    )? {
        Some(bytes) => {
            let Some((_, admitted_hash)) =
                crate::mesh_lighting_store::load(&scene_path, graph_revision)?
            else {
                return Err("mesh lighting sidecar could not be admitted".into());
            };
            if admitted_hash != digest(&bytes)
                || read_bounded(
                    &lighting_path,
                    crate::mesh_lighting::MAX_BYTES as u64,
                    "mesh lighting sidecar recheck",
                )? != bytes
            {
                return Err("mesh lighting sidecar changed during admission".into());
            }
            Some(bytes)
        }
        None => None,
    };
    let hud_path = crate::mesh_hud_store::sidecar_path(&scene_path)?;
    let hud_bytes = match read_optional_bounded(&hud_path, MAX_HUD_BYTES, "mesh HUD sidecar")? {
        Some(bytes) => {
            let Some((_, admitted_hash)) =
                crate::mesh_hud_store::load(&scene_path, graph_revision)?
            else {
                return Err("mesh HUD sidecar could not be admitted".into());
            };
            if admitted_hash != digest(&bytes)
                || read_bounded(&hud_path, MAX_HUD_BYTES, "mesh HUD sidecar recheck")? != bytes
            {
                return Err("mesh HUD sidecar changed during admission".into());
            }
            Some(bytes)
        }
        None => None,
    };
    let runtime_bytes = read_bounded(runtime, MAX_RUNTIME_BYTES, "mesh player runtime")?;
    validate_runtime_with_lighting(
        &runtime_bytes,
        RuntimeRequirements {
            require_props: props_bytes.is_some(),
            require_rotation: requires_rotation,
            require_camera: camera_bytes.is_some(),
            require_material: requires_material,
            require_material_slots: requires_material_slots,
            require_animation: animation_bytes.is_some(),
            require_surface: requires_surface,
            require_lighting: lighting_bytes.is_some(),
            require_character: requires_character,
            require_hud: hud_bytes.is_some(),
        },
    )?;
    requires_facing |= optional_facing_present(props_bytes.as_deref(), true)?
        || optional_facing_present(animation_bytes.as_deref(), false)?;
    validate_runtime_facing(&runtime_bytes, requires_facing)?;
    validate_runtime_dense_props(&runtime_bytes, requires_dense_props)?;
    validate_runtime_profile(&runtime_bytes, input_profile)?;
    validate_runtime_game_profile(&runtime_bytes, game_profile)?;
    let runtime_hash = digest(&runtime_bytes);
    let scene_name = scene_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("mesh scene filename must be Unicode")?
        .to_owned();
    let sidecar_name = sidecar_bytes
        .as_ref()
        .map(|_| format!("{scene_name}{SIDECAR_SUFFIX}"));
    let props_name = props_bytes
        .as_ref()
        .map(|_| format!("{scene_name}{PROPS_SUFFIX}"));
    let camera_name = camera_bytes
        .as_ref()
        .map(|_| format!("{scene_name}{CAMERA_SUFFIX}"));
    let animation_name = animation_bytes
        .as_ref()
        .map(|_| format!("{scene_name}{ANIMATION_SUFFIX}"));
    let lighting_name = lighting_bytes
        .as_ref()
        .map(|_| format!("{scene_name}{LIGHTING_SUFFIX}"));
    let hud_name = hud_bytes
        .as_ref()
        .map(|_| format!("{scene_name}{HUD_SUFFIX}"));
    let parent = checked_output_parent(destination)?;
    let output_name = checked_leaf(
        destination
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or("bundle destination must have a Unicode directory name")?,
    )?;
    let output = parent.join(output_name);
    refuse_existing(&output)?;
    let stage = tempfile::Builder::new()
        .prefix(".orr-mesh-bundle-")
        .tempdir_in(&parent)
        .map_err(|error| format!("create bundle staging directory: {error}"))?;
    let staged_scene = stage.path().join(&scene_name);
    write_new_file(&staged_scene, &scene_bytes)?;
    if let (Some(name), Some(bytes)) = (&sidecar_name, &sidecar_bytes) {
        write_new_file(&stage.path().join(name), bytes)?;
    }
    if let (Some(name), Some(bytes)) = (&props_name, &props_bytes) {
        write_new_file(&stage.path().join(name), bytes)?;
    }
    if let (Some(name), Some(bytes)) = (&camera_name, &camera_bytes) {
        write_new_file(&stage.path().join(name), bytes)?;
    }
    if let (Some(name), Some(bytes)) = (&animation_name, &animation_bytes) {
        write_new_file(&stage.path().join(name), bytes)?;
    }
    if let (Some(name), Some(bytes)) = (&lighting_name, &lighting_bytes) {
        write_new_file(&stage.path().join(name), bytes)?;
    }
    if let (Some(name), Some(bytes)) = (&hud_name, &hud_bytes) {
        write_new_file(&stage.path().join(name), bytes)?;
    }
    write_new_file(&stage.path().join(RUNTIME_FILENAME), &runtime_bytes)?;
    let manifest = BundleManifest {
        schema: 1,
        scene: bundle_file(&scene_name, &scene_bytes),
        sidecar: sidecar_name
            .as_ref()
            .zip(sidecar_bytes.as_deref())
            .map(|(name, bytes)| bundle_file(name, bytes)),
        props: props_name
            .as_ref()
            .zip(props_bytes.as_deref())
            .map(|(name, bytes)| bundle_file(name, bytes)),
        camera: camera_name
            .as_ref()
            .zip(camera_bytes.as_deref())
            .map(|(name, bytes)| bundle_file(name, bytes)),
        animation: animation_name
            .as_ref()
            .zip(animation_bytes.as_deref())
            .map(|(name, bytes)| bundle_file(name, bytes)),
        lighting: lighting_name
            .as_ref()
            .zip(lighting_bytes.as_deref())
            .map(|(name, bytes)| bundle_file(name, bytes)),
        hud: hud_name
            .as_ref()
            .zip(hud_bytes.as_deref())
            .map(|(name, bytes)| bundle_file(name, bytes)),
        runtime: bundle_file(RUNTIME_FILENAME, &runtime_bytes),
        checksum,
        graph_revision,
        input_profile,
        game_profile,
    };
    let manifest_bytes = serde_json::to_vec(&manifest)
        .map_err(|error| format!("serialize mesh bundle manifest: {error}"))?;
    if manifest_bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("mesh bundle manifest exceeds its byte limit".into());
    }
    write_new_file(&stage.path().join(MANIFEST_FILENAME), &manifest_bytes)?;
    if read_bounded(&scene_path, MAX_SCENE_BYTES, "mesh scene recheck")? != scene_bytes
        || read_bounded(runtime, MAX_RUNTIME_BYTES, "mesh player runtime recheck")? != runtime_bytes
        || match (
            &sidecar_bytes,
            read_optional_bounded(
                &sidecar_path,
                orr_model::MAX_FILE_BYTES as u64,
                "mesh art sidecar recheck",
            )?,
        ) {
            (Some(before), Some(after)) => before != &after,
            (None, None) => false,
            _ => true,
        }
        || match (
            &props_bytes,
            read_optional_bounded(&props_path, 16 * 1024 * 1024, "mesh props sidecar recheck")?,
        ) {
            (Some(before), Some(after)) => before != &after,
            (None, None) => false,
            _ => true,
        }
        || match (
            &camera_bytes,
            read_optional_bounded(
                &camera_path,
                crate::mesh_camera::MAX_BYTES as u64,
                "mesh camera sidecar recheck",
            )?,
        ) {
            (Some(before), Some(after)) => before != &after,
            (None, None) => false,
            _ => true,
        }
        || match (
            &animation_bytes,
            read_optional_bounded(
                &animation_path,
                MAX_ANIMATION_BYTES,
                "mesh animation sidecar recheck",
            )?,
        ) {
            (Some(before), Some(after)) => before != &after,
            (None, None) => false,
            _ => true,
        }
        || match (
            &lighting_bytes,
            read_optional_bounded(
                &lighting_path,
                crate::mesh_lighting::MAX_BYTES as u64,
                "mesh lighting sidecar recheck",
            )?,
        ) {
            (Some(before), Some(after)) => before != &after,
            (None, None) => false,
            _ => true,
        }
        || match (
            &hud_bytes,
            read_optional_bounded(&hud_path, MAX_HUD_BYTES, "mesh HUD sidecar recheck")?,
        ) {
            (Some(before), Some(after)) => before != &after,
            (None, None) => false,
            _ => true,
        }
    {
        return Err("mesh bundle inputs changed before publication".into());
    }
    check_parent_chain(&parent)?;
    if fs::canonicalize(&parent).map_err(|error| error.to_string())? != parent {
        return Err("mesh bundle destination parent changed before publication".into());
    }
    refuse_existing(&output)?;
    publish_new_directory(stage.path(), &output)?;
    let sidecar_sha256 = sidecar_bytes.as_deref().map(digest);
    let props_sha256 = props_bytes.as_deref().map(digest);
    let camera_sha256 = camera_bytes.as_deref().map(digest);
    let animation_sha256 = animation_bytes.as_deref().map(digest);
    let lighting_sha256 = lighting_bytes.as_deref().map(digest);
    let hud_sha256 = hud_bytes.as_deref().map(digest);
    Ok(Receipt {
        root: output.clone(),
        scene: output.join(scene_name),
        runtime: output.join(RUNTIME_FILENAME),
        manifest: output.join(MANIFEST_FILENAME),
        checksum,
        graph_revision,
        scene_sha256: digest(&scene_bytes),
        sidecar_sha256,
        props_sha256,
        camera_sha256,
        animation_sha256,
        lighting_sha256,
        hud_sha256,
        runtime_sha256: runtime_hash,
        input_profile,
    })
}

pub(crate) fn open_bundle(root: &Path) -> Result<PreparedBundle, String> {
    let canonical_root = checked_directory(root)?;
    let manifest_path = canonical_root.join(MANIFEST_FILENAME);
    let manifest_bytes = read_bounded(&manifest_path, MAX_MANIFEST_BYTES, "bundle manifest")?;
    let manifest: BundleManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("invalid mesh bundle manifest: {error}"))?;
    validate_game_profile(manifest.game_profile)?;
    if manifest.schema != 1 {
        return Err("unsupported mesh bundle manifest schema".into());
    }
    validate_scene_record(&manifest.scene)?;
    validate_bundle_file(&manifest.runtime, RUNTIME_FILENAME, MAX_RUNTIME_BYTES)?;
    if let Some(sidecar) = &manifest.sidecar {
        validate_bundle_file(
            sidecar,
            &format!("{}{}", manifest.scene.name, SIDECAR_SUFFIX),
            orr_model::MAX_FILE_BYTES as u64,
        )?;
    }
    let props_name = format!("{}{}", manifest.scene.name, PROPS_SUFFIX);
    if let Some(props) = &manifest.props {
        validate_bundle_file(props, &props_name, 16 * 1024 * 1024)?;
    }
    let camera_name = format!("{}{}", manifest.scene.name, CAMERA_SUFFIX);
    if let Some(camera) = &manifest.camera {
        validate_bundle_file(camera, &camera_name, crate::mesh_camera::MAX_BYTES as u64)?;
    }
    let animation_name = format!("{}{}", manifest.scene.name, ANIMATION_SUFFIX);
    let lighting_name = format!("{}{}", manifest.scene.name, LIGHTING_SUFFIX);
    let hud_name = format!("{}{}", manifest.scene.name, HUD_SUFFIX);
    #[cfg(feature = "mesh-animated-models")]
    if let Some(animation) = &manifest.animation {
        validate_bundle_file(animation, &animation_name, MAX_ANIMATION_BYTES)?;
    }
    #[cfg(not(feature = "mesh-animated-models"))]
    if manifest.animation.is_some()
        || read_optional_bounded(
            &canonical_root.join(&animation_name),
            MAX_ANIMATION_BYTES,
            "bundle mesh animation sidecar",
        )?
        .is_some()
    {
        return Err(
            "bundle contains mesh animation; rebuild with mesh-animated-models support".into(),
        );
    }
    if let Some(lighting) = &manifest.lighting {
        validate_bundle_file(
            lighting,
            &lighting_name,
            crate::mesh_lighting::MAX_BYTES as u64,
        )?;
    }
    if let Some(hud) = &manifest.hud {
        validate_bundle_file(hud, &hud_name, MAX_HUD_BYTES)?;
    }
    let expected_names: BTreeSet<String> = [
        MANIFEST_FILENAME.to_owned(),
        manifest.scene.name.clone(),
        manifest.runtime.name.clone(),
    ]
    .into_iter()
    .chain(manifest.sidecar.as_ref().map(|entry| entry.name.clone()))
    .chain(manifest.props.as_ref().map(|entry| entry.name.clone()))
    .chain(manifest.camera.as_ref().map(|entry| entry.name.clone()))
    .chain(manifest.animation.as_ref().map(|entry| entry.name.clone()))
    .chain(manifest.lighting.as_ref().map(|entry| entry.name.clone()))
    .chain(manifest.hud.as_ref().map(|entry| entry.name.clone()))
    .collect();
    let actual_names = inventory_names(&canonical_root, expected_names.len())?;
    if actual_names != expected_names {
        return Err("mesh bundle contains missing or unexpected files".into());
    }
    let scene_path = canonical_root.join(&manifest.scene.name);
    let scene_bytes = read_bounded(&scene_path, MAX_SCENE_BYTES, "bundle scene")?;
    verify_file(&manifest.scene, &scene_bytes, "bundle scene")?;
    let (checksum, graph_revision) = admit_scene_bytes(&scene_bytes)?;
    if checksum != manifest.checksum {
        return Err("bundle scene checksum does not match its manifest".into());
    }
    if graph_revision != manifest.graph_revision {
        return Err("bundle mesh graph revision does not match its manifest".into());
    }
    let sidecar_path = mesh_art::sidecar_path(&scene_path)?;
    let mut requires_rotation = false;
    let mut requires_material = false;
    let mut requires_material_slots = false;
    let mut requires_surface = false;
    let mut requires_facing = false;
    let mut requires_character = false;
    let mut requires_dense_props = false;
    match (
        &manifest.sidecar,
        read_optional_bounded(
            &sidecar_path,
            orr_model::MAX_FILE_BYTES as u64,
            "bundle art sidecar",
        )?,
    ) {
        (Some(record), Some(bytes)) => {
            verify_file(record, &bytes, "bundle art sidecar")?;
            let Some((art, admitted_hash)) = mesh_art::load(&scene_path, graph_revision)? else {
                return Err("bundle art sidecar could not be admitted".into());
            };
            requires_rotation |= art.rotation() != [0.0, 0.0, 0.0, 1.0]
                || serialized_rotation_present(&bytes, false)?;
            requires_material |=
                art.tint() != [1.0, 1.0, 1.0, 1.0] || serialized_material_present(&bytes, false)?;
            requires_material_slots |= !art.material_slots().is_empty()
                || serialized_material_slots_present(&bytes, false)?;
            requires_surface |=
                art.surface().is_some() || serialized_surface_present(&bytes, false)?;
            requires_facing |= serialized_facing_present(&bytes, false)?;
            if admitted_hash != record.sha256 {
                return Err("bundle art admission hash does not match its manifest".into());
            }
            if read_bounded(
                &sidecar_path,
                orr_model::MAX_FILE_BYTES as u64,
                "bundle art sidecar recheck",
            )? != bytes
            {
                return Err("bundle art sidecar changed during admission".into());
            }
        }
        (None, None) => {}
        _ => return Err("bundle art sidecar inventory does not match its manifest".into()),
    }
    let props_path = canonical_root.join(&props_name);
    match (
        &manifest.props,
        read_optional_bounded(&props_path, 16 * 1024 * 1024, "bundle mesh props sidecar")?,
    ) {
        (Some(record), Some(bytes)) => {
            verify_file(record, &bytes, "bundle mesh props sidecar")?;
            let Some((document, admitted_hash)) =
                crate::mesh_props::load(&scene_path, graph_revision)?
            else {
                return Err("bundle mesh props sidecar could not be admitted".into());
            };
            requires_dense_props = document.props.len() > LEGACY_PROP_LIMIT;
            requires_rotation |= document
                .props
                .iter()
                .any(|prop| prop.art.rotation() != [0.0, 0.0, 0.0, 1.0])
                || serialized_rotation_present(&bytes, true)?;
            requires_material |= document
                .props
                .iter()
                .any(|prop| prop.art.tint() != [1.0, 1.0, 1.0, 1.0])
                || serialized_material_present(&bytes, true)?;
            requires_material_slots |= document
                .props
                .iter()
                .any(|prop| !prop.art.material_slots().is_empty())
                || serialized_material_slots_present(&bytes, true)?;
            requires_surface |= document
                .props
                .iter()
                .any(|prop| prop.art.surface().is_some())
                || serialized_surface_present(&bytes, true)?;
            if admitted_hash != record.sha256 {
                return Err("bundle mesh props admission hash does not match its manifest".into());
            }
            requires_facing |= serialized_facing_present(&bytes, true)?;
            if read_bounded(&props_path, 16 * 1024 * 1024, "bundle mesh props recheck")? != bytes {
                return Err("bundle mesh props sidecar changed during admission".into());
            }
        }
        (None, None) => {}
        _ => return Err("bundle mesh props sidecar inventory does not match its manifest".into()),
    }
    let camera_path = canonical_root.join(&camera_name);
    match (
        &manifest.camera,
        read_optional_bounded(
            &camera_path,
            crate::mesh_camera::MAX_BYTES as u64,
            "bundle mesh camera sidecar",
        )?,
    ) {
        (Some(record), Some(bytes)) => {
            verify_file(record, &bytes, "bundle mesh camera sidecar")?;
            let Some((camera, admitted_hash)) =
                crate::mesh_camera_store::load(&scene_path, graph_revision)?
            else {
                return Err("bundle mesh camera sidecar could not be admitted".into());
            };
            requires_character |=
                camera.follow_offset.is_some() || serialized_camera_follow_present(&bytes)?;
            if admitted_hash != record.sha256 {
                return Err("bundle mesh camera admission hash does not match its manifest".into());
            }
            if read_bounded(
                &camera_path,
                crate::mesh_camera::MAX_BYTES as u64,
                "bundle mesh camera recheck",
            )? != bytes
            {
                return Err("bundle mesh camera sidecar changed during admission".into());
            }
        }
        (None, None) => {}
        _ => return Err("bundle mesh camera sidecar inventory does not match its manifest".into()),
    }
    let animation_path = canonical_root.join(&animation_name);
    #[cfg(feature = "mesh-animated-models")]
    match (
        &manifest.animation,
        read_optional_bounded(
            &animation_path,
            MAX_ANIMATION_BYTES,
            "bundle mesh animation sidecar",
        )?,
    ) {
        (Some(record), Some(bytes)) => {
            verify_file(record, &bytes, "bundle mesh animation sidecar")?;
            requires_facing |= serialized_facing_present(&bytes, false)?;
            let Some((animation, admitted_hash)) =
                crate::mesh_animation_store::load(&scene_path, graph_revision)?
            else {
                return Err("bundle mesh animation sidecar could not be admitted".into());
            };
            requires_character |= animation.settings().idle_clip.is_some()
                || serialized_animation_idle_present(&bytes)?;
            if admitted_hash != record.sha256
                || read_bounded(
                    &animation_path,
                    MAX_ANIMATION_BYTES,
                    "bundle mesh animation recheck",
                )? != bytes
            {
                return Err(
                    "bundle mesh animation admission hash does not match its manifest".into(),
                );
            }
        }
        (None, None) => {}
        _ => {
            return Err(
                "bundle mesh animation sidecar inventory does not match its manifest".into(),
            )
        }
    }
    let lighting_path = canonical_root.join(&lighting_name);
    match (
        &manifest.lighting,
        read_optional_bounded(
            &lighting_path,
            crate::mesh_lighting::MAX_BYTES as u64,
            "bundle mesh lighting sidecar",
        )?,
    ) {
        (Some(record), Some(bytes)) => {
            verify_file(record, &bytes, "bundle mesh lighting sidecar")?;
            let Some((_, admitted_hash)) =
                crate::mesh_lighting_store::load(&scene_path, graph_revision)?
            else {
                return Err("bundle mesh lighting sidecar could not be admitted".into());
            };
            if admitted_hash != record.sha256
                || read_bounded(
                    &lighting_path,
                    crate::mesh_lighting::MAX_BYTES as u64,
                    "bundle mesh lighting recheck",
                )? != bytes
            {
                return Err(
                    "bundle mesh lighting admission hash does not match its manifest".into(),
                );
            }
        }
        (None, None) => {}
        _ => {
            return Err("bundle mesh lighting sidecar inventory does not match its manifest".into())
        }
    }
    let hud_path = canonical_root.join(&hud_name);
    match (
        &manifest.hud,
        read_optional_bounded(&hud_path, MAX_HUD_BYTES, "bundle mesh HUD sidecar")?,
    ) {
        (Some(record), Some(bytes)) => {
            verify_file(record, &bytes, "bundle mesh HUD sidecar")?;
            let Some((_, admitted_hash)) =
                crate::mesh_hud_store::load(&scene_path, graph_revision)?
            else {
                return Err("bundle mesh HUD sidecar could not be admitted".into());
            };
            if admitted_hash != record.sha256
                || read_bounded(&hud_path, MAX_HUD_BYTES, "bundle mesh HUD recheck")? != bytes
            {
                return Err("bundle mesh HUD admission hash does not match its manifest".into());
            }
        }
        (None, None) => {}
        _ => return Err("bundle mesh HUD sidecar inventory does not match its manifest".into()),
    }
    let runtime_path = canonical_root.join(RUNTIME_FILENAME);
    let runtime_bytes = read_bounded(&runtime_path, MAX_RUNTIME_BYTES, "bundle runtime")?;
    verify_file(&manifest.runtime, &runtime_bytes, "bundle runtime")?;
    validate_runtime_with_lighting(
        &runtime_bytes,
        RuntimeRequirements {
            require_props: manifest.props.is_some(),
            require_rotation: requires_rotation,
            require_camera: manifest.camera.is_some(),
            require_material: requires_material,
            require_material_slots: requires_material_slots,
            require_animation: manifest.animation.is_some(),
            require_surface: requires_surface,
            require_lighting: manifest.lighting.is_some(),
            require_character: requires_character,
            require_hud: manifest.hud.is_some(),
        },
    )?;
    validate_runtime_facing(&runtime_bytes, requires_facing)?;
    validate_runtime_dense_props(&runtime_bytes, requires_dense_props)?;
    validate_runtime_profile(&runtime_bytes, manifest.input_profile)?;
    validate_runtime_game_profile(&runtime_bytes, manifest.game_profile)?;
    let sidecar_unchanged = match &manifest.sidecar {
        Some(record) => {
            let bytes = read_bounded(
                &sidecar_path,
                orr_model::MAX_FILE_BYTES as u64,
                "bundle art sidecar final recheck",
            )?;
            verify_file(record, &bytes, "bundle art sidecar")?;
            true
        }
        None => read_optional_bounded(
            &sidecar_path,
            orr_model::MAX_FILE_BYTES as u64,
            "bundle art sidecar final recheck",
        )?
        .is_none(),
    };
    let props_unchanged = match &manifest.props {
        Some(record) => {
            let bytes = read_bounded(
                &props_path,
                16 * 1024 * 1024,
                "bundle mesh props final recheck",
            )?;
            verify_file(record, &bytes, "bundle mesh props sidecar")?;
            true
        }
        None => read_optional_bounded(
            &props_path,
            16 * 1024 * 1024,
            "bundle mesh props final recheck",
        )?
        .is_none(),
    };
    let camera_unchanged = match &manifest.camera {
        Some(record) => {
            let bytes = read_bounded(
                &camera_path,
                crate::mesh_camera::MAX_BYTES as u64,
                "bundle mesh camera final recheck",
            )?;
            verify_file(record, &bytes, "bundle mesh camera sidecar")?;
            true
        }
        None => read_optional_bounded(
            &camera_path,
            crate::mesh_camera::MAX_BYTES as u64,
            "bundle mesh camera final recheck",
        )?
        .is_none(),
    };
    #[cfg(feature = "mesh-animated-models")]
    let animation_unchanged = match &manifest.animation {
        Some(record) => {
            let bytes = read_bounded(
                &animation_path,
                MAX_ANIMATION_BYTES,
                "bundle mesh animation final recheck",
            )?;
            verify_file(record, &bytes, "bundle mesh animation sidecar")?;
            true
        }
        None => read_optional_bounded(
            &animation_path,
            MAX_ANIMATION_BYTES,
            "bundle mesh animation final recheck",
        )?
        .is_none(),
    };
    #[cfg(not(feature = "mesh-animated-models"))]
    let animation_unchanged = read_optional_bounded(
        &animation_path,
        MAX_ANIMATION_BYTES,
        "bundle mesh animation final recheck",
    )?
    .is_none();
    let lighting_unchanged = match &manifest.lighting {
        Some(record) => {
            let bytes = read_bounded(
                &lighting_path,
                crate::mesh_lighting::MAX_BYTES as u64,
                "bundle mesh lighting final recheck",
            )?;
            verify_file(record, &bytes, "bundle mesh lighting sidecar")?;
            true
        }
        None => read_optional_bounded(
            &lighting_path,
            crate::mesh_lighting::MAX_BYTES as u64,
            "bundle mesh lighting final recheck",
        )?
        .is_none(),
    };
    let hud_unchanged = match &manifest.hud {
        Some(record) => {
            let bytes = read_bounded(&hud_path, MAX_HUD_BYTES, "bundle mesh HUD final recheck")?;
            verify_file(record, &bytes, "bundle mesh HUD sidecar")?;
            true
        }
        None => read_optional_bounded(&hud_path, MAX_HUD_BYTES, "bundle mesh HUD final recheck")?
            .is_none(),
    };
    if checked_directory(root)? != canonical_root
        || read_bounded(
            &manifest_path,
            MAX_MANIFEST_BYTES,
            "bundle manifest recheck",
        )? != manifest_bytes
        || read_bounded(&scene_path, MAX_SCENE_BYTES, "bundle scene recheck")? != scene_bytes
        || read_bounded(&runtime_path, MAX_RUNTIME_BYTES, "bundle runtime recheck")?
            != runtime_bytes
        || inventory_names(&canonical_root, expected_names.len())? != expected_names
        || !sidecar_unchanged
        || !props_unchanged
        || !camera_unchanged
        || !animation_unchanged
        || !lighting_unchanged
        || !hud_unchanged
    {
        return Err("mesh bundle changed while it was being opened".into());
    }
    Ok(PreparedBundle {
        scene: scene_path,
        checksum,
        graph_revision,
        runtime_sha256: digest(&runtime_bytes),
        input_profile: manifest.input_profile,
    })
}

fn admit_scene_bytes(bytes: &[u8]) -> Result<(u64, [u8; 32]), String> {
    let text =
        std::str::from_utf8(bytes).map_err(|error| format!("mesh scene is not UTF-8: {error}"))?;
    let document = mesh_doc_from_yaml(text)
        .map_err(|error| format!("mesh scene admission failed: {error}"))?;
    let checksum = document.frame().checksum();
    let view = mesh_from_view(FrameView::of(document.frame()))?;
    Ok((checksum, view.graph.revision()))
}

fn bundle_file(name: &str, bytes: &[u8]) -> BundleFile {
    BundleFile {
        name: name.to_owned(),
        bytes: bytes.len() as u64,
        sha256: digest(bytes),
    }
}

fn validate_bundle_file(file: &BundleFile, expected_name: &str, limit: u64) -> Result<(), String> {
    checked_leaf(&file.name)?;
    if file.name != expected_name
        || file.bytes == 0
        || file.bytes > limit
        || !valid_digest(&file.sha256)
    {
        return Err("mesh bundle manifest contains invalid file metadata".into());
    }
    Ok(())
}

fn validate_scene_record(file: &BundleFile) -> Result<(), String> {
    checked_leaf(&file.name)?;
    if !file.name.ends_with(".scene.yaml")
        || file.name.len() == ".scene.yaml".len()
        || file.name.len() > MAX_SCENE_LEAF_BYTES
        || file.bytes == 0
        || file.bytes > MAX_SCENE_BYTES
        || !valid_digest(&file.sha256)
    {
        return Err("mesh bundle manifest contains invalid scene metadata".into());
    }
    Ok(())
}

fn verify_file(record: &BundleFile, bytes: &[u8], context: &str) -> Result<(), String> {
    if bytes.len() as u64 != record.bytes || digest(bytes) != record.sha256 {
        return Err(format!("{context} bytes do not match the bundle manifest"));
    }
    Ok(())
}

fn inventory_names(root: &Path, expected_count: usize) -> Result<BTreeSet<String>, String> {
    let mut names = BTreeSet::new();
    let mut folded = BTreeSet::new();
    for entry in fs::read_dir(root).map_err(|error| format!("read bundle inventory: {error}"))? {
        let entry = entry.map_err(|error| format!("read bundle entry: {error}"))?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
        if !regular_file(&metadata) {
            return Err("mesh bundle entries must be regular non-reparse files".into());
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "bundle filename must be Unicode")?;
        checked_leaf(&name)?;
        if !folded.insert(name.to_lowercase()) || !names.insert(name) {
            return Err("mesh bundle contains colliding filenames".into());
        }
        if names.len() > expected_count {
            return Err("mesh bundle contains too many files".into());
        }
    }
    Ok(names)
}

fn checked_scene_path(path: &Path) -> Result<PathBuf, String> {
    let canonical = checked_existing_file(path, MAX_SCENE_BYTES, "mesh scene")?;
    let filename = canonical
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("mesh scene filename must be Unicode")?;
    if !filename.ends_with(".scene.yaml")
        || filename.len() == ".scene.yaml".len()
        || filename.len() > MAX_SCENE_LEAF_BYTES
    {
        return Err("mesh scene filename must be a bounded *.scene.yaml name".into());
    }
    Ok(canonical)
}

fn checked_existing_file(path: &Path, limit: u64, context: &str) -> Result<PathBuf, String> {
    let text = path.to_str().ok_or("path must be Unicode")?;
    let components: Vec<_> = path.components().collect();
    if !path.is_absolute()
        || text.is_empty()
        || text.len() > MAX_PATH_BYTES
        || components.len() > MAX_PATH_COMPONENTS
        || text
            .chars()
            .any(|character| character.is_control() || character == '\0')
        || text
            .split(['\\', '/'])
            .any(|part| part == "." || part == "..")
    {
        return Err(format!(
            "{context} path must be bounded, absolute and normalized"
        ));
    }
    validate_local_prefix(&components)?;
    let filename = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("path must have a Unicode filename")?;
    checked_leaf(filename)?;
    let parent = path.parent().ok_or("path parent missing")?;
    check_parent_chain(parent)?;
    let canonical_parent = fs::canonicalize(parent)
        .map_err(|error| format!("canonicalize {context} parent: {error}"))?;
    let canonical = canonical_parent.join(filename);
    let metadata =
        fs::symlink_metadata(&canonical).map_err(|error| format!("inspect {context}: {error}"))?;
    if !regular_file(&metadata) || metadata.len() > limit {
        return Err(format!(
            "{context} must be a regular non-reparse file within its byte limit"
        ));
    }
    Ok(canonical)
}

fn checked_output_parent(path: &Path) -> Result<PathBuf, String> {
    let text = path.to_str().ok_or("bundle destination must be Unicode")?;
    let components: Vec<_> = path.components().collect();
    if !path.is_absolute()
        || text.is_empty()
        || text.len() > MAX_PATH_BYTES
        || components.len() > MAX_PATH_COMPONENTS
        || text
            .chars()
            .any(|character| character.is_control() || character == '\0')
        || text
            .split(['\\', '/'])
            .any(|part| part == "." || part == "..")
    {
        return Err("bundle destination must be bounded, absolute and normalized".into());
    }
    validate_local_prefix(&components)?;
    let leaf = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("bundle destination must have a Unicode directory name")?;
    checked_leaf(leaf)?;
    let parent = path.parent().ok_or("bundle destination parent missing")?;
    check_parent_chain(parent)?;
    fs::canonicalize(parent)
        .map_err(|error| format!("canonicalize bundle destination parent: {error}"))
}

fn checked_directory(path: &Path) -> Result<PathBuf, String> {
    let text = path.to_str().ok_or("bundle root must be Unicode")?;
    let components: Vec<_> = path.components().collect();
    if !path.is_absolute()
        || text.len() > MAX_PATH_BYTES
        || components.len() > MAX_PATH_COMPONENTS
        || text
            .chars()
            .any(|character| character.is_control() || character == '\0')
        || text
            .split(['\\', '/'])
            .any(|part| part == "." || part == "..")
    {
        return Err("bundle root must be a bounded absolute normalized path".into());
    }
    validate_local_prefix(&components)?;
    check_parent_chain(path)?;
    let canonical =
        fs::canonicalize(path).map_err(|error| format!("canonicalize bundle root: {error}"))?;
    let metadata = fs::symlink_metadata(&canonical).map_err(|error| error.to_string())?;
    if !regular_directory(&metadata) {
        return Err("bundle root must be a regular non-reparse directory".into());
    }
    Ok(canonical)
}

fn validate_local_prefix(components: &[Component<'_>]) -> Result<(), String> {
    #[cfg(windows)]
    if !matches!(components.first(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)))
    {
        return Err("path must use an absolute local disk prefix".into());
    }
    #[cfg(not(windows))]
    let _ = components;
    Ok(())
}

fn check_parent_chain(parent: &Path) -> Result<(), String> {
    let components: Vec<_> = parent.components().collect();
    let mut current = PathBuf::new();
    for component in components {
        match component {
            Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            Component::RootDir => current.push(component.as_os_str()),
            Component::Normal(part) => {
                let value = part.to_str().ok_or("path components must be Unicode")?;
                checked_leaf(value)?;
                current.push(part);
                let metadata = fs::symlink_metadata(&current)
                    .map_err(|error| format!("inspect path ancestry: {error}"))?;
                if !regular_directory(&metadata) {
                    return Err("path ancestry must contain only non-reparse directories".into());
                }
            }
            Component::CurDir | Component::ParentDir => {
                return Err("path ancestry is not normalized".into())
            }
        }
    }
    Ok(())
}

fn checked_leaf(value: &str) -> Result<&str, String> {
    if value.is_empty()
        || value.len() > 255
        || value.ends_with(['.', ' '])
        || value.contains([':', '/', '\\', '\0', '<', '>', '|', '?', '*'])
        || value.chars().any(char::is_control)
        || reserved_device_name(value)
    {
        return Err("path contains an invalid Windows filename component".into());
    }
    Ok(value)
}

fn reserved_device_name(value: &str) -> bool {
    let stem = value
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(['.', ' '])
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        })
}

fn read_optional_bounded(
    path: &Path,
    limit: u64,
    context: &str,
) -> Result<Option<Vec<u8>>, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => read_bounded(path, limit, context).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("inspect {context}: {error}")),
    }
}

fn read_bounded(path: &Path, limit: u64, context: &str) -> Result<Vec<u8>, String> {
    let canonical = checked_existing_file(path, limit, context)?;
    let before = fs::symlink_metadata(&canonical).map_err(|error| error.to_string())?;
    let mut file =
        open_without_following(&canonical).map_err(|error| format!("open {context}: {error}"))?;
    let opened = file.metadata().map_err(|error| error.to_string())?;
    if !regular_file(&opened) || opened.len() > limit || !same_observation(&before, &opened) {
        return Err(format!("{context} changed before reading"));
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    Read::by_ref(&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read {context}: {error}"))?;
    let after = file.metadata().map_err(|error| error.to_string())?;
    let current = fs::symlink_metadata(&canonical).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > limit
        || bytes.len() as u64 != opened.len()
        || !regular_file(&after)
        || !regular_file(&current)
        || !same_observation(&opened, &after)
        || !same_observation(&before, &current)
    {
        return Err(format!("{context} changed while being read"));
    }
    let parent = canonical.parent().ok_or("file parent missing")?;
    check_parent_chain(parent)?;
    if fs::canonicalize(parent).map_err(|error| error.to_string())? != parent {
        return Err(format!("{context} parent changed while being read"));
    }
    Ok(bytes)
}

fn open_without_following(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options.open(path)
}

fn regular_file(metadata: &Metadata) -> bool {
    if !metadata.is_file() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
    }
    #[cfg(not(windows))]
    {
        !metadata.file_type().is_symlink()
    }
}

fn regular_directory(metadata: &Metadata) -> bool {
    if !metadata.is_dir() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
    }
    #[cfg(not(windows))]
    {
        !metadata.file_type().is_symlink()
    }
}

fn same_observation(first: &Metadata, second: &Metadata) -> bool {
    first.len() == second.len()
        && match (first.modified(), second.modified()) {
            (Ok(first_time), Ok(second_time)) => first_time == second_time,
            _ => true,
        }
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("create bundle file: {error}"))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("write bundle file: {error}"))
}

fn refuse_existing(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err("mesh bundle destination already exists".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("inspect mesh bundle destination: {error}")),
    }
}

fn publish_new_directory(stage: &Path, destination: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
        }
        let stage_metadata = fs::symlink_metadata(stage).map_err(|error| error.to_string())?;
        if !regular_directory(&stage_metadata) {
            return Err("bundle staging directory changed before publish".into());
        }
        refuse_existing(destination)?;
        let stage_wide: Vec<u16> = stage.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination_wide: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let result = unsafe { MoveFileExW(stage_wide.as_ptr(), destination_wide.as_ptr(), 0) };
        if result == 0 {
            return Err(format!(
                "native no-replace bundle publication failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        unsafe extern "C" {
            fn renameat2(
                old_dirfd: i32,
                old_path: *const i8,
                new_dirfd: i32,
                new_path: *const i8,
                flags: u32,
            ) -> i32;
        }
        let old_path =
            CString::new(stage.as_os_str().as_bytes()).map_err(|_| "staging path contains NUL")?;
        let new_path = CString::new(destination.as_os_str().as_bytes())
            .map_err(|_| "destination path contains NUL")?;
        let result = unsafe { renameat2(-100, old_path.as_ptr(), -100, new_path.as_ptr(), 1) };
        if result != 0 {
            return Err(format!(
                "native no-replace bundle publication failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (stage, destination);
        Err("native no-replace bundle publication is unavailable on this platform".into())
    }
}

#[cfg(test)]
fn validate_runtime(bytes: &[u8], require_props: bool) -> Result<(), String> {
    validate_runtime_for(bytes, require_props, false)
}

fn serialized_rotation_present(bytes: &[u8], props: bool) -> Result<bool, String> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("inspect serialized mesh rotation: {error}"))?;
    if !props {
        return Ok(value.get("rotation").is_some());
    }
    let records = value
        .get("props")
        .and_then(serde_json::Value::as_array)
        .ok_or("mesh props sidecar has no props inventory")?;
    Ok(records.iter().any(|record| {
        record
            .get("art")
            .is_some_and(|art| art.get("rotation").is_some())
    }))
}

fn serialized_material_present(bytes: &[u8], props: bool) -> Result<bool, String> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("inspect serialized mesh material: {error}"))?;
    if !props {
        return Ok(value.get("tint").is_some());
    }
    let records = value
        .get("props")
        .and_then(serde_json::Value::as_array)
        .ok_or("mesh props sidecar has no props inventory")?;
    Ok(records.iter().any(|record| {
        record
            .get("art")
            .is_some_and(|art| art.get("tint").is_some())
    }))
}

fn serialized_material_slots_present(bytes: &[u8], props: bool) -> Result<bool, String> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("inspect serialized mesh material slots: {error}"))?;
    if !props {
        return Ok(value.get("material_slots").is_some());
    }
    let records = value
        .get("props")
        .and_then(serde_json::Value::as_array)
        .ok_or("mesh props sidecar has no props inventory")?;
    Ok(records.iter().any(|record| {
        record
            .get("art")
            .is_some_and(|art| art.get("material_slots").is_some())
    }))
}

fn serialized_surface_present(bytes: &[u8], props: bool) -> Result<bool, String> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("inspect serialized mesh surface: {error}"))?;
    if !props {
        return Ok(value.get("surface").is_some());
    }
    let records = value
        .get("props")
        .and_then(serde_json::Value::as_array)
        .ok_or("mesh props sidecar has no props inventory")?;
    Ok(records.iter().any(|record| {
        record
            .get("art")
            .is_some_and(|art| art.get("surface").is_some())
    }))
}

#[cfg(test)]
fn validate_runtime_for(
    bytes: &[u8],
    require_props: bool,
    require_rotation: bool,
) -> Result<(), String> {
    validate_runtime_with_camera(bytes, require_props, require_rotation, false)
}

#[cfg(test)]
fn validate_runtime_with_camera(
    bytes: &[u8],
    require_props: bool,
    require_rotation: bool,
    require_camera: bool,
) -> Result<(), String> {
    validate_runtime_with_material(
        bytes,
        require_props,
        require_rotation,
        require_camera,
        false,
    )
}

#[cfg(test)]
fn validate_runtime_with_material(
    bytes: &[u8],
    require_props: bool,
    require_rotation: bool,
    require_camera: bool,
    require_material: bool,
) -> Result<(), String> {
    validate_runtime_with_animation(
        bytes,
        require_props,
        require_rotation,
        require_camera,
        require_material,
        false,
        false,
    )
}

#[cfg(test)]
fn validate_runtime_with_material_slots(bytes: &[u8], required: bool) -> Result<(), String> {
    validate_runtime_with_lighting(
        bytes,
        RuntimeRequirements {
            require_props: false,
            require_rotation: false,
            require_camera: false,
            require_material: false,
            require_material_slots: required,
            require_animation: false,
            require_surface: false,
            require_lighting: false,
            require_character: false,
            require_hud: false,
        },
    )
}

#[cfg(test)]
fn validate_runtime_with_surface(bytes: &[u8], require_props: bool) -> Result<(), String> {
    validate_runtime_with_animation(bytes, require_props, false, false, false, false, true)
}

#[cfg(test)]
fn validate_runtime_with_animation(
    bytes: &[u8],
    require_props: bool,
    require_rotation: bool,
    require_camera: bool,
    require_material: bool,
    require_animation: bool,
    require_surface: bool,
) -> Result<(), String> {
    validate_runtime_with_lighting(
        bytes,
        RuntimeRequirements {
            require_props,
            require_rotation,
            require_camera,
            require_material,
            require_material_slots: false,
            require_animation,
            require_surface,
            require_lighting: false,
            require_character: false,
            require_hud: false,
        },
    )
}

struct RuntimeRequirements {
    require_props: bool,
    require_rotation: bool,
    require_camera: bool,
    require_material: bool,
    require_material_slots: bool,
    require_animation: bool,
    require_surface: bool,
    require_lighting: bool,
    require_character: bool,
    require_hud: bool,
}

fn validate_runtime_with_lighting(
    bytes: &[u8],
    requirements: RuntimeRequirements,
) -> Result<(), String> {
    let RuntimeRequirements {
        require_props,
        require_rotation,
        require_camera,
        require_material,
        require_material_slots,
        require_animation,
        require_surface,
        require_lighting,
        require_character,
        require_hud,
    } = requirements;
    if bytes.len() < 0x40 || &bytes[..2] != b"MZ" {
        return Err("mesh player runtime is missing its DOS executable header".into());
    }
    let pe_offset = u32::from_le_bytes(
        bytes[0x3c..0x40]
            .try_into()
            .map_err(|_| "invalid DOS header")?,
    ) as usize;
    let pe_end = pe_offset
        .checked_add(24)
        .ok_or("invalid PE header offset")?;
    if pe_end > bytes.len() || &bytes[pe_offset..pe_offset + 4] != b"PE\0\0" {
        return Err("mesh player runtime has an invalid PE signature".into());
    }
    let machine = u16::from_le_bytes(
        bytes[pe_offset + 4..pe_offset + 6]
            .try_into()
            .map_err(|_| "invalid PE machine")?,
    );
    let optional_size = u16::from_le_bytes(
        bytes[pe_offset + 20..pe_offset + 22]
            .try_into()
            .map_err(|_| "invalid PE optional header size")?,
    ) as usize;
    let section_count = u16::from_le_bytes(
        bytes[pe_offset + 6..pe_offset + 8]
            .try_into()
            .map_err(|_| "invalid PE section count")?,
    ) as usize;
    let characteristics = u16::from_le_bytes(
        bytes[pe_offset + 22..pe_offset + 24]
            .try_into()
            .map_err(|_| "invalid PE characteristics")?,
    );
    let optional_start = pe_offset + 24;
    if machine != 0x8664
        || optional_size < 2
        || optional_start
            .checked_add(optional_size)
            .is_none_or(|end| end > bytes.len())
        || u16::from_le_bytes(
            bytes[optional_start..optional_start + 2]
                .try_into()
                .map_err(|_| "invalid PE optional header")?,
        ) != 0x20b
    {
        return Err("mesh player runtime must be an x64 PE32+ executable".into());
    }
    if section_count == 0
        || section_count > 96
        || characteristics & 0x0002 == 0
        || characteristics & 0x2000 != 0
    {
        return Err(
            "mesh player runtime must be a non-DLL x64 executable with bounded sections".into(),
        );
    }
    let section_table = optional_start
        .checked_add(optional_size)
        .ok_or("invalid PE section table offset")?;
    let section_table_bytes = section_count
        .checked_mul(40)
        .ok_or("invalid PE section count")?;
    let section_table_end = section_table
        .checked_add(section_table_bytes)
        .ok_or("invalid PE section table size")?;
    if section_table_end > bytes.len() {
        return Err("mesh player runtime has a truncated PE section table".into());
    }
    let runtime_marker_section = validate_runtime_section(bytes, RUNTIME_SECTION, RUNTIME_MARKER)?;
    if runtime_marker_section.is_none() {
        return Err("mesh player runtime has no .orrmesh marker section".into());
    }
    if require_props
        && validate_runtime_section(bytes, PROPS_RUNTIME_SECTION, PROPS_RUNTIME_MARKER)?.is_none()
    {
        return Err("mesh player runtime does not advertise mesh-props bundle support".into());
    }
    if require_rotation
        && validate_runtime_section(bytes, ROTATION_RUNTIME_SECTION, ROTATION_RUNTIME_MARKER)?
            .is_none()
    {
        return Err("mesh player runtime does not advertise mesh-rotation support".into());
    }
    if require_camera
        && validate_runtime_section(bytes, CAMERA_RUNTIME_SECTION, CAMERA_RUNTIME_MARKER)?.is_none()
    {
        return Err("mesh player runtime does not advertise mesh-camera bundle support".into());
    }
    if require_material
        && validate_runtime_section(bytes, MATERIAL_RUNTIME_SECTION, MATERIAL_RUNTIME_MARKER)?
            .is_none()
    {
        return Err("mesh player runtime does not advertise mesh-material bundle support".into());
    }
    if require_material_slots
        && validate_runtime_section(
            bytes,
            MATERIAL_SLOTS_RUNTIME_SECTION,
            MATERIAL_SLOTS_RUNTIME_MARKER,
        )?
        .is_none()
    {
        return Err("mesh player runtime does not advertise material-slot support".into());
    }
    if require_animation
        && validate_runtime_section(bytes, ANIMATION_RUNTIME_SECTION, ANIMATION_RUNTIME_MARKER)?
            .is_none()
    {
        return Err("mesh player runtime does not advertise mesh-animation bundle support".into());
    }
    if require_surface
        && validate_runtime_section(bytes, SURFACE_RUNTIME_SECTION, SURFACE_RUNTIME_MARKER)?
            .is_none()
    {
        return Err("mesh player runtime does not advertise mesh-surface bundle support".into());
    }
    if require_lighting
        && validate_runtime_section(bytes, LIGHTING_RUNTIME_SECTION, LIGHTING_RUNTIME_MARKER)?
            .is_none()
    {
        return Err("mesh player runtime does not advertise mesh-lighting bundle support".into());
    }
    if require_character
        && validate_runtime_section(bytes, CHARACTER_RUNTIME_SECTION, CHARACTER_RUNTIME_MARKER)?
            .is_none()
    {
        return Err("mesh player runtime does not advertise character-follow support".into());
    }
    if require_hud
        && validate_runtime_section(bytes, HUD_RUNTIME_SECTION, HUD_RUNTIME_MARKER)?.is_none()
    {
        return Err("mesh player runtime does not advertise Mesh HUD support".into());
    }
    Ok(())
}

fn optional_facing_present(bytes: Option<&[u8]>, props: bool) -> Result<bool, String> {
    bytes
        .map(|bytes| serialized_facing_present(bytes, props))
        .transpose()
        .map(|present| present.unwrap_or(false))
}

fn serialized_facing_present(bytes: &[u8], props: bool) -> Result<bool, String> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("inspect serialized mesh facing: {error}"))?;
    if props {
        let records = value
            .get("props")
            .and_then(serde_json::Value::as_array)
            .ok_or("mesh props sidecar has no props inventory")?;
        return Ok(records.iter().any(|record| {
            record
                .get("art")
                .is_some_and(|art| art.get("facing").is_some())
        }));
    }
    Ok(value.get("facing").is_some()
        || value
            .get("settings")
            .is_some_and(|settings| settings.get("facing").is_some()))
}

fn serialized_camera_follow_present(bytes: &[u8]) -> Result<bool, String> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("inspect serialized mesh camera follow: {error}"))?;
    Ok(value
        .get("camera")
        .is_some_and(|camera| camera.get("follow_offset").is_some()))
}

#[cfg(feature = "mesh-animated-models")]
fn serialized_animation_idle_present(bytes: &[u8]) -> Result<bool, String> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("inspect serialized mesh animation idle clip: {error}"))?;
    Ok(value
        .get("settings")
        .is_some_and(|settings| settings.get("idle_clip").is_some()))
}

#[cfg(test)]
fn validate_runtime_with_character(bytes: &[u8], required: bool) -> Result<(), String> {
    validate_runtime_with_lighting(
        bytes,
        RuntimeRequirements {
            require_props: false,
            require_rotation: false,
            require_camera: false,
            require_material: false,
            require_material_slots: false,
            require_animation: false,
            require_surface: false,
            require_lighting: false,
            require_character: required,
            require_hud: false,
        },
    )
}

fn validate_runtime_facing(bytes: &[u8], required: bool) -> Result<(), String> {
    if required
        && validate_runtime_section(bytes, FACING_RUNTIME_SECTION, FACING_RUNTIME_MARKER)?.is_none()
    {
        return Err("mesh player runtime does not advertise route-facing support".into());
    }
    Ok(())
}

fn validate_runtime_dense_props(bytes: &[u8], required: bool) -> Result<(), String> {
    if required
        && validate_runtime_section(
            bytes,
            DENSE_PROPS_RUNTIME_SECTION,
            DENSE_PROPS_RUNTIME_MARKER,
        )?
        .is_none()
    {
        return Err("mesh player runtime does not advertise dense mesh-props support".into());
    }
    Ok(())
}

fn validate_runtime_profile(
    bytes: &[u8],
    input_profile: Option<InputProfile>,
) -> Result<(), String> {
    if input_profile == Some(InputProfile::MeshClickToMoveV1)
        && validate_runtime_section(bytes, INPUT_RUNTIME_SECTION, INPUT_RUNTIME_MARKER)?.is_none()
    {
        return Err(
            "mesh player runtime does not advertise mesh click-to-move input support".into(),
        );
    }
    Ok(())
}

fn compiled_game_profile() -> Option<GameProfile> {
    #[cfg(feature = "mesh-mission")]
    {
        Some(GameProfile::MeshMissionV1)
    }
    #[cfg(not(feature = "mesh-mission"))]
    {
        None
    }
}

fn validate_game_profile(profile: Option<GameProfile>) -> Result<(), String> {
    if profile != compiled_game_profile() {
        #[cfg(feature = "mesh-mission")]
        return Err("mesh bundle is missing the mesh-mission-v1 game profile".into());
        #[cfg(not(feature = "mesh-mission"))]
        return Err("mesh-mission bundle requires rebuilding with mesh-mission support".into());
    }
    Ok(())
}

fn validate_runtime_game_profile(bytes: &[u8], profile: Option<GameProfile>) -> Result<(), String> {
    validate_game_profile(profile)?;
    if profile == Some(GameProfile::MeshMissionV1)
        && validate_runtime_section(bytes, MISSION_RUNTIME_SECTION, MISSION_RUNTIME_MARKER)?
            .is_none()
    {
        return Err("mesh player runtime does not advertise MeshMission support".into());
    }
    Ok(())
}

fn animation_sidecar_path(scene: &Path) -> Result<PathBuf, String> {
    let filename = scene
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("mesh scene filename must be Unicode")?;
    Ok(scene.with_file_name(format!("{filename}{ANIMATION_SUFFIX}")))
}

fn validate_runtime_section(
    bytes: &[u8],
    section_name: &[u8; 8],
    marker: &[u8],
) -> Result<Option<usize>, String> {
    let pe_offset = u32::from_le_bytes(
        bytes[0x3c..0x40]
            .try_into()
            .map_err(|_| "invalid DOS header")?,
    ) as usize;
    let section_count = u16::from_le_bytes(
        bytes[pe_offset + 6..pe_offset + 8]
            .try_into()
            .map_err(|_| "invalid PE section count")?,
    ) as usize;
    let optional_size = u16::from_le_bytes(
        bytes[pe_offset + 20..pe_offset + 22]
            .try_into()
            .map_err(|_| "invalid PE optional header size")?,
    ) as usize;
    let section_table = pe_offset
        .checked_add(24)
        .and_then(|offset| offset.checked_add(optional_size))
        .ok_or("invalid PE section table offset")?;
    let section_table_end = section_table
        .checked_add(
            section_count
                .checked_mul(40)
                .ok_or("invalid PE section count")?,
        )
        .ok_or("invalid PE section table size")?;
    if section_table_end > bytes.len() {
        return Err("mesh player runtime has a truncated PE section table".into());
    }
    let mut found = None;
    for index in 0..section_count {
        let section_start = section_table + index * 40;
        let section = &bytes[section_start..section_start + 40];
        if &section[..8] != section_name {
            continue;
        }
        if found.is_some() {
            return Err(format!(
                "mesh player runtime has duplicate {} sections",
                String::from_utf8_lossy(section_name)
            ));
        }
        let raw_size = u32::from_le_bytes(
            section[16..20]
                .try_into()
                .map_err(|_| "invalid PE section raw size")?,
        ) as usize;
        let raw_offset = u32::from_le_bytes(
            section[20..24]
                .try_into()
                .map_err(|_| "invalid PE section raw offset")?,
        ) as usize;
        let raw_end = raw_offset
            .checked_add(raw_size)
            .ok_or("invalid PE section bounds")?;
        if raw_size < marker.len()
            || raw_offset < section_table_end
            || raw_end > bytes.len()
            || &bytes[raw_offset..raw_offset + marker.len()] != marker
        {
            return Err(format!(
                "mesh player runtime marker is not at the start of {} section data",
                String::from_utf8_lossy(section_name)
            ));
        }
        found = Some(index);
    }
    Ok(found)
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
