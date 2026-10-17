use super::{load, save, sidecar_path, sidecar_stamp, Document, Prop};
use crate::mesh_art::Snapshot;
use orr_package::Manifest;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const STATIC_GLB: &[u8] = include_bytes!("../../../orr_model/tests/fixtures/model.glb");
const LICENSE: &[u8] = b"Mesh props fixture license\n";
const REVISION: [u8; 32] = [0x5a; 32];

fn write_package(root: &Path) {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join("model.glb"), STATIC_GLB).unwrap();
    fs::write(root.join("LICENSE.txt"), LICENSE).unwrap();
    let runtime = orr_package::Runtime::content_only();
    let manifest = Manifest {
        schema: 1,
        name: "mesh-props-fixture".into(),
        version: "1.0.0".into(),
        engine: format!("={}", runtime.engine_version),
        capabilities: BTreeSet::from(["models".into()]),
        dependencies: BTreeMap::new(),
        files: BTreeSet::from(["LICENSE.txt".into(), "model.glb".into()]),
    };
    fs::write(
        root.join("orr.package.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
}

fn snapshot(root: &Path) -> Snapshot {
    let package = tempfile::tempdir_in(root).unwrap();
    write_package(package.path());
    Snapshot::from_package(package.path(), "model.glb", "LICENSE.txt", REVISION).unwrap()
}

fn package(parent: &Path) -> PathBuf {
    let root = parent.join("source-package");
    write_package(&root);
    root
}

fn scene(root: &Path) -> PathBuf {
    let path = root.join("mesh.scene.yaml");
    fs::write(&path, b"mesh scene\n").unwrap();
    path
}

fn document(root: &Path) -> Document {
    Document {
        props: vec![
            Prop {
                id: 7,
                name: "Bench".into(),
                art: snapshot(root),
            },
            Prop {
                id: 19,
                name: "Lamp".into(),
                art: snapshot(root),
            },
        ],
    }
}

#[test]
fn roundtrips_embedded_props_after_package_source_is_removed() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let source = package(&root);
    let scene_path = scene(&root);
    let document = Document {
        props: vec![Prop {
            id: 7,
            name: "Bench".into(),
            art: Snapshot::from_package(&source, "model.glb", "LICENSE.txt", REVISION).unwrap(),
        }],
    };
    let saved = save(&scene_path, Some(&document), None, REVISION)
        .unwrap()
        .unwrap();
    assert_eq!(
        sidecar_stamp(&scene_path).unwrap().as_deref(),
        Some(saved.as_str())
    );
    assert_eq!(
        sidecar_path(&scene_path).unwrap(),
        root.join("mesh.scene.yaml.mesh-props.json")
    );
    fs::remove_dir_all(source).unwrap();
    let (loaded, stamp) = load(&scene_path, REVISION).unwrap().unwrap();
    assert_eq!(stamp, saved);
    assert_eq!(loaded.props.len(), 1);
    assert_eq!(loaded.props[0].id, 7);
    assert_eq!(loaded.props[0].name, "Bench");
    assert_eq!(loaded.props[0].art.graph_revision(), REVISION);
    assert_eq!(loaded.placements().unwrap().len(), 1);
}

#[test]
fn maximum_props_reopen_as_six_assets_with_two_viewport_slots_reserved() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let source = package(&root);
    let scene_path = scene(&root);
    let art = Snapshot::from_package(&source, "model.glb", "LICENSE.txt", REVISION).unwrap();
    let document = Document {
        props: (0..super::MAX_PROPS)
            .map(|index| Prop {
                id: index as u64 + 1,
                name: format!("Prop {index}"),
                art: art.clone(),
            })
            .collect(),
    };
    save(&scene_path, Some(&document), None, REVISION).unwrap();
    fs::remove_dir_all(source).unwrap();
    let (loaded, _) = load(&scene_path, REVISION).unwrap().unwrap();
    let placements = loaded.placements().unwrap();
    assert_eq!(placements.len(), 6);
    assert_eq!(placements.len() + 2, 8);
    for (index, placement) in placements.iter().enumerate() {
        for other in &placements[index + 1..] {
            assert!(!std::sync::Arc::ptr_eq(&placement.model, &other.model));
        }
    }
}

#[test]
fn optimistic_save_and_remove_reject_stale_digest() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene_path = scene(&root);
    let document = document(&root);
    let first = save(&scene_path, Some(&document), None, REVISION)
        .unwrap()
        .unwrap();
    assert!(save(&scene_path, Some(&document), None, REVISION).is_err());
    assert!(save(
        &scene_path,
        Some(&document),
        Some(&"0".repeat(64)),
        REVISION
    )
    .is_err());
    assert!(save(&scene_path, None, Some(&"0".repeat(64)), REVISION).is_err());
    assert!(sidecar_path(&scene_path).unwrap().exists());
    assert_eq!(
        save(&scene_path, None, Some(&first), REVISION).unwrap(),
        None
    );
    assert!(!sidecar_path(&scene_path).unwrap().exists());
}

#[test]
fn validates_prop_ids_names_count_and_graph_revision() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let mut document = document(&root);
    assert!(document.validate("mesh.scene.yaml", REVISION).is_ok());
    document.props[1].id = document.props[0].id;
    assert!(document.validate("mesh.scene.yaml", REVISION).is_err());
    document.props[1].id = 0;
    assert!(document.validate("mesh.scene.yaml", REVISION).is_err());
    document.props[1].id = 19;
    document.props[1].name = "  \n".into();
    assert!(document.validate("mesh.scene.yaml", REVISION).is_err());
    document.props[1].name = document.props[0].name.clone();
    assert!(document.validate("mesh.scene.yaml", REVISION).is_err());
    assert!(document.placements().is_err());
    document.props[1].name = " Lamp ".into();
    assert!(document.validate("mesh.scene.yaml", REVISION).is_err());
    document.props[1].name = "Lamp".into();
    let other_package = tempfile::tempdir_in(&root).unwrap();
    write_package(other_package.path());
    document.props[1].art =
        Snapshot::from_package(other_package.path(), "model.glb", "LICENSE.txt", [0x33; 32])
            .unwrap();
    assert!(document.validate("mesh.scene.yaml", REVISION).is_err());
    document.props[1].art = snapshot(&root);
    let first_prop = document.props[0].clone();
    document
        .props
        .resize_with(super::MAX_PROPS + 1, || first_prop.clone());
    assert!(document.validate("mesh.scene.yaml", REVISION).is_err());
}

#[test]
fn rejects_corrupt_or_wrong_revision_sidecars() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene_path = scene(&root);
    let document = document(&root);
    save(&scene_path, Some(&document), None, REVISION).unwrap();
    assert!(load(&scene_path, [0x33; 32]).is_err());
    let sidecar = sidecar_path(&scene_path).unwrap();
    let mut bytes = fs::read(&sidecar).unwrap();
    bytes.push(b'!');
    fs::write(&sidecar, bytes).unwrap();
    assert!(load(&scene_path, REVISION).is_err());
}

#[test]
fn refuses_existing_destination_and_non_scene_paths() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene_path = scene(&root);
    let document = document(&root);
    save(&scene_path, Some(&document), None, REVISION).unwrap();
    assert!(save(&scene_path, Some(&document), None, REVISION).is_err());
    assert!(sidecar_path(&root.join("missing.scene.yaml")).is_err());
    assert!(sidecar_path(&root.join("..\\escape.scene.yaml")).is_err());
}
