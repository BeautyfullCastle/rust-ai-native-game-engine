use super::{checked_scene_path, prepare, Intent, SIDECAR_SUFFIX};
use crate::game::EditorGame;
use orr_edit::{Op, Origin};
use orr_fp::{fp, FP};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use orr_remote::mesh_navigation::{
    mesh_doc_from_yaml, mesh_from_view, AgentSpec, MeshScenePin, PIN_NAME,
};
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

fn scene_with_pin() -> String {
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
    document.save_yaml()
}

fn sidecar_path(scene: &Path) -> std::path::PathBuf {
    let filename = scene.file_name().unwrap().to_str().unwrap();
    scene.with_file_name(format!("{filename}{SIDECAR_SUFFIX}"))
}

#[test]
fn create_prepares_a_real_local_mesh_host_with_an_empty_admitted_frame() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene = root.join("new-mesh.scene.yaml");

    let editor = prepare(&scene, Intent::Create).unwrap();

    assert_eq!(editor.game(), EditorGame::MeshNavigation);
    assert!(editor.spec().is_local());
    assert_eq!(fs::read(&scene).unwrap(), super::BLANK_SCENE);
    let frame = editor.snapshot().unwrap().predicted();
    assert!(frame.singleton::<MeshScenePin>().is_empty());
}

#[test]
fn open_prepares_a_real_host_and_admits_the_pinned_graph() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene = root.join("pinned-mesh.scene.yaml");
    let original = scene_with_pin();
    fs::write(&scene, &original).unwrap();

    let editor = prepare(&scene, Intent::Open).unwrap();

    assert_eq!(editor.game(), EditorGame::MeshNavigation);
    assert!(editor.spec().is_local());
    let frame = editor.snapshot().unwrap().predicted();
    let view = mesh_from_view(frame).unwrap();
    assert_eq!(view.graph.revision(), view.pin.graph_revision);
    assert_eq!(fs::read_to_string(scene).unwrap(), original);
}

#[test]
fn create_refuses_existing_scene_and_art_sidecar_without_overwriting() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let existing_scene = root.join("existing.scene.yaml");
    let scene_bytes = b"preserve scene";
    fs::write(&existing_scene, scene_bytes).unwrap();
    assert!(prepare(&existing_scene, Intent::Create).is_err());
    assert_eq!(fs::read(&existing_scene).unwrap(), scene_bytes);

    let blocked_scene = root.join("blocked.scene.yaml");
    let sidecar = sidecar_path(&blocked_scene);
    let sidecar_bytes = b"preserve sidecar";
    fs::write(&sidecar, sidecar_bytes).unwrap();
    assert!(prepare(&blocked_scene, Intent::Create).is_err());
    assert!(!blocked_scene.exists());
    assert_eq!(fs::read(sidecar).unwrap(), sidecar_bytes);
}

#[test]
fn open_rejects_invalid_scene_without_changing_source_bytes() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene = root.join("invalid.scene.yaml");
    let original = b"not a valid admitted MeshNavigation scene";
    fs::write(&scene, original).unwrap();

    assert!(prepare(&scene, Intent::Open).is_err());
    assert_eq!(fs::read(&scene).unwrap(), original);
}

#[cfg(windows)]
#[test]
fn canonical_verbatim_disk_paths_are_accepted_but_network_paths_are_rejected() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let root_text = root.to_string_lossy();
    let verbatim_text = if root_text.starts_with(r"\\?\") {
        root_text.into_owned()
    } else {
        format!(r"\\?\{root_text}")
    };
    let verbatim_scene = Path::new(&verbatim_text).join("verbatim.scene.yaml");
    assert_eq!(
        checked_scene_path(&verbatim_scene, false).unwrap(),
        root.join("verbatim.scene.yaml")
    );

    assert!(checked_scene_path(Path::new(r"\\server\share\network.scene.yaml"), true).is_err());
}
