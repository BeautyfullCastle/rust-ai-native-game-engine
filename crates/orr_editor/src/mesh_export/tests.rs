use super::{
    export, export_interactive, open_bundle, validate_runtime, validate_runtime_profile,
    InputProfile, MANIFEST_FILENAME, PROPS_SUFFIX, RUNTIME_FILENAME,
};
use crate::{
    mesh_art::Snapshot,
    mesh_camera::{Document as CameraDocument, Projection as CameraProjection},
    mesh_props::{self, Document, Prop},
};
use orr_fp::{fp, FP};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use orr_remote::mesh_navigation::{authoring::create_mesh_scene, AgentSpec};
use orr_render::camera3d::OrbitCamera;
use std::fs;

const STATIC_GLB: &[u8] = include_bytes!("../../../orr_model/tests/fixtures/model.glb");
const LICENSE: &[u8] = b"Example license text\n";

fn admitted_scene() -> (String, [u8; 32], u64) {
    let graph = StaticMeshGraph::bake(
        &[
            [FP::ZERO, FP::ZERO, FP::ZERO],
            [fp!(2), FP::ZERO, FP::ZERO],
            [FP::ZERO, FP::ZERO, fp!(2)],
            [fp!(2), FP::ZERO, fp!(2)],
        ],
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile::default(),
    )
    .unwrap();
    let spec = AgentSpec {
        start: [fp!(0.25), FP::ZERO, fp!(0.25)],
        goal: [fp!(1.75), FP::ZERO, fp!(1.75)],
        max_vertical: FP::ZERO,
        distance_per_tick: fp!(0.25),
    };
    let mut document = create_mesh_scene(&graph, spec).unwrap();
    let checksum = document.frame().checksum();
    (document.save_yaml(), graph.revision(), checksum)
}

fn test_runtime() -> Vec<u8> {
    let mut bytes = vec![0; 512];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    bytes[0x40..0x44].copy_from_slice(b"PE\0\0");
    bytes[0x44..0x46].copy_from_slice(&0x8664u16.to_le_bytes());
    bytes[0x46..0x48].copy_from_slice(&1u16.to_le_bytes());
    bytes[0x54..0x56].copy_from_slice(&240u16.to_le_bytes());
    bytes[0x56..0x58].copy_from_slice(&0x0002u16.to_le_bytes());
    bytes[0x58..0x5a].copy_from_slice(&0x20bu16.to_le_bytes());
    bytes[0x148..0x150].copy_from_slice(b".orrmesh");
    bytes[0x158..0x15c].copy_from_slice(&64u32.to_le_bytes());
    bytes[0x15c..0x160].copy_from_slice(&0x180u32.to_le_bytes());
    bytes[0x16c..0x170].copy_from_slice(&0x4000_0040u32.to_le_bytes());
    bytes[0x180..0x180 + b"ORR_MESH_PLAYER_V1".len()].copy_from_slice(b"ORR_MESH_PLAYER_V1");
    bytes
}

fn test_runtime_with_input() -> Vec<u8> {
    let mut bytes = test_runtime();
    bytes.resize(0x240, 0);
    bytes[0x180..].fill(0);
    bytes[0x15c..0x160].copy_from_slice(&0x1c0u32.to_le_bytes());
    bytes[0x46..0x48].copy_from_slice(&2u16.to_le_bytes());
    bytes[0x170..0x178].copy_from_slice(b".orrctrl");
    bytes[0x180..0x184].copy_from_slice(&64u32.to_le_bytes());
    bytes[0x184..0x188].copy_from_slice(&0x200u32.to_le_bytes());
    bytes[0x194..0x198].copy_from_slice(&0x4000_0040u32.to_le_bytes());
    bytes[0x1c0..0x1c0 + b"ORR_MESH_PLAYER_V1".len()].copy_from_slice(b"ORR_MESH_PLAYER_V1");
    bytes[0x200..0x200 + b"ORR_MESH_INPUT_V1".len()].copy_from_slice(b"ORR_MESH_INPUT_V1");
    bytes
}

fn test_runtime_with_lighting() -> Vec<u8> {
    let mut bytes = test_runtime();
    bytes.resize(0x240, 0);
    bytes[0x180..].fill(0);
    bytes[0x15c..0x160].copy_from_slice(&0x1c0u32.to_le_bytes());
    bytes[0x46..0x48].copy_from_slice(&2u16.to_le_bytes());
    bytes[0x170..0x178].copy_from_slice(b".orrlit\0");
    bytes[0x180..0x184].copy_from_slice(&64u32.to_le_bytes());
    bytes[0x184..0x188].copy_from_slice(&0x200u32.to_le_bytes());
    bytes[0x194..0x198].copy_from_slice(&0x4000_0040u32.to_le_bytes());
    bytes[0x1c0..0x1c0 + b"ORR_MESH_PLAYER_V1".len()].copy_from_slice(b"ORR_MESH_PLAYER_V1");
    bytes[0x200..0x200 + b"ORR_MESH_LIGHTING_V1".len()].copy_from_slice(b"ORR_MESH_LIGHTING_V1");
    bytes
}

#[cfg(feature = "mesh-animated-models")]
fn test_runtime_with_all_presentations() -> Vec<u8> {
    let sections: [(&[u8; 8], &[u8]); 8] = [
        (b".orrmesh", b"ORR_MESH_PLAYER_V1"),
        (b".orrprop", b"ORR_MESH_PROPS_V1"),
        (b".orrrot\0", b"ORR_MESH_ROTATION_V1"),
        (b".orrcam\0", b"ORR_MESH_CAMERA_V1"),
        (b".orrmat\0", b"ORR_MESH_MATERIAL_V1"),
        (b".orranim", b"ORR_MESH_ANIMATION_V1"),
        (b".orrsurf", b"ORR_MESH_SURFACE_V1"),
        (b".orrlit\0", b"ORR_MESH_LIGHTING_V1"),
    ];
    let mut bytes = test_runtime();
    bytes.resize(0x600, 0);
    bytes[0x148..].fill(0);
    bytes[0x46..0x48].copy_from_slice(&(sections.len() as u16).to_le_bytes());
    for (index, (name, marker)) in sections.iter().enumerate() {
        let header = 0x148 + index * 40;
        let payload = 0x400 + index * 64;
        bytes[header..header + 8].copy_from_slice(*name);
        bytes[header + 16..header + 20].copy_from_slice(&64u32.to_le_bytes());
        bytes[header + 20..header + 24].copy_from_slice(&(payload as u32).to_le_bytes());
        bytes[header + 36..header + 40].copy_from_slice(&0x4000_0040u32.to_le_bytes());
        bytes[payload..payload + marker.len()].copy_from_slice(marker);
    }
    bytes
}

#[cfg(feature = "mesh-animated-models")]
#[test]
fn complete_presentation_bundles_admit_seven_and_eight_files_but_no_extra_inventory() {
    for include_lighting in [false, true] {
        let (temporary, scene, runtime, checksum) = fixture();
        let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
        add_props(&scene, revision);
        let props = crate::mesh_props::load(&scene, revision)
            .unwrap()
            .unwrap()
            .0;
        crate::mesh_art::save(&scene, Some(&props.props[0].art), None).unwrap();
        add_camera(&scene, revision);
        let package = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/animation_demo")
            .canonicalize()
            .unwrap();
        let animation = crate::mesh_animation::Snapshot::from_package(
            &package,
            "animated.glb",
            "LICENSE.txt",
            revision,
        )
        .unwrap();
        crate::mesh_animation_store::save(&scene, Some(&animation), None, revision).unwrap();
        if include_lighting {
            add_lighting(&scene, revision);
        }
        fs::write(&runtime, test_runtime_with_all_presentations()).unwrap();
        let destination = temporary.path().join("complete-presentation-bundle");
        export(&scene, checksum, &destination, &runtime).unwrap();
        assert_eq!(
            fs::read_dir(&destination).unwrap().count(),
            if include_lighting { 8 } else { 7 }
        );
        let reopened = open_bundle(&destination).unwrap();
        assert_eq!(reopened.checksum, checksum);
        assert_eq!(reopened.graph_revision, revision);
        fs::write(destination.join("unlisted.bin"), b"unexpected").unwrap();
        assert!(open_bundle(&destination).is_err());
    }
}

fn add_lighting(scene: &std::path::Path, revision: [u8; 32]) -> Vec<u8> {
    let document =
        crate::mesh_lighting::Document::from_lighting(orr_render::Lighting::default()).unwrap();
    crate::mesh_lighting_store::save(scene, Some(&document), None, revision)
        .unwrap()
        .unwrap();
    fs::read(crate::mesh_lighting_store::sidecar_path(scene).unwrap()).unwrap()
}

#[test]
fn lighting_bundle_requires_an_explicit_runtime_marker_and_preserves_source_free_bytes() {
    let (temporary, scene, runtime, checksum) = fixture();
    let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
    let lighting_bytes = add_lighting(&scene, revision);
    let source_lighting = crate::mesh_lighting_store::sidecar_path(&scene).unwrap();
    let destination = temporary.path().join("lighting-bundle");

    let error = export(&scene, checksum, &destination, &runtime).unwrap_err();
    assert!(error.contains("mesh-lighting"));
    assert!(!destination.exists());
    fs::write(&runtime, test_runtime_with_lighting()).unwrap();
    let receipt = export(&scene, checksum, &destination, &runtime).unwrap();

    assert_eq!(
        receipt.lighting_sha256,
        Some(super::digest(&lighting_bytes))
    );
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 4);
    let copied_lighting = destination.join("walk.scene.yaml.mesh-lighting.json");
    assert_eq!(fs::read(&copied_lighting).unwrap(), lighting_bytes);
    let admitted = open_bundle(&destination).unwrap();
    assert_eq!(admitted.checksum, checksum);
    assert_eq!(admitted.graph_revision, revision);
    fs::remove_file(source_lighting).unwrap();
    fs::remove_file(&scene).unwrap();
    let reopened = open_bundle(&destination).unwrap();
    assert_eq!(reopened.checksum, checksum);
    assert_eq!(fs::read(copied_lighting).unwrap(), lighting_bytes);
    assert!(crate::mesh_lighting_store::load(&reopened.scene, revision)
        .unwrap()
        .is_some());
}

#[test]
fn malformed_or_stale_lighting_never_creates_an_export_directory() {
    for variant in [
        "invalid-json",
        "wrong-scene",
        "wrong-graph",
        "unknown-field",
    ] {
        let (temporary, scene, runtime, checksum) = fixture();
        let scene_bytes = fs::read(&scene).unwrap();
        let revision = graph_revision(std::str::from_utf8(&scene_bytes).unwrap());
        let document =
            crate::mesh_lighting::Document::from_lighting(orr_render::Lighting::default()).unwrap();
        let mut bytes = match variant {
            "invalid-json" => b"invalid lighting JSON".to_vec(),
            "wrong-scene" => document.encode("different.scene.yaml", revision).unwrap(),
            "wrong-graph" => document.encode("walk.scene.yaml", [255; 32]).unwrap(),
            _ => document.encode("walk.scene.yaml", revision).unwrap(),
        };
        if variant == "unknown-field" {
            let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            value["unexpected"] = true.into();
            bytes = serde_json::to_vec(&value).unwrap();
        }
        fs::write(
            crate::mesh_lighting_store::sidecar_path(&scene).unwrap(),
            &bytes,
        )
        .unwrap();
        fs::write(&runtime, test_runtime_with_lighting()).unwrap();
        let destination = temporary.path().join("rejected-lighting-bundle");

        assert!(
            export(&scene, checksum, &destination, &runtime).is_err(),
            "{variant}"
        );
        assert!(!destination.exists(), "{variant}");
        assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    }
}

#[test]
fn lighting_bundle_refuses_missing_extra_tampered_and_graph_rebound_payloads() {
    for variant in ["missing", "extra", "tampered", "rebound", "old-runtime"] {
        let (temporary, scene, runtime, checksum) = fixture();
        let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
        let lighting_bytes = add_lighting(&scene, revision);
        fs::write(&runtime, test_runtime_with_lighting()).unwrap();
        let destination = temporary.path().join("validated-lighting-bundle");
        export(&scene, checksum, &destination, &runtime).unwrap();
        let lighting_path = destination.join("walk.scene.yaml.mesh-lighting.json");
        let manifest_path = destination.join(MANIFEST_FILENAME);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        match variant {
            "missing" => fs::remove_file(&lighting_path).unwrap(),
            "extra" => {
                manifest.as_object_mut().unwrap().remove("lighting");
                fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
            }
            "tampered" => {
                let mut tampered = lighting_bytes;
                tampered.push(b' ');
                fs::write(&lighting_path, tampered).unwrap();
            }
            "rebound" => {
                let mut lighting: serde_json::Value =
                    serde_json::from_slice(&lighting_bytes).unwrap();
                lighting["graph_revision"] = serde_json::json!(vec![255u8; 32]);
                let rebound = serde_json::to_vec(&lighting).unwrap();
                manifest["lighting"]["bytes"] = serde_json::json!(rebound.len());
                manifest["lighting"]["sha256"] = super::digest(&rebound).into();
                fs::write(&lighting_path, rebound).unwrap();
                fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
            }
            "old-runtime" => {
                let old_runtime = test_runtime();
                manifest["runtime"]["bytes"] = serde_json::json!(old_runtime.len());
                manifest["runtime"]["sha256"] = super::digest(&old_runtime).into();
                fs::write(destination.join(RUNTIME_FILENAME), old_runtime).unwrap();
                fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
            }
            _ => unreachable!(),
        }

        assert!(open_bundle(&destination).is_err(), "{variant}");
    }
}

#[test]
fn lighting_capability_cannot_be_advertised_by_an_unsectioned_or_shifted_marker() {
    for shifted in [false, true] {
        let (temporary, scene, runtime, checksum) = fixture();
        let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
        add_lighting(&scene, revision);
        let bytes = if shifted {
            let mut bytes = test_runtime_with_lighting();
            bytes[0x200..0x240].fill(0);
            bytes[0x201..0x201 + b"ORR_MESH_LIGHTING_V1".len()]
                .copy_from_slice(b"ORR_MESH_LIGHTING_V1");
            bytes
        } else {
            let mut bytes = test_runtime();
            bytes.extend_from_slice(b"ORR_MESH_LIGHTING_V1");
            bytes
        };
        fs::write(&runtime, bytes).unwrap();
        let destination = temporary.path().join("false-capability-bundle");

        assert!(export(&scene, checksum, &destination, &runtime).is_err());
        assert!(!destination.exists());
    }
}

fn test_runtime_with_props() -> Vec<u8> {
    let mut bytes = vec![0; 640];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    bytes[0x40..0x44].copy_from_slice(b"PE\0\0");
    bytes[0x44..0x46].copy_from_slice(&0x8664u16.to_le_bytes());
    bytes[0x46..0x48].copy_from_slice(&2u16.to_le_bytes());
    bytes[0x54..0x56].copy_from_slice(&240u16.to_le_bytes());
    bytes[0x56..0x58].copy_from_slice(&0x0002u16.to_le_bytes());
    bytes[0x58..0x5a].copy_from_slice(&0x20bu16.to_le_bytes());
    bytes[0x148..0x150].copy_from_slice(b".orrmesh");
    bytes[0x158..0x15c].copy_from_slice(&64u32.to_le_bytes());
    bytes[0x15c..0x160].copy_from_slice(&0x1c0u32.to_le_bytes());
    bytes[0x16c..0x170].copy_from_slice(&0x4000_0040u32.to_le_bytes());
    bytes[0x170..0x178].copy_from_slice(b".orrprop");
    bytes[0x180..0x184].copy_from_slice(&64u32.to_le_bytes());
    bytes[0x184..0x188].copy_from_slice(&0x200u32.to_le_bytes());
    bytes[0x198..0x19c].copy_from_slice(&0x4000_0040u32.to_le_bytes());
    bytes[0x1c0..0x1c0 + b"ORR_MESH_PLAYER_V1".len()].copy_from_slice(b"ORR_MESH_PLAYER_V1");
    bytes[0x200..0x200 + b"ORR_MESH_PROPS_V1".len()].copy_from_slice(b"ORR_MESH_PROPS_V1");
    bytes
}

fn graph_revision(yaml: &str) -> [u8; 32] {
    let document = orr_remote::mesh_navigation::mesh_doc_from_yaml(yaml).unwrap();
    orr_remote::mesh_navigation::mesh_from_view(orr_bridge::FrameView::of(document.frame()))
        .unwrap()
        .graph
        .revision()
}

fn test_runtime_with_rotation() -> Vec<u8> {
    let mut bytes = test_runtime_with_props();
    bytes.resize(768, 0);
    bytes[0x46..0x48].copy_from_slice(&3u16.to_le_bytes());
    bytes[0x15c..0x160].copy_from_slice(&0x200u32.to_le_bytes());
    bytes[0x184..0x188].copy_from_slice(&0x240u32.to_le_bytes());
    bytes[0x198..0x1a0].copy_from_slice(b".orrrot\0");
    bytes[0x1a8..0x1ac].copy_from_slice(&64u32.to_le_bytes());
    bytes[0x1ac..0x1b0].copy_from_slice(&0x280u32.to_le_bytes());
    bytes[0x1bc..0x1c0].copy_from_slice(&0x4000_0040u32.to_le_bytes());
    bytes[0x1c0..].fill(0);
    bytes[0x200..0x200 + b"ORR_MESH_PLAYER_V1".len()].copy_from_slice(b"ORR_MESH_PLAYER_V1");
    bytes[0x240..0x240 + b"ORR_MESH_PROPS_V1".len()].copy_from_slice(b"ORR_MESH_PROPS_V1");
    bytes[0x280..0x280 + b"ORR_MESH_ROTATION_V1".len()].copy_from_slice(b"ORR_MESH_ROTATION_V1");
    bytes
}

fn test_runtime_with_camera() -> Vec<u8> {
    let mut bytes = test_runtime_with_rotation();
    bytes[0x46..0x48].copy_from_slice(&4u16.to_le_bytes());
    bytes[0x1c0..0x1c8].copy_from_slice(b".orrcam\0");
    bytes[0x1d0..0x1d4].copy_from_slice(&64u32.to_le_bytes());
    bytes[0x1d4..0x1d8].copy_from_slice(&0x2c0u32.to_le_bytes());
    bytes[0x1e4..0x1e8].copy_from_slice(&0x4000_0040u32.to_le_bytes());
    bytes[0x2c0..0x2c0 + b"ORR_MESH_CAMERA_V1".len()].copy_from_slice(b"ORR_MESH_CAMERA_V1");
    bytes
}

fn test_runtime_with_material() -> Vec<u8> {
    let mut bytes = test_runtime_with_camera();
    bytes.resize(1024, 0);
    bytes[0x200..0x240].fill(0);
    bytes[0x46..0x48].copy_from_slice(&5u16.to_le_bytes());
    bytes[0x15c..0x160].copy_from_slice(&0x340u32.to_le_bytes());
    bytes[0x1e8..0x1f0].copy_from_slice(b".orrmat\0");
    bytes[0x1f0..0x1f4].copy_from_slice(&64u32.to_le_bytes());
    bytes[0x1f8..0x1fc].copy_from_slice(&64u32.to_le_bytes());
    bytes[0x1fc..0x200].copy_from_slice(&0x300u32.to_le_bytes());
    bytes[0x20c..0x210].copy_from_slice(&0x4000_0040u32.to_le_bytes());
    bytes[0x300..0x300 + b"ORR_MESH_MATERIAL_V1".len()].copy_from_slice(b"ORR_MESH_MATERIAL_V1");
    bytes[0x340..0x340 + b"ORR_MESH_PLAYER_V1".len()].copy_from_slice(b"ORR_MESH_PLAYER_V1");
    bytes
}

fn test_runtime_with_surface() -> Vec<u8> {
    let mut bytes = test_runtime_with_material();
    bytes.resize(1088, 0);
    bytes[0x46..0x48].copy_from_slice(&6u16.to_le_bytes());
    bytes[0x210..0x218].copy_from_slice(b".orrsurf");
    bytes[0x220..0x224].copy_from_slice(&64u32.to_le_bytes());
    bytes[0x224..0x228].copy_from_slice(&0x380u32.to_le_bytes());
    bytes[0x234..0x238].copy_from_slice(&0x4000_0040u32.to_le_bytes());
    bytes[0x380..0x380 + b"ORR_MESH_SURFACE_V1".len()].copy_from_slice(b"ORR_MESH_SURFACE_V1");
    bytes
}

fn surface_settings(uv_offset: [f32; 2]) -> crate::mesh_surface::Settings {
    crate::mesh_surface::Settings {
        material: 0,
        uv_offset,
        uv_scale: [1.0, 1.0],
        uv_quarter_turns: 0,
        pivot: [0.0, 0.0, 0.0],
    }
}

#[cfg(feature = "mesh-animated-models")]
fn test_runtime_with_animation() -> Vec<u8> {
    let mut bytes = test_runtime_with_material();
    bytes.resize(1088, 0);
    bytes[0x46..0x48].copy_from_slice(&6u16.to_le_bytes());
    bytes[0x210..0x218].copy_from_slice(b".orranim");
    bytes[0x218..0x21c].copy_from_slice(&64u32.to_le_bytes());
    bytes[0x220..0x224].copy_from_slice(&64u32.to_le_bytes());
    bytes[0x224..0x228].copy_from_slice(&0x380u32.to_le_bytes());
    bytes[0x234..0x238].copy_from_slice(&0x4000_0040u32.to_le_bytes());
    bytes[0x380..0x380 + b"ORR_MESH_ANIMATION_V1".len()].copy_from_slice(b"ORR_MESH_ANIMATION_V1");
    bytes
}

fn set_art_tint(scene: &std::path::Path, revision: [u8; 32], tint: [f32; 4]) {
    add_props(scene, revision);
    let (document, props_hash) = mesh_props::load(scene, revision).unwrap().unwrap();
    let filename = scene.file_name().unwrap().to_str().unwrap();
    let mut art: serde_json::Value =
        serde_json::from_slice(&document.props[0].art.encode_embedded(filename).unwrap()).unwrap();
    art["tint"] = serde_json::json!(tint);
    mesh_props::save(scene, None, Some(&props_hash), revision).unwrap();
    fs::write(
        crate::mesh_art::sidecar_path(scene).unwrap(),
        serde_json::to_vec(&art).unwrap(),
    )
    .unwrap();
    crate::mesh_art::load(scene, revision).unwrap().unwrap();
}

fn set_prop_tint(scene: &std::path::Path, revision: [u8; 32], tint: [f32; 4]) {
    add_props(scene, revision);
    let path = mesh_props::sidecar_path(scene).unwrap();
    let mut props: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    props["props"][0]["art"]["tint"] = serde_json::json!(tint);
    fs::write(&path, serde_json::to_vec(&props).unwrap()).unwrap();
    mesh_props::load(scene, revision).unwrap().unwrap();
}

fn save_surface(
    scene: &std::path::Path,
    revision: [u8; 32],
    settings: crate::mesh_surface::Settings,
    art_only: bool,
) {
    add_props(scene, revision);
    let (mut document, props_hash) = mesh_props::load(scene, revision).unwrap().unwrap();
    document.props[0].art = document.props[0]
        .art
        .clone()
        .with_surface(Some(settings))
        .unwrap();
    if art_only {
        crate::mesh_art::save(scene, Some(&document.props[0].art), None).unwrap();
        fs::remove_file(mesh_props::sidecar_path(scene).unwrap()).unwrap();
    } else {
        mesh_props::save(scene, Some(&document), Some(&props_hash), revision).unwrap();
    }
}

fn add_camera(scene: &std::path::Path, revision: [u8; 32]) -> Vec<u8> {
    let camera = CameraDocument::from_orbit(OrbitCamera::new([1.0, 2.0, 3.0], 0.4, 0.2, 8.0))
        .unwrap()
        .with_projection(
            CameraProjection::Orthographic { half_height: 4.0 },
            0.1,
            500.0,
        )
        .unwrap();
    crate::mesh_camera_store::save(scene, Some(&camera), None, revision).unwrap();
    fs::read(crate::mesh_camera_store::sidecar_path(scene).unwrap()).unwrap()
}

#[test]
fn rotation_capability_requires_a_dedicated_bounded_marker_section() {
    super::validate_runtime_for(&test_runtime_with_rotation(), true, true).unwrap();
    assert!(super::validate_runtime_for(&test_runtime_with_props(), true, true).is_err());
    let mut bytes = test_runtime_with_rotation();
    bytes[0x280..0x280 + b"ORR_MESH_ROTATION_V1".len()].fill(0);
    bytes[0x100..0x100 + b"ORR_MESH_ROTATION_V1".len()].copy_from_slice(b"ORR_MESH_ROTATION_V1");
    assert!(super::validate_runtime_for(&bytes, true, true).is_err());
    let mut bytes = test_runtime_with_rotation();
    bytes[0x1ac..0x1b0].copy_from_slice(&0x100u32.to_le_bytes());
    assert!(super::validate_runtime_for(&bytes, true, true).is_err());
    let mut bytes = test_runtime_with_rotation();
    bytes[0x46..0x48].copy_from_slice(&4u16.to_le_bytes());
    bytes[0x1c0..0x1c8].copy_from_slice(b".orrrot\0");
    assert!(super::validate_runtime_for(&bytes, true, true).is_err());
    super::validate_runtime_for(&test_runtime(), false, false).unwrap();
    super::validate_runtime_for(&test_runtime_with_props(), true, false).unwrap();
}

#[test]
fn explicit_identity_fields_still_require_rotation_aware_players_for_export_and_open() {
    for art_only in [false, true] {
        let (temporary, scene, runtime, checksum) = fixture();
        let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
        add_props(&scene, revision);
        let (document, props_hash) = mesh_props::load(&scene, revision).unwrap().unwrap();
        if art_only {
            let filename = scene.file_name().unwrap().to_str().unwrap();
            let mut art: serde_json::Value =
                serde_json::from_slice(&document.props[0].art.encode_embedded(filename).unwrap())
                    .unwrap();
            art["rotation"] = serde_json::json!([0, 0, 0, 1]);
            mesh_props::save(&scene, None, Some(&props_hash), revision).unwrap();
            fs::write(
                crate::mesh_art::sidecar_path(&scene).unwrap(),
                serde_json::to_vec(&art).unwrap(),
            )
            .unwrap();
            crate::mesh_art::load(&scene, revision).unwrap().unwrap();
        } else {
            let path = mesh_props::sidecar_path(&scene).unwrap();
            let mut props: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            props["props"][0]["art"]["rotation"] = serde_json::json!([0, 0, 0, 1]);
            fs::write(path, serde_json::to_vec(&props).unwrap()).unwrap();
            mesh_props::load(&scene, revision).unwrap().unwrap();
        }
        let legacy = test_runtime_with_props();
        fs::write(&runtime, &legacy).unwrap();
        let destination = temporary.path().join("identity-field-bundle");
        assert!(export(&scene, checksum, &destination, &runtime).is_err());
        assert!(!destination.exists());
        fs::write(&runtime, test_runtime_with_rotation()).unwrap();
        export(&scene, checksum, &destination, &runtime).unwrap();
        open_bundle(&destination).unwrap();
        fs::write(destination.join(RUNTIME_FILENAME), &legacy).unwrap();
        let manifest_path = destination.join(MANIFEST_FILENAME);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["runtime"]["bytes"] = serde_json::json!(legacy.len());
        manifest["runtime"]["sha256"] = serde_json::json!(super::digest(&legacy));
        fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(open_bundle(&destination).is_err());
    }
}

#[test]
fn rotated_props_and_art_each_require_capability_and_reopen_from_embedded_bytes() {
    for art_only in [false, true] {
        let (temporary, scene, runtime, checksum) = fixture();
        let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
        add_props(&scene, revision);
        let (mut document, hash) = mesh_props::load(&scene, revision).unwrap().unwrap();
        let rotation = crate::mesh_rotation::from_degrees([20.0, 35.0, -15.0]).unwrap();
        document.props[0].art = document.props[0]
            .art
            .clone()
            .with_rotation(rotation)
            .unwrap();
        if art_only {
            crate::mesh_art::save(&scene, Some(&document.props[0].art), None).unwrap();
            mesh_props::save(&scene, None, Some(&hash), revision).unwrap();
        } else {
            mesh_props::save(&scene, Some(&document), Some(&hash), revision).unwrap();
            fs::write(&runtime, test_runtime_with_props()).unwrap();
        }
        let destination = temporary.path().join("rotated-bundle");
        assert!(export(&scene, checksum, &destination, &runtime).is_err());
        assert!(!destination.exists());
        fs::write(&runtime, test_runtime_with_rotation()).unwrap();
        let receipt = export(&scene, checksum, &destination, &runtime).unwrap();
        assert_eq!(receipt.props_sha256.is_some(), !art_only);
        assert_eq!(receipt.sidecar_sha256.is_some(), art_only);
        open_bundle(&destination).unwrap();
        fs::rename(
            scene.parent().unwrap().join("verified-static-package"),
            scene.parent().unwrap().join("package-hidden"),
        )
        .unwrap();
        let bundle_scene = destination.join("walk.scene.yaml");
        let loaded = if art_only {
            crate::mesh_art::load(&bundle_scene, revision)
                .unwrap()
                .unwrap()
                .0
        } else {
            mesh_props::load(&bundle_scene, revision)
                .unwrap()
                .unwrap()
                .0
                .props
                .remove(0)
                .art
        };
        for (actual, expected) in loaded.rotation().into_iter().zip(rotation) {
            assert!((actual - expected).abs() < 1.0e-6);
        }
        open_bundle(&destination).unwrap();
    }
}

fn add_props(scene: &std::path::Path, revision: [u8; 32]) {
    let root = scene.parent().unwrap();
    let package_root = root.join("verified-static-package");
    fs::create_dir(&package_root).unwrap();
    fs::write(package_root.join("model.glb"), STATIC_GLB).unwrap();
    fs::write(package_root.join("LICENSE.txt"), LICENSE).unwrap();
    let runtime = orr_package::Runtime::content_only();
    let manifest = orr_package::Manifest {
        schema: 1,
        name: "verified-static".into(),
        version: "1.0.0".into(),
        engine: format!("={}", runtime.engine_version),
        capabilities: std::collections::BTreeSet::from(["models".into()]),
        dependencies: std::collections::BTreeMap::new(),
        files: std::collections::BTreeSet::from(["LICENSE.txt".into(), "model.glb".into()]),
    };
    fs::write(
        package_root.join("orr.package.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let art = Snapshot::from_package(&package_root, "model.glb", "LICENSE.txt", revision).unwrap();
    let document = Document {
        props: vec![Prop {
            id: 1,
            name: "crate".into(),
            art,
        }],
    };
    mesh_props::save(scene, Some(&document), None, revision).unwrap();
}

fn fixture() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    std::path::PathBuf,
    u64,
) {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene = root.join("walk.scene.yaml");
    let runtime = root.join("player.exe");
    let (yaml, _, checksum) = admitted_scene();
    fs::write(&scene, yaml).unwrap();
    fs::write(&runtime, test_runtime()).unwrap();
    (temporary, scene, runtime, checksum)
}

#[test]
fn export_opens_self_contained_bundle_without_changing_sources() {
    let (temporary, scene, runtime, checksum) = fixture();
    let scene_before = fs::read(&scene).unwrap();
    let runtime_before = fs::read(&runtime).unwrap();
    let destination = temporary.path().join("bundle");

    let receipt = export(&scene, checksum, &destination, &runtime).unwrap();
    let prepared = open_bundle(&destination).unwrap();

    assert_eq!(receipt.checksum, checksum);
    assert_eq!(prepared.checksum, checksum);
    assert_eq!(receipt.graph_revision, prepared.graph_revision);
    assert_eq!(prepared.runtime_sha256, receipt.runtime_sha256);
    assert_eq!(fs::read(&scene).unwrap(), scene_before);
    assert_eq!(fs::read(&runtime).unwrap(), runtime_before);
    assert!(receipt.scene.is_file());
    assert!(receipt.runtime.is_file());
    assert!(receipt.manifest.is_file());
    assert_eq!(receipt.manifest.file_name().unwrap(), MANIFEST_FILENAME);
    assert_eq!(receipt.scene.file_name().unwrap(), "walk.scene.yaml");
    assert_eq!(receipt.runtime.file_name().unwrap(), RUNTIME_FILENAME);
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&receipt.manifest).unwrap()).unwrap();
    assert!(manifest.get("props").is_none());
    assert!(manifest.get("camera").is_none());
    assert!(manifest.get("animation").is_none());
}

#[test]
fn export_refuses_existing_destination_without_changing_it() {
    let (temporary, scene, runtime, checksum) = fixture();
    let destination = temporary.path().join("bundle");
    fs::create_dir(&destination).unwrap();
    let sentinel = destination.join("sentinel");
    fs::write(&sentinel, b"keep").unwrap();

    assert!(export(&scene, checksum, &destination, &runtime).is_err());
    assert_eq!(fs::read(sentinel).unwrap(), b"keep");
}

#[test]
fn open_bundle_rejects_tampered_scene_and_unknown_manifest_fields() {
    let (temporary, scene, runtime, checksum) = fixture();
    let destination = temporary.path().join("bundle");
    export(&scene, checksum, &destination, &runtime).unwrap();
    let bundle_scene = destination.join("walk.scene.yaml");
    fs::write(&bundle_scene, b"tampered").unwrap();
    assert!(open_bundle(&destination).is_err());
    fs::write(&bundle_scene, fs::read(&scene).unwrap()).unwrap();

    let manifest_path = destination.join(MANIFEST_FILENAME);
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    let mut object = manifest.as_object().unwrap().clone();
    object.insert("unexpected".into(), serde_json::Value::Bool(true));
    fs::write(&manifest_path, serde_json::to_vec(&object).unwrap()).unwrap();
    assert!(open_bundle(&destination).is_err());
}

#[test]
fn runtime_validation_requires_x64_pe32_plus_and_marker() {
    let mut runtime = test_runtime();
    validate_runtime(&runtime, false).unwrap();
    runtime[0x44..0x46].copy_from_slice(&0x14cu16.to_le_bytes());
    assert!(validate_runtime(&runtime, false).is_err());
}

#[test]
fn runtime_marker_outside_executable_section_is_not_accepted() {
    let mut runtime = test_runtime();
    runtime[0x100..0x100 + b"ORR_MESH_PLAYER_V1".len()].copy_from_slice(b"ORR_MESH_PLAYER_V1");
    runtime[0x180..0x180 + b"ORR_MESH_PLAYER_V1".len()].fill(0);

    assert!(validate_runtime(&runtime, false).is_err());
}

#[test]
fn open_bundle_rejects_unlisted_files() {
    let (temporary, scene, runtime, checksum) = fixture();
    let destination = temporary.path().join("bundle");
    export(&scene, checksum, &destination, &runtime).unwrap();
    fs::write(destination.join("extra.txt"), b"not in the manifest").unwrap();

    assert!(open_bundle(&destination).is_err());
}

#[test]
fn props_sidecar_is_admitted_and_requires_props_capable_runtime() {
    let (temporary, scene, runtime, checksum) = fixture();
    let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
    add_props(&scene, revision);
    let props_path = mesh_props::sidecar_path(&scene).unwrap();
    let props_before = fs::read(&props_path).unwrap();
    let destination = temporary.path().join("with-props");

    assert!(export(&scene, checksum, &destination, &runtime).is_err());
    assert!(!destination.exists());
    let props_runtime = temporary.path().join("props-player.exe");
    fs::write(&props_runtime, test_runtime_with_props()).unwrap();
    let receipt = export(&scene, checksum, &destination, &props_runtime).unwrap();
    let prepared = open_bundle(&destination).unwrap();
    let bundle_props = destination.join(format!("walk.scene.yaml{PROPS_SUFFIX}"));
    let expected_props_hash = super::digest(&props_before);
    assert_eq!(
        receipt.props_sha256.as_deref(),
        Some(expected_props_hash.as_str())
    );
    assert_eq!(fs::read(&bundle_props).unwrap(), props_before);
    assert_eq!(fs::read(&props_path).unwrap(), props_before);
    assert_eq!(prepared.graph_revision, revision);
}

#[test]
fn props_bundle_rejects_tampering_and_missing_inventory() {
    let (temporary, scene, _, checksum) = fixture();
    let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
    add_props(&scene, revision);
    let props_runtime = temporary.path().join("props-player.exe");
    fs::write(&props_runtime, test_runtime_with_props()).unwrap();
    let destination = temporary.path().join("bundle");
    export(&scene, checksum, &destination, &props_runtime).unwrap();
    let props_path = destination.join(format!("walk.scene.yaml{PROPS_SUFFIX}"));
    let original = fs::read(&props_path).unwrap();
    fs::write(&props_path, b"tampered").unwrap();
    assert!(open_bundle(&destination).is_err());
    fs::write(&props_path, original).unwrap();
    fs::remove_file(&props_path).unwrap();
    assert!(open_bundle(&destination).is_err());
}

#[test]
fn props_runtime_marker_must_be_in_a_distinct_pe_section() {
    validate_runtime(&test_runtime_with_props(), true).unwrap();
    assert!(validate_runtime(&test_runtime(), true).is_err());
    let mut bytes = test_runtime_with_props();
    bytes[0x200..0x200 + b"ORR_MESH_PROPS_V1".len()].fill(0);
    bytes[0x100..0x100 + b"ORR_MESH_PROPS_V1".len()].copy_from_slice(b"ORR_MESH_PROPS_V1");
    assert!(validate_runtime(&bytes, true).is_err());
    validate_runtime(&test_runtime(), false).unwrap();
}

#[test]
fn props_bundle_rejects_invalid_admission_even_with_updated_file_hash() {
    let (temporary, scene, _, checksum) = fixture();
    let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
    add_props(&scene, revision);
    let runtime = temporary.path().join("props-player.exe");
    fs::write(&runtime, test_runtime_with_props()).unwrap();
    let destination = temporary.path().join("bundle");
    export(&scene, checksum, &destination, &runtime).unwrap();
    let props_path = destination.join(format!("walk.scene.yaml{PROPS_SUFFIX}"));
    let mut props: serde_json::Value =
        serde_json::from_slice(&fs::read(&props_path).unwrap()).unwrap();
    props["graph_revision"] = serde_json::to_value([0_u8; 32]).unwrap();
    let bytes = serde_json::to_vec(&props).unwrap();
    fs::write(&props_path, &bytes).unwrap();
    let manifest_path = destination.join(MANIFEST_FILENAME);
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["props"]["bytes"] = serde_json::json!(bytes.len());
    manifest["props"]["sha256"] = serde_json::json!(super::digest(&bytes));
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(open_bundle(&destination).is_err());
}

#[test]
fn camera_sidecar_is_copied_and_re_admitted_only_by_a_camera_capable_player() {
    let (temporary, scene, runtime, checksum) = fixture();
    let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
    let camera_before = add_camera(&scene, revision);
    let camera_path = crate::mesh_camera_store::sidecar_path(&scene).unwrap();
    let destination = temporary.path().join("with-camera");

    assert!(export(&scene, checksum, &destination, &runtime).is_err());
    assert!(!destination.exists());

    let camera_runtime = temporary.path().join("camera-player.exe");
    fs::write(&camera_runtime, test_runtime_with_camera()).unwrap();
    let receipt = export(&scene, checksum, &destination, &camera_runtime).unwrap();
    let prepared = open_bundle(&destination).unwrap();
    let bundled_camera = destination.join("walk.scene.yaml.mesh-camera.json");
    let expected_hash = super::digest(&camera_before);
    assert_eq!(
        receipt.camera_sha256.as_deref(),
        Some(expected_hash.as_str())
    );
    assert_eq!(fs::read(&bundled_camera).unwrap(), camera_before);
    assert_eq!(fs::read(&camera_path).unwrap(), camera_before);
    assert_eq!(prepared.graph_revision, revision);
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&receipt.manifest).unwrap()).unwrap();
    assert_eq!(
        manifest["camera"]["name"],
        "walk.scene.yaml.mesh-camera.json"
    );
    assert_eq!(manifest["camera"]["sha256"], expected_hash);
}

#[test]
fn camera_bundle_rejects_tampering_missing_files_and_invalid_admission() {
    let (temporary, scene, _, checksum) = fixture();
    let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
    add_camera(&scene, revision);
    let runtime = temporary.path().join("camera-player.exe");
    fs::write(&runtime, test_runtime_with_camera()).unwrap();
    let destination = temporary.path().join("bundle");
    export(&scene, checksum, &destination, &runtime).unwrap();

    let camera_path = destination.join("walk.scene.yaml.mesh-camera.json");
    let original = fs::read(&camera_path).unwrap();
    fs::write(&camera_path, b"tampered").unwrap();
    assert!(open_bundle(&destination).is_err());
    fs::write(&camera_path, &original).unwrap();
    fs::remove_file(&camera_path).unwrap();
    assert!(open_bundle(&destination).is_err());
    fs::write(&camera_path, &original).unwrap();

    let mut invalid: serde_json::Value = serde_json::from_slice(&original).unwrap();
    invalid["graph_revision"] = serde_json::to_value([0_u8; 32]).unwrap();
    let invalid_bytes = serde_json::to_vec(&invalid).unwrap();
    fs::write(&camera_path, &invalid_bytes).unwrap();
    let manifest_path = destination.join(MANIFEST_FILENAME);
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["camera"]["bytes"] = serde_json::json!(invalid_bytes.len());
    manifest["camera"]["sha256"] = serde_json::json!(super::digest(&invalid_bytes));
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(open_bundle(&destination).is_err());
}

#[test]
fn camera_runtime_marker_must_be_in_a_distinct_bounded_pe_section() {
    super::validate_runtime_with_camera(&test_runtime_with_camera(), false, false, true).unwrap();
    assert!(
        super::validate_runtime_with_camera(&test_runtime_with_rotation(), false, false, true)
            .is_err()
    );
    let mut bytes = test_runtime_with_camera();
    bytes[0x2c0..0x2c0 + b"ORR_MESH_CAMERA_V1".len()].fill(0);
    bytes[0x100..0x100 + b"ORR_MESH_CAMERA_V1".len()].copy_from_slice(b"ORR_MESH_CAMERA_V1");
    assert!(super::validate_runtime_with_camera(&bytes, false, false, true).is_err());
    let mut bytes = test_runtime_with_camera();
    bytes[0x1d4..0x1d8].copy_from_slice(&0x100u32.to_le_bytes());
    assert!(super::validate_runtime_with_camera(&bytes, false, false, true).is_err());
    let mut bytes = test_runtime_with_camera();
    bytes[0x1c0..0x1c8].copy_from_slice(b".orrrot\0");
    assert!(super::validate_runtime_with_camera(&bytes, false, false, true).is_err());
    super::validate_runtime_with_camera(&test_runtime(), false, false, false).unwrap();
}

#[test]
fn explicit_material_fields_require_material_capability_even_when_identity() {
    for art_only in [false, true] {
        for tint in [[1.0, 1.0, 1.0, 1.0], [0.25, 0.5, 0.75, 1.0]] {
            let (temporary, scene, runtime, checksum) = fixture();
            let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
            if art_only {
                set_art_tint(&scene, revision, tint);
            } else {
                set_prop_tint(&scene, revision, tint);
            }
            let legacy_runtime = if art_only {
                test_runtime()
            } else {
                test_runtime_with_props()
            };
            fs::write(&runtime, &legacy_runtime).unwrap();
            let destination = temporary.path().join("material-bundle");
            assert!(export(&scene, checksum, &destination, &runtime).is_err());
            assert!(!destination.exists());

            let material_runtime = temporary.path().join("material-player.exe");
            fs::write(&material_runtime, test_runtime_with_material()).unwrap();
            let receipt = export(&scene, checksum, &destination, &material_runtime).unwrap();
            let prepared = open_bundle(&destination).unwrap();
            assert_eq!(prepared.checksum, checksum);
            if art_only {
                let (art, _) = crate::mesh_art::load(&prepared.scene, revision)
                    .unwrap()
                    .unwrap();
                assert_eq!(art.tint(), tint);
                assert!(receipt.sidecar_sha256.is_some());
                assert_eq!(receipt.props_sha256, None);
            } else {
                let (document, _) = mesh_props::load(&prepared.scene, revision)
                    .unwrap()
                    .unwrap();
                assert_eq!(document.props[0].art.tint(), tint);
                assert!(receipt.props_sha256.is_some());
                assert_eq!(receipt.sidecar_sha256, None);
            }

            fs::write(destination.join(RUNTIME_FILENAME), &legacy_runtime).unwrap();
            let manifest_path = destination.join(MANIFEST_FILENAME);
            let mut manifest: serde_json::Value =
                serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
            manifest["runtime"]["bytes"] = serde_json::json!(legacy_runtime.len());
            manifest["runtime"]["sha256"] = serde_json::json!(super::digest(&legacy_runtime));
            fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
            assert!(open_bundle(&destination).is_err());
        }
    }
}

#[test]
fn material_runtime_marker_requires_its_own_bounded_pe_section() {
    super::validate_runtime_with_material(&test_runtime_with_material(), false, false, false, true)
        .unwrap();
    assert!(super::validate_runtime_with_material(
        &test_runtime_with_camera(),
        false,
        false,
        false,
        true
    )
    .is_err());

    let mut bytes = test_runtime_with_material();
    bytes[0x300..0x300 + b"ORR_MESH_MATERIAL_V1".len()].fill(0);
    bytes[0x100..0x100 + b"ORR_MESH_MATERIAL_V1".len()].copy_from_slice(b"ORR_MESH_MATERIAL_V1");
    assert!(super::validate_runtime_with_material(&bytes, false, false, false, true).is_err());

    let mut bytes = test_runtime_with_material();
    bytes[0x1fc..0x200].copy_from_slice(&0x100u32.to_le_bytes());
    assert!(super::validate_runtime_with_material(&bytes, false, false, false, true).is_err());

    let mut bytes = test_runtime_with_material();
    bytes[0x1f8..0x1fc].copy_from_slice(&8u32.to_le_bytes());
    assert!(super::validate_runtime_with_material(&bytes, false, false, false, true).is_err());

    super::validate_runtime_with_material(&test_runtime(), false, false, false, false).unwrap();
}

#[test]
fn explicit_surface_fields_require_a_surface_capable_player_for_export_and_open() {
    for art_only in [false, true] {
        for uv_offset in [[0.0, 0.0], [0.25, -0.5]] {
            let (temporary, scene, runtime, checksum) = fixture();
            let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
            let settings = surface_settings(uv_offset);
            save_surface(&scene, revision, settings, art_only);
            let legacy_runtime = test_runtime_with_material();
            fs::write(&runtime, &legacy_runtime).unwrap();
            let destination = temporary.path().join("surface-bundle");
            assert!(export(&scene, checksum, &destination, &runtime).is_err());
            assert!(!destination.exists());

            let capable_runtime = temporary.path().join("surface-player.exe");
            fs::write(&capable_runtime, test_runtime_with_surface()).unwrap();
            let receipt = export(&scene, checksum, &destination, &capable_runtime).unwrap();
            let manifest: serde_json::Value =
                serde_json::from_slice(&fs::read(&receipt.manifest).unwrap()).unwrap();
            assert!(manifest.get("surface").is_none());
            let prepared = open_bundle(&destination).unwrap();
            assert_eq!(prepared.checksum, checksum);
            assert_eq!(receipt.props_sha256.is_some(), !art_only);
            assert_eq!(receipt.sidecar_sha256.is_some(), art_only);
            let bundle_scene = destination.join("walk.scene.yaml");
            if art_only {
                let (art, _) = crate::mesh_art::load(&bundle_scene, revision)
                    .unwrap()
                    .unwrap();
                assert_eq!(art.surface(), Some(settings));
            } else {
                let (document, _) = mesh_props::load(&bundle_scene, revision).unwrap().unwrap();
                assert_eq!(document.props[0].art.surface(), Some(settings));
            }

            fs::write(destination.join(RUNTIME_FILENAME), &legacy_runtime).unwrap();
            let mut manifest: serde_json::Value =
                serde_json::from_slice(&fs::read(&receipt.manifest).unwrap()).unwrap();
            manifest["runtime"]["bytes"] = serde_json::json!(legacy_runtime.len());
            manifest["runtime"]["sha256"] = serde_json::json!(super::digest(&legacy_runtime));
            fs::write(&receipt.manifest, serde_json::to_vec(&manifest).unwrap()).unwrap();
            assert!(open_bundle(&destination).is_err());
        }
    }
}

#[test]
fn surface_marker_must_be_a_distinct_bounded_pe_section() {
    super::validate_runtime_with_surface(&test_runtime_with_surface(), false).unwrap();
    assert!(super::validate_runtime_with_surface(&test_runtime_with_material(), false).is_err());

    let mut bytes = test_runtime_with_surface();
    bytes[0x380..0x380 + b"ORR_MESH_SURFACE_V1".len()].fill(0);
    bytes[0x100..0x100 + b"ORR_MESH_SURFACE_V1".len()].copy_from_slice(b"ORR_MESH_SURFACE_V1");
    assert!(super::validate_runtime_with_surface(&bytes, false).is_err());

    let mut bytes = test_runtime_with_surface();
    bytes[0x224..0x228].copy_from_slice(&0x100u32.to_le_bytes());
    assert!(super::validate_runtime_with_surface(&bytes, false).is_err());
    super::validate_runtime_with_material(
        &test_runtime_with_material(),
        false,
        false,
        false,
        false,
    )
    .unwrap();
}

#[test]
fn interactive_export_records_and_reopens_input_profile() {
    let (temporary, scene, runtime, checksum) = fixture();
    let destination = temporary.path().join("interactive-bundle");
    fs::write(&runtime, test_runtime_with_input()).unwrap();

    let receipt = export_interactive(&scene, checksum, &destination, &runtime).unwrap();
    let prepared = open_bundle(&destination).unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&receipt.manifest).unwrap()).unwrap();

    assert_eq!(receipt.input_profile, Some(InputProfile::MeshClickToMoveV1));
    assert_eq!(
        prepared.input_profile,
        Some(InputProfile::MeshClickToMoveV1)
    );
    assert_eq!(
        manifest["input_profile"],
        serde_json::json!("mesh-click-to-move-v1")
    );
    let mut bundle_names = fs::read_dir(&destination)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    bundle_names.sort();
    assert_eq!(
        bundle_names,
        ["orr.mesh-bundle.json", "orr_mesh_player.exe", "walk.scene.yaml"]
    );
}

#[test]
fn interactive_export_requires_marker_and_open_rejects_profile_mismatch() {
    let (temporary, scene, runtime, checksum) = fixture();
    let destination = temporary.path().join("interactive-bundle");
    let error = export_interactive(&scene, checksum, &destination, &runtime).unwrap_err();
    assert!(error.contains("mesh click-to-move input support"));
    assert!(!destination.exists());

    fs::write(&runtime, test_runtime_with_input()).unwrap();
    let receipt = export_interactive(&scene, checksum, &destination, &runtime).unwrap();
    let legacy_runtime = test_runtime();
    fs::write(destination.join(RUNTIME_FILENAME), &legacy_runtime).unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&receipt.manifest).unwrap()).unwrap();
    let mut manifest = manifest;
    manifest["runtime"]["bytes"] = serde_json::json!(legacy_runtime.len());
    manifest["runtime"]["sha256"] = serde_json::json!(super::digest(&legacy_runtime));
    fs::write(&receipt.manifest, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let error = open_bundle(&destination).unwrap_err();
    assert!(error.contains("mesh click-to-move input support"));
}

#[test]
fn legacy_bundle_is_replay_only_and_unknown_profile_is_rejected() {
    let (temporary, scene, runtime, checksum) = fixture();
    let destination = temporary.path().join("legacy-bundle");
    let receipt = export(&scene, checksum, &destination, &runtime).unwrap();
    let prepared = open_bundle(&destination).unwrap();
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&receipt.manifest).unwrap()).unwrap();
    assert!(manifest.get("input_profile").is_none());
    assert_eq!(prepared.input_profile, None);

    manifest["input_profile"] = serde_json::json!("unknown-input-profile");
    fs::write(&receipt.manifest, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(open_bundle(&destination).is_err());
}

#[test]
fn input_capability_marker_must_be_in_its_own_pe_section() {
    validate_runtime_profile(
        &test_runtime_with_input(),
        Some(InputProfile::MeshClickToMoveV1),
    )
    .unwrap();

    let mut bytes = test_runtime_with_input();
    bytes[0x200..0x200 + b"ORR_MESH_INPUT_V1".len()].fill(0);
    bytes[0x100..0x100 + b"ORR_MESH_INPUT_V1".len()].copy_from_slice(b"ORR_MESH_INPUT_V1");
    assert!(validate_runtime_profile(&bytes, Some(InputProfile::MeshClickToMoveV1)).is_err());
}

#[test]
fn camera_sidecar_with_another_graph_revision_is_refused_before_staging() {
    let (temporary, scene, _runtime, checksum) = fixture();
    let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
    add_camera(&scene, [revision[0].wrapping_add(1); 32]);
    let camera_runtime = temporary.path().join("camera-player.exe");
    fs::write(&camera_runtime, test_runtime_with_camera()).unwrap();
    let destination = temporary.path().join("bundle");

    assert!(export(&scene, checksum, &destination, &camera_runtime).is_err());
    assert!(!destination.exists());
}

#[cfg(feature = "mesh-animated-models")]
#[test]
fn animation_bundle_requires_marker_and_reopens_without_source_package() {
    let (temporary, scene, runtime, checksum) = fixture();
    let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
    let package = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/animation_demo")
        .canonicalize()
        .unwrap();
    let snapshot = crate::mesh_animation::Snapshot::from_package(
        &package,
        "animated.glb",
        "LICENSE.txt",
        revision,
    )
    .unwrap();
    let sidecar_hash = crate::mesh_animation_store::save(&scene, Some(&snapshot), None, revision)
        .unwrap()
        .unwrap();
    let destination = temporary.path().join("animation-bundle");

    assert!(export(&scene, checksum, &destination, &runtime).is_err());
    assert!(!destination.exists());

    let capable_runtime = temporary.path().join("animation-player.exe");
    fs::write(&capable_runtime, test_runtime_with_animation()).unwrap();
    let receipt = export(&scene, checksum, &destination, &capable_runtime).unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&receipt.manifest).unwrap()).unwrap();
    assert_eq!(
        manifest["animation"]["name"],
        "walk.scene.yaml.mesh-animation.json"
    );
    assert_eq!(manifest["animation"]["sha256"], sidecar_hash);
    assert_eq!(
        receipt.animation_sha256.as_deref(),
        Some(sidecar_hash.as_str())
    );
    assert_eq!(open_bundle(&destination).unwrap().checksum, checksum);

    fs::remove_file(crate::mesh_animation_store::sidecar_path(&scene).unwrap()).unwrap();
    assert_eq!(open_bundle(&destination).unwrap().checksum, checksum);
}

#[cfg(feature = "mesh-animated-models")]
#[test]
fn animation_bundle_rejects_wrong_marker_and_revision_before_publish() {
    let (temporary, scene, _runtime, checksum) = fixture();
    let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
    let package = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/animation_demo")
        .canonicalize()
        .unwrap();
    let snapshot = crate::mesh_animation::Snapshot::from_package(
        &package,
        "animated.glb",
        "LICENSE.txt",
        revision,
    )
    .unwrap();
    let existing_hash = crate::mesh_animation_store::save(&scene, Some(&snapshot), None, revision)
        .unwrap()
        .unwrap();
    let destination = temporary.path().join("animation-bundle");

    let mut wrong_marker = test_runtime_with_animation();
    wrong_marker[0x380..0x380 + b"ORR_MESH_ANIMATION_V1".len()].fill(0);
    wrong_marker[0x100..0x100 + b"ORR_MESH_ANIMATION_V1".len()]
        .copy_from_slice(b"ORR_MESH_ANIMATION_V1");
    let incapable_runtime = temporary.path().join("incapable-player.exe");
    fs::write(&incapable_runtime, wrong_marker).unwrap();
    assert!(export(&scene, checksum, &destination, &incapable_runtime).is_err());
    assert!(!destination.exists());

    let bad_revision = [revision[0].wrapping_add(1); 32];
    let bad_snapshot = crate::mesh_animation::Snapshot::from_package(
        &package,
        "animated.glb",
        "LICENSE.txt",
        bad_revision,
    )
    .unwrap();
    crate::mesh_animation_store::save(
        &scene,
        Some(&bad_snapshot),
        Some(&existing_hash),
        bad_revision,
    )
    .unwrap();
    let capable_runtime = temporary.path().join("animation-player.exe");
    fs::write(&capable_runtime, test_runtime_with_animation()).unwrap();
    assert!(export(&scene, checksum, &destination, &capable_runtime).is_err());
    assert!(!destination.exists());
}

#[cfg(not(feature = "mesh-animated-models"))]
#[test]
fn animation_sidecars_fail_closed_without_feature_support() {
    let (temporary, scene, runtime, checksum) = fixture();
    let sidecar = scene.with_file_name("walk.scene.yaml.mesh-animation.json");
    fs::write(&sidecar, b"{}").unwrap();
    let destination = temporary.path().join("animation-bundle");
    let error = export(&scene, checksum, &destination, &runtime).unwrap_err();
    assert!(error.contains("rebuild with mesh-animated-models support"));
    assert!(!destination.exists());

    fs::remove_file(&sidecar).unwrap();
    export(&scene, checksum, &destination, &runtime).unwrap();
    fs::write(
        destination.join("walk.scene.yaml.mesh-animation.json"),
        b"{}",
    )
    .unwrap();
    let error = open_bundle(&destination).unwrap_err();
    assert!(error.contains("rebuild with mesh-animated-models support"));
}

#[cfg(feature = "mesh-animated-models")]
fn test_runtime_with_animation_and_surface() -> Vec<u8> {
    let mut bytes = test_runtime_with_animation();
    bytes.resize(1216, 0);
    bytes[0x46..0x48].copy_from_slice(&7u16.to_le_bytes());
    bytes[0x184..0x188].copy_from_slice(&0x400u32.to_le_bytes());
    bytes[0x238..0x240].copy_from_slice(b".orrsurf");
    bytes[0x248..0x24c].copy_from_slice(&64u32.to_le_bytes());
    bytes[0x24c..0x250].copy_from_slice(&0x3c0u32.to_le_bytes());
    bytes[0x25c..0x260].copy_from_slice(&0x4000_0040u32.to_le_bytes());
    bytes[0x3c0..0x3c0 + b"ORR_MESH_SURFACE_V1".len()].copy_from_slice(b"ORR_MESH_SURFACE_V1");
    bytes[0x400..0x400 + b"ORR_MESH_PROPS_V1".len()].copy_from_slice(b"ORR_MESH_PROPS_V1");
    bytes
}

#[cfg(feature = "mesh-animated-models")]
#[test]
fn surface_marker_composes_with_all_existing_player_sections() {
    super::validate_runtime_with_animation(
        &test_runtime_with_animation_and_surface(),
        true,
        true,
        true,
        true,
        true,
        true,
    )
    .unwrap();
    assert!(super::validate_runtime_with_animation(
        &test_runtime_with_animation(),
        true,
        true,
        true,
        true,
        true,
        true,
    )
    .is_err());
}
