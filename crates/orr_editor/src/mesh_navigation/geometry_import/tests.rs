use super::*;
use crate::{
    editor::Editor,
    game::EditorGame,
    mesh_navigation::{self, geometry_panel::Action},
};
use orr_model::{Dependency, Image, Material, ModelSource, Primitive, StaticModel, Vertex, Wrap};
use orr_navigation::AgentProfile;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

fn flat_model(transform: [[f32; 4]; 4]) -> StaticModel {
    let asset_id = "floor.orrmodel";
    StaticModel::new(ModelSource {
        format: "orr_static_model".into(),
        version: 1,
        asset_id: asset_id.into(),
        dependencies: vec![Dependency {
            uri: "$source".into(),
            sha256: "0".repeat(64),
        }],
        primitives: vec![Primitive {
            id: format!("{asset_id}#node=0/mesh=0/primitive=0"),
            vertices: vec![
                vertex([-1.0, 0.0, -1.0]),
                vertex([-1.0, 0.0, 1.0]),
                vertex([1.0, 0.0, 1.0]),
                vertex([1.0, 0.0, -1.0]),
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            material: 0,
            transform,
        }],
        materials: vec![Material {
            base_color: [1.0; 4],
            image: 0,
            linear_filter: true,
            wrap_s: Wrap::Clamp,
            wrap_t: Wrap::Clamp,
        }],
        images: vec![Image {
            width: 1,
            height: 1,
            rgba8: vec![255; 4],
        }],
    })
    .unwrap()
}

fn vertex(position: [f32; 3]) -> Vertex {
    Vertex {
        position,
        normal: [0.0, 1.0, 0.0],
        uv: [0.0; 2],
    }
}

fn placement(model: StaticModel) -> SceneModel {
    SceneModel {
        model: Arc::new(model),
        instance: StaticInstance::default(),
    }
}

fn package(root: &Path, model: &StaticModel) -> std::path::PathBuf {
    let package_root = root.join("walkable-floor");
    fs::create_dir(&package_root).unwrap();
    let model_bytes = model.to_bytes().unwrap();
    let license_bytes = b"Example license text\n";
    fs::write(package_root.join("floor.orrmodel"), &model_bytes).unwrap();
    fs::write(package_root.join("LICENSE.txt"), license_bytes).unwrap();
    let runtime = orr_package::Runtime::content_only();
    let manifest = orr_package::Manifest {
        schema: 1,
        name: "walkable-floor".into(),
        version: "1.0.0".into(),
        engine: format!("={}", runtime.engine_version),
        capabilities: BTreeSet::from(["models".into()]),
        dependencies: BTreeMap::new(),
        files: BTreeSet::from(["LICENSE.txt".into(), "floor.orrmodel".into()]),
    };
    fs::write(
        package_root.join("orr.package.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    package_root
}

fn revision() -> [u8; 32] {
    [0x4a; 32]
}

fn blank_mesh_scene(path: &Path) {
    fs::write(path, "schema: orr.scene/1\nsingletons: {}\nentities: {}\n").unwrap();
}

fn imported_graph(root: &Path, profile: AgentProfile) -> StaticMeshGraph {
    let package_root = package(root, &flat_model(orr_model::IDENTITY));
    let snapshot =
        Snapshot::from_package(&package_root, "floor.orrmodel", "LICENSE.txt", revision()).unwrap();
    candidate(&snapshot, true, profile).unwrap()
}

#[test]
fn imported_surface_stages_local_preview_undo_redo_without_host_edits() {
    let temporary = tempfile::tempdir().unwrap();
    let scene_path = temporary.path().join("imported-surface.scene.yaml");
    blank_mesh_scene(&scene_path);
    let mut editor = Editor::open_game(&scene_path, EditorGame::MeshNavigation).unwrap();
    editor.sync();
    let mut navigation = mesh_navigation::Panel::default();
    navigation.sync(&editor);
    let graph = imported_graph(temporary.path(), navigation.geometry_profile().unwrap());
    let expected_revision = graph.revision();
    let original_checksum = editor.checksum();
    let original_history_len = editor.history().entries.len();

    navigation
        .stage_geometry(Action::ReplaceImportedSurface(graph), &editor)
        .unwrap();
    assert_eq!(editor.checksum(), original_checksum);
    assert_eq!(editor.history().entries.len(), original_history_len);
    assert_eq!(
        navigation.graph.as_ref().unwrap().revision(),
        expected_revision
    );
    assert!(navigation.geometry_staged);
    assert!(navigation.geometry_preview_active());

    navigation.change_geometry_history(&editor, false).unwrap();
    assert!(!navigation.geometry_staged);
    assert!(!navigation.geometry_preview_active());
    assert!(navigation.graph.is_none());
    assert_eq!(editor.checksum(), original_checksum);
    assert_eq!(editor.history().entries.len(), original_history_len);

    navigation.change_geometry_history(&editor, true).unwrap();
    assert!(navigation.geometry_staged);
    assert!(navigation.geometry_preview_active());
    assert_eq!(
        navigation.graph.as_ref().unwrap().revision(),
        expected_revision
    );
    assert_eq!(editor.checksum(), original_checksum);
    assert_eq!(editor.history().entries.len(), original_history_len);
}

#[cfg(feature = "mesh-mission")]
#[test]
fn imported_surface_replacement_is_refused_even_with_mission_reprojection_enabled() {
    use egui_kittest::{kittest::Queryable, Harness};

    struct State {
        editor: Editor,
        navigation: mesh_navigation::Panel,
    }

    let temporary = tempfile::tempdir().unwrap();
    let scene_path = temporary.path().join("mission.scene.yaml");
    let mut editor = crate::mesh_workspace::mission::prepare(
        &scene_path,
        &crate::mesh_workspace::mission::Settings::default(),
    )
    .unwrap();
    editor.sync();
    let mut navigation = mesh_navigation::Panel::default();
    navigation.sync(&editor);
    let graph = imported_graph(temporary.path(), navigation.geometry_profile().unwrap());
    let original_checksum = editor.checksum();
    let original_history_len = editor.history().entries.len();
    let original_revision = navigation.graph.as_ref().unwrap().revision();
    assert!(navigation.mission_bound);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 240.0))
        .build_ui_state(
            |ui, state: &mut State| {
                state.navigation.mission_geometry.show(
                    ui,
                    Err("Enable mission reprojection for this guard test".into()),
                );
            },
            State { editor, navigation },
        );
    harness.run_steps(2);
    harness
        .get_by_label("Preserve and reproject the active mesh mission")
        .click_accesskit();
    harness.run_steps(2);
    assert!(harness.state().navigation.mission_geometry.is_enabled());
    assert!(harness.state().navigation.mission_reprojection_enabled());

    let state = harness.state_mut();
    let error = state
        .navigation
        .stage_geometry(Action::ReplaceImportedSurface(graph), &state.editor)
        .unwrap_err();
    assert!(error.contains("Remove the mesh mission before importing"));
    assert_eq!(state.editor.checksum(), original_checksum);
    assert_eq!(state.editor.history().entries.len(), original_history_len);
    assert_eq!(
        state.navigation.graph.as_ref().unwrap().revision(),
        original_revision
    );
    assert!(!state.navigation.geometry_staged);
    assert!(!state.navigation.geometry_preview_active());
}

#[test]
fn graph_bound_art_sidecar_refuses_imported_surface_without_host_changes() {
    let temporary = tempfile::tempdir().unwrap();
    let scene_path = temporary.path().join("bound-art.scene.yaml");
    blank_mesh_scene(&scene_path);
    let mut editor = Editor::open_game(&scene_path, EditorGame::MeshNavigation).unwrap();
    editor.sync();
    let mut navigation = mesh_navigation::Panel::default();
    navigation.sync(&editor);
    navigation.create_ramp_and_bridge().unwrap();
    navigation.place_agent(&mut editor).unwrap();
    editor.sync();
    navigation.sync(&editor);

    let graph = imported_graph(temporary.path(), navigation.geometry_profile().unwrap());
    let original_checksum = editor.checksum();
    let original_history_len = editor.history().entries.len();
    let original_revision = navigation.graph.as_ref().unwrap().revision();
    let sidecar = PathBuf::from(format!(
        "{}.mesh-art.json",
        editor.path().unwrap().display()
    ));
    let sentinel = b"existing graph-bound art sidecar";
    fs::write(&sidecar, sentinel).unwrap();

    let error = navigation
        .stage_geometry(Action::ReplaceImportedSurface(graph), &editor)
        .unwrap_err();
    assert!(error.contains("existing scene-bound art"));
    assert_eq!(fs::read(&sidecar).unwrap(), sentinel);
    assert_eq!(editor.checksum(), original_checksum);
    assert_eq!(editor.history().entries.len(), original_history_len);
    assert_eq!(
        navigation.graph.as_ref().unwrap().revision(),
        original_revision
    );
    assert!(!navigation.geometry_staged);
    assert!(!navigation.geometry_preview_active());
}

#[test]
fn admitted_package_converts_to_shared_walkable_triangles() {
    let temporary = tempfile::tempdir().unwrap();
    let model = flat_model(orr_model::IDENTITY);
    let package_root = package(temporary.path(), &model);
    let source_path = package_root.join("floor.orrmodel");
    let original_source_bytes = fs::read(&source_path).unwrap();
    let snapshot =
        Snapshot::from_package(&package_root, "floor.orrmodel", "LICENSE.txt", revision()).unwrap();
    let graph = candidate(&snapshot, true, AgentProfile::default()).unwrap();
    assert_eq!(graph.triangles().len(), 2);
    assert_eq!(graph.vertices().len(), 4);
    assert!(graph
        .triangles()
        .iter()
        .all(|triangle| triangle.portals.len() == 1));
    assert!(graph.to_bytes().len() <= MAX_GRAPH_BYTES);
    assert_eq!(fs::read(source_path).unwrap(), original_source_bytes);
}

#[test]
fn transformed_instances_preserve_shared_edges_and_bounded_coordinates() {
    let model = flat_model(orr_model::IDENTITY);
    let mut placed = placement(model);
    placed.instance.translation = [4.0, 1.0, -3.0];
    placed.instance.scale = [2.0, 1.0, 0.5];
    let graph = candidate_model(&placed, true, AgentProfile::default()).unwrap();
    assert_eq!(graph.triangles().len(), 2);
    assert_eq!(graph.vertices().len(), 4);
    assert_eq!(graph.vertices()[0][1], FP::ONE);
    assert!(graph.vertices().iter().all(|point| {
        point[0] >= FP::from_raw(FP::ONE.raw() * 2)
            && point[0] <= FP::from_raw(FP::ONE.raw() * 6)
            && point[2] >= FP::from_raw(FP::ONE.raw() * -4)
            && point[2] <= FP::from_raw(FP::ONE.raw() * -2)
    }));
}

#[test]
fn mirrored_node_transform_is_reoriented_without_losing_faces() {
    let mut mirrored = orr_model::IDENTITY;
    mirrored[0][0] = -1.0;
    let graph = candidate_model(
        &placement(flat_model(mirrored)),
        true,
        AgentProfile::default(),
    )
    .unwrap();
    assert_eq!(graph.triangles().len(), 2);
    assert_eq!(graph.vertices().len(), 4);
}

#[test]
fn rejects_missing_license_acknowledgement_and_nonwalkable_geometry() {
    let flat = placement(flat_model(orr_model::IDENTITY));
    assert!(candidate_model(&flat, false, AgentProfile::default())
        .unwrap_err()
        .contains("license acknowledgement"));

    let steep_vertices = [
        [-1.0, 0.0, -1.0],
        [-1.0, 0.0, 1.0],
        [1.0, 4.0, 1.0],
        [1.0, 4.0, -1.0],
    ];
    let mut source = flat_model(orr_model::IDENTITY).source().clone();
    for (vertex, position) in source.primitives[0].vertices.iter_mut().zip(steep_vertices) {
        vertex.position = position;
    }
    let steep = StaticModel::new(source).unwrap();
    assert!(
        candidate_model(&placement(steep), true, AgentProfile::default())
            .unwrap_err()
            .contains("slope limit")
    );
}

#[test]
fn rejects_out_of_bounds_and_degenerate_models_without_clamping() {
    let mut outside = orr_model::IDENTITY;
    outside[3][0] = 257.0;
    let error = candidate_model(
        &placement(flat_model(outside)),
        true,
        AgentProfile::default(),
    )
    .unwrap_err();
    assert!(error.contains("±256"));

    let degenerate_positions = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [0.0, 0.0, 0.0],
    ];
    let mut source = flat_model(orr_model::IDENTITY).source().clone();
    for (vertex, position) in source.primitives[0]
        .vertices
        .iter_mut()
        .zip(degenerate_positions)
    {
        vertex.position = position;
    }
    let model = StaticModel::new(source).unwrap();
    assert!(
        candidate_model(&placement(model), true, AgentProfile::default())
            .unwrap_err()
            .contains("degenerate")
    );
}

#[test]
fn rejects_excessive_graph_and_source_vertices_before_graph_admission() {
    let mut large_source = flat_model(orr_model::IDENTITY).source().clone();
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for face_index in 0..300u32 {
        let column = (face_index % 20) as f32 * 2.0;
        let row = (face_index / 20) as f32 * 2.0;
        let base = vertices.len() as u32;
        vertices.extend([
            vertex([column, 0.0, row]),
            vertex([column, 0.0, row + 1.0]),
            vertex([column + 1.0, 0.0, row + 1.0]),
        ]);
        indices.extend([base, base + 1, base + 2]);
    }
    large_source.primitives[0].vertices = vertices;
    large_source.primitives[0].indices = indices;
    let large_graph_model = StaticModel::new(large_source).unwrap();
    assert!(
        candidate_model(&placement(large_graph_model), true, AgentProfile::default())
            .unwrap_err()
            .contains("8 KiB")
    );

    let mut oversized_source = flat_model(orr_model::IDENTITY).source().clone();
    oversized_source.primitives[0]
        .vertices
        .resize(4097, vertex([0.0, 0.0, 0.0]));
    let oversized_model = StaticModel::new(oversized_source).unwrap();
    assert!(
        candidate_model(&placement(oversized_model), true, AgentProfile::default())
            .unwrap_err()
            .contains("4096 source vertex limit")
    );
}

#[test]
fn rejects_external_model_dependencies_and_leaves_package_bytes_unchanged() {
    let temporary = tempfile::tempdir().unwrap();
    let mut source = flat_model(orr_model::IDENTITY).source().clone();
    source.dependencies.push(Dependency {
        uri: "textures/external.bin".into(),
        sha256: "1".repeat(64),
    });
    let model = StaticModel::new(source).unwrap();
    let package_root = package(temporary.path(), &model);
    let model_path = package_root.join("floor.orrmodel");
    let original_bytes = fs::read(&model_path).unwrap();
    let result = Snapshot::from_package(&package_root, "floor.orrmodel", "LICENSE.txt", revision());
    assert!(result.err().unwrap().contains("external dependencies"));
    assert_eq!(fs::read(model_path).unwrap(), original_bytes);
}
