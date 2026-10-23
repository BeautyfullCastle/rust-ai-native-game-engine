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

fn cooked_package(root: &Path, model: &orr_model::StaticModel) -> PathBuf {
    let package_root = root.join(format!(
        "cooked-prop-package-{}",
        model.source().primitives.len()
    ));
    fs::create_dir(&package_root).unwrap();
    fs::write(
        package_root.join("model.orrmodel"),
        model.to_bytes().unwrap(),
    )
    .unwrap();
    fs::write(package_root.join("LICENSE.txt"), LICENSE).unwrap();
    let runtime = orr_package::Runtime::content_only();
    let manifest = Manifest {
        schema: 1,
        name: "cooked-props".into(),
        version: "1.0.0".into(),
        engine: format!("={}", runtime.engine_version),
        capabilities: BTreeSet::from(["models".into()]),
        dependencies: BTreeMap::new(),
        files: BTreeSet::from(["LICENSE.txt".into(), "model.orrmodel".into()]),
    };
    fs::write(
        package_root.join("orr.package.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    package_root
}

fn snapshot_with_primitive_count(root: &Path, count: usize) -> Snapshot {
    let asset_id = "model.orrmodel";
    let transform = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let source = orr_model::ModelSource {
        format: "orr_static_model".into(),
        version: 1,
        asset_id: asset_id.into(),
        dependencies: vec![orr_model::Dependency {
            uri: "$source".into(),
            sha256: "0".repeat(64),
        }],
        primitives: (0..count)
            .map(|index| orr_model::Primitive {
                id: format!("{asset_id}#node=0/mesh=0/primitive={index}"),
                vertices: vec![
                    orr_model::Vertex {
                        position: [0.0, 0.0, 0.0],
                        normal: [0.0, 1.0, 0.0],
                        uv: [0.0, 0.0],
                    },
                    orr_model::Vertex {
                        position: [1.0, 0.0, 0.0],
                        normal: [0.0, 1.0, 0.0],
                        uv: [1.0, 0.0],
                    },
                    orr_model::Vertex {
                        position: [0.0, 0.0, 1.0],
                        normal: [0.0, 1.0, 0.0],
                        uv: [0.0, 1.0],
                    },
                ],
                indices: vec![0, 1, 2],
                material: 0,
                transform,
            })
            .collect(),
        materials: vec![orr_model::Material {
            base_color: [1.0; 4],
            image: 0,
            linear_filter: false,
            wrap_s: orr_model::Wrap::Clamp,
            wrap_t: orr_model::Wrap::Clamp,
        }],
        images: vec![orr_model::Image {
            width: 1,
            height: 1,
            rgba8: vec![255; 4],
        }],
    };
    let model = orr_model::StaticModel::new(source).unwrap();
    let package_root = cooked_package(root, &model);
    Snapshot::from_package(&package_root, "model.orrmodel", "LICENSE.txt", REVISION).unwrap()
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
fn maximum_props_reopen_as_one_interned_asset_with_reserved_draw_budget() {
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
                art: art
                    .clone()
                    .with_transform([index as f32, 0.0, 0.0], 1.0)
                    .unwrap(),
            })
            .collect(),
    };
    assert_eq!(document.props.len(), 128);
    let primitive_count = art.source_model_arc().source().primitives.len();
    assert!(primitive_count * document.props.len() <= 3584);
    let original_model_bytes = art.source_model_arc().to_bytes().unwrap();
    let original_art: serde_json::Value =
        serde_json::from_slice(&art.encode_embedded("mesh.scene.yaml").unwrap()).unwrap();
    save(&scene_path, Some(&document), None, REVISION).unwrap();
    fs::remove_dir_all(source).unwrap();
    let (loaded, _) = load(&scene_path, REVISION).unwrap().unwrap();
    let placements = loaded.placements().unwrap();
    assert_eq!(placements.len(), 128);
    for (index, placement) in placements.iter().enumerate() {
        assert!(
            std::sync::Arc::ptr_eq(&placements[0].model, &placement.model),
            "same-appearance prop model Arc is interned after reload"
        );
        assert_eq!(loaded.props[index].id, index as u64 + 1);
        assert_eq!(loaded.props[index].name, format!("Prop {index}"));
        assert_eq!(placement.instance.translation, [index as f32, 0.0, 0.0]);
        assert_eq!(
            placement.model.to_bytes().unwrap(),
            original_model_bytes,
            "interning preserves immutable source model bytes"
        );
    }
    let first_loaded_art: serde_json::Value = serde_json::from_slice(
        &loaded.props[0]
            .art
            .encode_embedded("mesh.scene.yaml")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(first_loaded_art, original_art);
}

#[test]
fn rejects_129_props_before_sidecar_creation() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene_path = scene(&root);
    let art = snapshot(&root);
    let document = Document {
        props: (0..=super::MAX_PROPS)
            .map(|index| Prop {
                id: index as u64 + 1,
                name: format!("Prop {index}"),
                art: art.clone(),
            })
            .collect(),
    };
    assert!(document.validate("mesh.scene.yaml", REVISION).is_err());
    assert!(document.placements().is_err());
    assert!(save(&scene_path, Some(&document), None, REVISION).is_err());
    assert!(!sidecar_path(&scene_path).unwrap().exists());
}

#[test]
fn rejects_seven_distinct_appearance_identities() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let art = snapshot(&root);
    let appearances = (0..7)
        .map(|index| {
            art.clone()
                .with_tint([0.2 + index as f32 * 0.1, 1.0, 1.0, 1.0])
                .unwrap()
        })
        .collect::<Vec<_>>();
    let six_appearances = Document {
        props: appearances[..6]
            .iter()
            .enumerate()
            .map(|(index, art)| Prop {
                id: index as u64 + 1,
                name: format!("Appearance {index}"),
                art: art.clone(),
            })
            .collect(),
    };
    assert!(six_appearances
        .validate("mesh.scene.yaml", REVISION)
        .is_ok());
    let six_placements = six_appearances.placements().unwrap();
    for (index, placement) in six_placements.iter().enumerate() {
        for other in &six_placements[index + 1..] {
            assert!(!std::sync::Arc::ptr_eq(&placement.model, &other.model));
        }
    }
    let seven_appearances = Document {
        props: appearances
            .iter()
            .enumerate()
            .map(|(index, art)| Prop {
                id: index as u64 + 1,
                name: format!("Appearance {index}"),
                art: art.clone(),
            })
            .collect(),
    };
    assert!(seven_appearances
        .validate("mesh.scene.yaml", REVISION)
        .is_err());
    assert!(seven_appearances.placements().is_err());
}

#[test]
fn render_budget_accepts_the_limit_and_rejects_one_extra_draw() {
    assert!(super::validate_render_budget(6, 3584).is_ok());
    assert!(super::validate_render_budget(6, 3585).is_err());
    assert!(super::validate_render_budget(7, 3584).is_err());

    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let scene_path = scene(&root);
    let accepted_art = snapshot_with_primitive_count(&root, 28);
    let accepted = Document {
        props: (0..128)
            .map(|index| Prop {
                id: index + 1,
                name: format!("Draw {index}"),
                art: accepted_art.clone(),
            })
            .collect(),
    };
    assert_eq!(
        accepted_art.source_model_arc().source().primitives.len(),
        28
    );
    assert!(accepted.validate("mesh.scene.yaml", REVISION).is_ok());
    assert_eq!(accepted.placements().unwrap().len(), 128);

    let over_budget_art = snapshot_with_primitive_count(&root, 29);
    let over_budget = Document {
        props: (0..128)
            .map(|index| Prop {
                id: index + 1,
                name: format!("Draw {index}"),
                art: over_budget_art.clone(),
            })
            .collect(),
    };
    assert_eq!(
        over_budget_art.source_model_arc().source().primitives.len(),
        29
    );
    assert!(over_budget.validate("mesh.scene.yaml", REVISION).is_err());
    assert!(over_budget.placements().is_err());
    assert!(save(&scene_path, Some(&over_budget), None, REVISION).is_err());
    assert!(!sidecar_path(&scene_path).unwrap().exists());
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
