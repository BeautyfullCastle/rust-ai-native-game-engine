use super::*;
use crate::mesh_facing::Facing;

fn facing_runtime() -> Vec<u8> {
    let sections: [(&[u8; 8], &[u8]); 10] = [
        (b".orrmesh", b"ORR_MESH_PLAYER_V1"),
        (b".orrprop", b"ORR_MESH_PROPS_V1"),
        (b".orrrot\0", b"ORR_MESH_ROTATION_V1"),
        (b".orrcam\0", b"ORR_MESH_CAMERA_V1"),
        (b".orrmat\0", b"ORR_MESH_MATERIAL_V1"),
        (b".orranim", b"ORR_MESH_ANIMATION_V1"),
        (b".orrsurf", b"ORR_MESH_SURFACE_V1"),
        (b".orrlit\0", b"ORR_MESH_LIGHTING_V1"),
        (b".orrctrl", b"ORR_MESH_INPUT_V1"),
        (b".orrface", b"ORR_MESH_FACING_V1"),
    ];
    let mut bytes = test_runtime();
    bytes.resize(0x680, 0);
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

fn without_facing_marker(mut bytes: Vec<u8>) -> Vec<u8> {
    let position = bytes
        .windows(b".orrface".len())
        .position(|window| window == b".orrface")
        .unwrap();
    bytes[position] = b'X';
    bytes
}

fn replace_bundle_runtime(destination: &std::path::Path, bytes: &[u8]) {
    fs::write(destination.join(RUNTIME_FILENAME), bytes).unwrap();
    let path = destination.join(MANIFEST_FILENAME);
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["runtime"]["bytes"] = serde_json::json!(bytes.len());
    manifest["runtime"]["sha256"] = serde_json::json!(super::super::digest(bytes));
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
}

#[test]
fn static_route_export_requires_dedicated_runtime_section() {
    let (temporary, scene, runtime, checksum) = fixture();
    let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
    add_props(&scene, revision);
    let (document, digest) = mesh_props::load(&scene, revision).unwrap().unwrap();
    let art = document.props[0]
        .art
        .clone()
        .with_facing(Facing::Route)
        .unwrap();
    mesh_props::save(&scene, None, Some(&digest), revision).unwrap();
    crate::mesh_art::save(&scene, Some(&art), None).unwrap();
    let destination = temporary.path().join("route-bundle");
    let capable = facing_runtime();
    let incapable = without_facing_marker(capable.clone());
    fs::write(&runtime, &incapable).unwrap();
    assert!(export_interactive(&scene, checksum, &destination, &runtime)
        .unwrap_err()
        .contains("route-facing support"));
    assert!(!destination.exists());
    fs::write(&runtime, &capable).unwrap();
    export_interactive(&scene, checksum, &destination, &runtime).unwrap();
    open_bundle(&destination).unwrap();
    let bundle_art = crate::mesh_art::load(&destination.join("walk.scene.yaml"), revision)
        .unwrap()
        .unwrap()
        .0;
    assert_eq!(bundle_art.facing(), Facing::Route);
    replace_bundle_runtime(&destination, &incapable);
    assert!(open_bundle(&destination)
        .unwrap_err()
        .contains("route-facing support"));
}

#[test]
fn explicitly_serialized_fixed_facing_also_requires_new_reader() {
    for prop in [false, true] {
        let (temporary, scene, runtime, checksum) = fixture();
        let revision = graph_revision(&fs::read_to_string(&scene).unwrap());
        add_props(&scene, revision);
        let (document, digest) = mesh_props::load(&scene, revision).unwrap().unwrap();
        if prop {
            let path = mesh_props::sidecar_path(&scene).unwrap();
            let mut value: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            value["props"][0]["art"]["facing"] = serde_json::json!("fixed");
            fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
        } else {
            let filename = scene.file_name().unwrap().to_str().unwrap();
            let mut value: serde_json::Value =
                serde_json::from_slice(&document.props[0].art.encode_embedded(filename).unwrap())
                    .unwrap();
            value["facing"] = serde_json::json!("fixed");
            mesh_props::save(&scene, None, Some(&digest), revision).unwrap();
            fs::write(
                crate::mesh_art::sidecar_path(&scene).unwrap(),
                serde_json::to_vec(&value).unwrap(),
            )
            .unwrap();
        }
        let destination = temporary.path().join("fixed-explicit-bundle");
        fs::write(&runtime, without_facing_marker(facing_runtime())).unwrap();
        assert!(export(&scene, checksum, &destination, &runtime)
            .unwrap_err()
            .contains("route-facing support"));
        assert!(!destination.exists());
        fs::write(&runtime, facing_runtime()).unwrap();
        export(&scene, checksum, &destination, &runtime).unwrap();
        open_bundle(&destination).unwrap();
    }
}

#[cfg(feature = "mesh-animated-models")]
#[test]
fn animated_route_export_checks_marker_and_preserves_setting() {
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
    let snapshot = snapshot
        .clone()
        .with_settings(crate::mesh_animation::Settings {
            facing: Facing::Route,
            ..snapshot.settings()
        })
        .unwrap();
    crate::mesh_animation_store::save(&scene, Some(&snapshot), None, revision).unwrap();
    let destination = temporary.path().join("animated-route-bundle");
    fs::write(&runtime, without_facing_marker(facing_runtime())).unwrap();
    assert!(export_interactive(&scene, checksum, &destination, &runtime)
        .unwrap_err()
        .contains("route-facing support"));
    assert!(!destination.exists());
    fs::write(&runtime, facing_runtime()).unwrap();
    export_interactive(&scene, checksum, &destination, &runtime).unwrap();
    open_bundle(&destination).unwrap();
    let reopened =
        crate::mesh_animation_store::load(&destination.join("walk.scene.yaml"), revision)
            .unwrap()
            .unwrap()
            .0;
    assert_eq!(reopened.settings().facing, Facing::Route);
    replace_bundle_runtime(&destination, &without_facing_marker(facing_runtime()));
    assert!(open_bundle(&destination)
        .unwrap_err()
        .contains("route-facing support"));
}

#[test]
fn facing_marker_is_checked_in_its_section_not_anywhere_in_executable() {
    let valid = facing_runtime();
    super::super::validate_runtime_facing(&valid, true).unwrap();
    let mut misplaced = without_facing_marker(valid);
    misplaced.extend_from_slice(b"ORR_MESH_FACING_V1");
    assert!(super::super::validate_runtime_facing(&misplaced, true).is_err());
    super::super::validate_runtime_facing(&misplaced, false).unwrap();
}
