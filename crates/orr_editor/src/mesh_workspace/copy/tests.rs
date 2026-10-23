use super::{checked_destination, Plan, SIDECAR_SUFFIXES};
use orr_edit::{Op, Origin};
use orr_fp::{fp, FP};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use orr_remote::mesh_navigation::{mesh_doc_from_yaml, AgentSpec, MeshScenePin, PIN_NAME};
use std::{fs, path::Path};

const EMPTY_SCENE: &str = "schema: orr.scene/1\nsingletons: {}\nentities: {}\n";

fn graph() -> StaticMeshGraph {
    StaticMeshGraph::bake(
        &[
            [FP::ZERO, FP::ZERO, FP::ZERO],
            [fp!(2), FP::ZERO, FP::ZERO],
            [FP::ZERO, FP::ZERO, fp!(2)],
            [fp!(2), FP::ZERO, fp!(2)],
        ],
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile::default(),
    )
    .unwrap()
}

fn scene_with_pin() -> Vec<u8> {
    let mut document = mesh_doc_from_yaml(EMPTY_SCENE).unwrap();
    let spec = AgentSpec {
        start: [fp!(0.25), FP::ZERO, fp!(0.25)],
        goal: [fp!(1.75), FP::ZERO, fp!(1.75)],
        max_vertical: FP::ZERO,
        distance_per_tick: fp!(0.25),
    };
    let pin = MeshScenePin::new(&graph(), spec).unwrap();
    let value = document
        .types()
        .get(PIN_NAME)
        .unwrap()
        .read(orr_reflect::bytemuck::bytes_of(&pin));
    document
        .apply(
            Op::SetSingletonField {
                singleton: PIN_NAME.into(),
                path: String::new(),
                value,
            },
            Origin::User,
        )
        .unwrap();
    document.save_yaml().into_bytes()
}

fn put_scene(root: &Path) -> (std::path::PathBuf, Vec<u8>) {
    let scene = root.join("copied.scene.yaml");
    let bytes = scene_with_pin();
    fs::write(&scene, &bytes).unwrap();
    (scene, bytes)
}

#[test]
fn copy_preserves_exact_scene_bytes_and_contains_no_unrelated_files() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let (scene, original) = put_scene(&root);
    fs::write(root.join("unrelated.txt"), b"not copied").unwrap();
    let destination = root.join("copy");

    let copied_scene = Plan::prepare(&scene, &destination)
        .unwrap()
        .publish()
        .unwrap();

    assert_eq!(copied_scene, destination.join("copied.scene.yaml"));
    assert_eq!(fs::read(&scene).unwrap(), original);
    assert_eq!(fs::read(&copied_scene).unwrap(), original);
    let entries = fs::read_dir(destination)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].file_name().to_string_lossy(),
        "copied.scene.yaml"
    );
    assert_eq!(fs::read(root.join("unrelated.txt")).unwrap(), b"not copied");
}

#[test]
fn copy_preserves_and_validates_embedded_camera_bytes() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let (scene, scene_bytes) = put_scene(&root);
    let graph_revision = graph().revision();
    let camera = crate::mesh_camera::Document::from_orbit(orr_render::camera3d::OrbitCamera::new(
        [0.0, 0.0, 0.0],
        0.0,
        0.0,
        6.0,
    ))
    .unwrap();
    let camera_bytes = camera.encode("copied.scene.yaml", graph_revision).unwrap();
    let camera_path = scene.with_file_name("copied.scene.yaml.mesh-camera.json");
    fs::write(&camera_path, &camera_bytes).unwrap();
    let destination = root.join("camera-copy");

    let copied_scene = Plan::prepare(&scene, &destination)
        .unwrap()
        .publish()
        .unwrap();

    assert_eq!(fs::read(&copied_scene).unwrap(), scene_bytes);
    assert_eq!(
        fs::read(destination.join("copied.scene.yaml.mesh-camera.json")).unwrap(),
        camera_bytes
    );
    let entries = fs::read_dir(destination)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(entries.len(), 2);
}

#[test]
fn copy_preserves_valid_scene_bound_lighting_without_rebinding_bytes() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let (scene, scene_bytes) = put_scene(&root);
    let document =
        crate::mesh_lighting::Document::from_lighting(orr_render::Lighting::default()).unwrap();
    let lighting_bytes = document
        .encode("copied.scene.yaml", graph().revision())
        .unwrap();
    let lighting_path = scene.with_file_name("copied.scene.yaml.mesh-lighting.json");
    fs::write(&lighting_path, &lighting_bytes).unwrap();
    let destination = root.join("lighting-copy");

    let copied_scene = Plan::prepare(&scene, &destination)
        .unwrap()
        .publish()
        .unwrap();

    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    assert_eq!(fs::read(lighting_path).unwrap(), lighting_bytes);
    assert_eq!(fs::read(&copied_scene).unwrap(), scene_bytes);
    assert_eq!(
        fs::read(destination.join("copied.scene.yaml.mesh-lighting.json")).unwrap(),
        lighting_bytes
    );
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 2);
    let loaded = crate::mesh_lighting_store::load(&copied_scene, graph().revision())
        .unwrap()
        .unwrap()
        .0;
    assert_eq!(loaded, document);
}

#[test]
fn stale_lighting_refuses_copy_without_creating_a_destination() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let (scene, scene_bytes) = put_scene(&root);
    let document =
        crate::mesh_lighting::Document::from_lighting(orr_render::Lighting::default()).unwrap();
    let lighting_bytes = document.encode("copied.scene.yaml", [255; 32]).unwrap();
    let lighting_path = scene.with_file_name("copied.scene.yaml.mesh-lighting.json");
    fs::write(&lighting_path, &lighting_bytes).unwrap();
    let destination = root.join("stale-lighting-copy");

    assert!(Plan::prepare(&scene, &destination).is_err());
    assert!(!destination.exists());
    assert_eq!(fs::read(scene).unwrap(), scene_bytes);
    assert_eq!(fs::read(lighting_path).unwrap(), lighting_bytes);
}

#[test]
fn malformed_or_orphan_sidecars_are_refused_without_publication() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let (scene, _) = put_scene(&root);

    for suffix in SIDECAR_SUFFIXES {
        let sidecar = scene.with_file_name(format!(
            "{}{}",
            scene.file_name().unwrap().to_string_lossy(),
            suffix
        ));
        fs::write(&sidecar, b"invalid sidecar").unwrap();
        let destination = root.join(format!("copy-{}", suffix.trim_start_matches('.')));
        assert!(Plan::prepare(&scene, &destination).is_err(), "{suffix}");
        assert!(!destination.exists());
        fs::remove_file(sidecar).unwrap();
    }

    let orphan_scene = root.join("blank.scene.yaml");
    fs::write(&orphan_scene, EMPTY_SCENE).unwrap();
    fs::write(
        orphan_scene.with_file_name(format!("blank.scene.yaml{}", SIDECAR_SUFFIXES[0])),
        b"orphan",
    )
    .unwrap();
    assert!(Plan::prepare(&orphan_scene, &root.join("orphan-copy")).is_err());
    assert!(!root.join("orphan-copy").exists());
}

#[cfg(not(feature = "mesh-animated-models"))]
#[test]
fn workspace_copy_refuses_animation_presence_without_the_optional_feature() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let (scene, scene_bytes) = put_scene(&root);
    let sidecar = scene.with_file_name("copied.scene.yaml.mesh-animation.json");
    let animation_bytes = b"animation cannot be admitted without feature support";
    fs::write(&sidecar, animation_bytes).unwrap();
    let destination = root.join("unsupported-animation-copy");

    let error = Plan::prepare(&scene, &destination).err().unwrap();

    assert!(error.contains("requires the mesh-animated-models feature"));
    assert!(!destination.exists());
    assert_eq!(fs::read(scene).unwrap(), scene_bytes);
    assert_eq!(fs::read(sidecar).unwrap(), animation_bytes);
}

#[test]
fn existing_destination_is_never_replaced_even_when_empty() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let (scene, original) = put_scene(&root);
    let destination = root.join("existing-copy");
    fs::create_dir(&destination).unwrap();
    let sentinel = destination.join("winner.txt");
    fs::write(&sentinel, b"preserve winner").unwrap();

    assert!(Plan::prepare(&scene, &destination).is_err());
    assert_eq!(fs::read(&scene).unwrap(), original);
    assert_eq!(fs::read(&sentinel).unwrap(), b"preserve winner");
}

#[test]
fn destination_created_after_prepare_is_preserved() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let (scene, original) = put_scene(&root);
    let destination = root.join("racing-copy");
    let plan = Plan::prepare(&scene, &destination).unwrap();
    fs::create_dir(&destination).unwrap();
    let sentinel = destination.join("winner.txt");
    fs::write(&sentinel, b"preserve concurrent winner").unwrap();

    assert!(plan.publish().is_err());
    assert_eq!(fs::read(&scene).unwrap(), original);
    assert_eq!(fs::read(&sentinel).unwrap(), b"preserve concurrent winner");
}

#[test]
fn changed_source_after_prepare_discards_the_staged_copy() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let (scene, _) = put_scene(&root);
    let destination = root.join("changed-copy");
    let plan = Plan::prepare(&scene, &destination).unwrap();
    fs::write(&scene, b"changed after preparation").unwrap();

    assert!(plan.publish().is_err());
    assert!(!destination.exists());
}

#[test]
fn destination_accepts_unicode_spaces_and_rejects_device_names() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let accepted = root.join("Mesh copy Ω ! &");
    assert!(checked_destination(&accepted).is_ok());
    for name in ["CON", "LPT1", "COM¹", "trailing.", "trailing "] {
        assert!(checked_destination(&root.join(name)).is_err(), "{name}");
    }
}

#[test]
fn sidecar_presence_change_after_prepare_discards_the_staged_copy() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let (scene, _) = put_scene(&root);
    let destination = root.join("sidecar-race-copy");
    let plan = Plan::prepare(&scene, &destination).unwrap();
    fs::write(
        scene.with_file_name("copied.scene.yaml.mesh-camera.json"),
        b"new sidecar",
    )
    .unwrap();

    assert!(plan.publish().is_err());
    assert!(!destination.exists());
}

#[test]
fn staged_file_tampering_and_extra_entries_refuse_publication() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let (scene, original) = put_scene(&root);

    let destination = root.join("tampered-copy");
    let plan = Plan::prepare(&scene, &destination).unwrap();
    fs::write(
        plan.stage.path().join("copied.scene.yaml"),
        b"tampered stage",
    )
    .unwrap();
    assert!(plan.publish().is_err());
    assert!(!destination.exists());
    assert_eq!(fs::read(&scene).unwrap(), original);

    let destination = root.join("extra-copy");
    let plan = Plan::prepare(&scene, &destination).unwrap();
    fs::write(plan.stage.path().join("unexpected.txt"), b"extra").unwrap();
    assert!(plan.publish().is_err());
    assert!(!destination.exists());
    assert_eq!(fs::read(&scene).unwrap(), original);
}

#[test]
fn copy_preserves_hud_and_rejects_invalid_or_changed_hud_before_publication() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let (scene, scene_bytes) = put_scene(&root);
    let revision = graph().revision();
    let document = crate::mesh_hud::Document {
        title: "Harbor route".into(),
        ..Default::default()
    };
    let bytes = document.encode("copied.scene.yaml", revision).unwrap();
    let sidecar = crate::mesh_hud_store::sidecar_path(&scene).unwrap();
    fs::write(&sidecar, &bytes).unwrap();
    let destination = root.join("hud-copy");
    let copied = Plan::prepare(&scene, &destination)
        .unwrap()
        .publish()
        .unwrap();
    assert_eq!(fs::read(&copied).unwrap(), scene_bytes);
    assert_eq!(
        fs::read(destination.join("copied.scene.yaml.mesh-hud.json")).unwrap(),
        bytes
    );
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 2);

    let stale_destination = root.join("hud-stale");
    let stale = Plan::prepare(&scene, &stale_destination).unwrap();
    fs::write(&sidecar, b"invalid replacement").unwrap();
    assert!(stale.publish().is_err());
    assert!(!stale_destination.exists());
    let invalid_destination = root.join("hud-invalid");
    assert!(Plan::prepare(&scene, &invalid_destination).is_err());
    assert!(!invalid_destination.exists());
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
}
