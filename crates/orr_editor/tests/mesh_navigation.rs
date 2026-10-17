#![cfg(feature = "mesh-navigation")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::{accesskit::Role, Key, Modifiers};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_editor::{
    app::{LBL_PAUSE, LBL_PLAY, LBL_STEP, LBL_STOP},
    game::EditorGame,
    Editor, EditorApp, Mode,
};
use orr_fp::{fp, FP};
use orr_model::{
    Dependency, Image, Material, ModelSource, Primitive, StaticModel, Vertex, Wrap, IDENTITY,
};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use orr_remote::mesh_navigation::{decode_asset, mesh_from_view};
use std::path::Path;

const BLANK_SCENE: &str = "schema: orr.scene/1\nsingletons: {}\nentities: {}\n";

fn setup(scene: &Path) -> Harness<'static, EditorApp> {
    std::fs::write(scene, BLANK_SCENE).unwrap();
    let mut editor = Editor::open_game(scene, EditorGame::MeshNavigation).unwrap();
    editor.sync();
    Harness::builder()
        .with_size([1280.0, 850.0])
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |_| EditorApp::new(editor, None))
}

fn click(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness.get_by_label(label).scroll_to_me();
    harness.run_steps(3);
    harness.get_by_label(label).click();
    harness.run_steps(3);
    harness.state_mut().editor.sync();
    harness.run_steps(2);
}

fn type_into(harness: &mut Harness<'_, EditorApp>, label: &str, text: &str) {
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .focus();
    harness.run_steps(2);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .type_text(text);
    harness.run_steps(2);
    harness.key_press(Key::Enter);
    harness.run_steps(2);
}

fn wait_for_playing_frame(harness: &mut Harness<'_, EditorApp>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        harness.state_mut().editor.sync();
        harness.run_steps(2);
        if harness
            .state()
            .editor
            .timeline()
            .is_some_and(|timeline| timeline.playing)
            && harness
                .state()
                .editor
                .snapshot()
                .is_some_and(|snapshot| snapshot.predicted().tick() > 0)
        {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    panic!("MeshNavigation host did not publish a playing Frame within three seconds");
}

fn ramp_bridge_graph() -> StaticMeshGraph {
    StaticMeshGraph::bake(
        &[
            [fp!(0), fp!(0), fp!(0)],
            [fp!(4), fp!(0), fp!(0)],
            [fp!(0), fp!(0), fp!(2)],
            [fp!(4), fp!(0), fp!(2)],
            [fp!(6), fp!(2), fp!(0)],
            [fp!(6), fp!(2), fp!(2)],
            [fp!(2), fp!(2), fp!(0)],
            [fp!(2), fp!(2), fp!(2)],
        ],
        &[
            [0, 2, 3],
            [0, 3, 1],
            [1, 3, 5],
            [1, 5, 4],
            [6, 7, 5],
            [6, 5, 4],
        ],
        AgentProfile {
            max_slope: fp!(4),
            radius: FP::ZERO,
            headroom: FP::ZERO,
            max_step: FP::ZERO,
        },
    )
    .unwrap()
}

fn ramp_bridge_model_bytes() -> Vec<u8> {
    let graph = ramp_bridge_graph();
    let source = ModelSource {
        format: "orr_static_model".into(),
        version: 1,
        asset_id: "ramp.orrmodel".into(),
        dependencies: vec![Dependency {
            uri: "$source".into(),
            sha256: "0".repeat(64),
        }],
        primitives: vec![Primitive {
            id: "ramp.orrmodel#node=0".into(),
            vertices: graph
                .vertices()
                .iter()
                .map(|point| Vertex {
                    position: point.map(FP::to_f32),
                    normal: [0.0, 1.0, 0.0],
                    uv: [0.0, 0.0],
                })
                .collect(),
            indices: graph
                .triangles()
                .iter()
                .flat_map(|triangle| triangle.vertices)
                .collect(),
            material: 0,
            transform: IDENTITY,
        }],
        materials: vec![Material {
            base_color: [0.7, 0.7, 0.7, 1.0],
            image: 0,
            linear_filter: false,
            wrap_s: Wrap::Clamp,
            wrap_t: Wrap::Clamp,
        }],
        images: vec![Image {
            width: 1,
            height: 1,
            rgba8: vec![255; 4],
        }],
    };
    StaticModel::new(source).unwrap().to_bytes().unwrap()
}

#[test]
fn open_mesh_asset_button_places_and_reopens_the_validated_asset() {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let scene = root_path.join("mesh-navigation.scene.yaml");
    let asset = root_path.join("bridge.orrmesh");
    let graph = ramp_bridge_graph();
    std::fs::write(&asset, graph.to_bytes()).unwrap();
    let mut harness = setup(&scene);
    click(&mut harness, "Static-mesh navigation");
    harness.state_mut().mesh_navigation.asset_path = asset.display().to_string();
    click(&mut harness, "Open mesh asset");
    let before_place = harness.state().editor.checksum();
    assert!(harness.state().mesh_navigation.model().is_none());
    click(&mut harness, "Place mesh agent");
    assert_ne!(harness.state().editor.checksum(), before_place);
    let admitted = mesh_from_view(harness.state().editor.snapshot().unwrap().predicted()).unwrap();
    assert_eq!(admitted.graph.revision(), graph.revision());
    assert!(
        harness.state().mesh_navigation.model().is_some(),
        "admitted frame view model failed to build: {:?}",
        harness.state().mesh_navigation.error
    );
    assert!(harness.state_mut().editor.save());
    let saved_checksum = harness.state().editor.checksum();
    drop(harness);

    let mut reopened = Editor::open_game(&scene, EditorGame::MeshNavigation).unwrap();
    reopened.sync();
    assert_eq!(reopened.checksum(), saved_checksum);
    let restored = mesh_from_view(reopened.snapshot().unwrap().predicted()).unwrap();
    assert_eq!(restored.graph.revision(), graph.revision());
}

#[test]
fn opening_authored_mesh_scene_refits_camera_from_admitted_frame() {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let scene = root_path.join("mesh-navigation.scene.yaml");
    let mut harness = setup(&scene);
    click(&mut harness, "Static-mesh navigation");
    click(&mut harness, "Create ramp and bridge");
    click(&mut harness, "Place mesh agent");
    assert!(harness.state_mut().editor.save());

    let admitted = mesh_from_view(harness.state().editor.snapshot().unwrap().predicted()).unwrap();
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for point in admitted
        .graph
        .vertices()
        .iter()
        .copied()
        .chain([admitted.agent.spec.start, admitted.agent.spec.goal])
    {
        for axis in 0..3 {
            let value = point[axis].to_f32();
            low[axis] = low[axis].min(value);
            high[axis] = high[axis].max(value);
        }
    }
    let expected_target = std::array::from_fn(|axis| (low[axis] + high[axis]) * 0.5);
    let extent = (0..3)
        .map(|axis| high[axis] - low[axis])
        .fold(0.0_f32, f32::max);
    let expected_distance = (extent * 1.8).max(2.0);

    harness.state_mut().editor.camera3d.target = [900.0, 900.0, 900.0];
    harness.state_mut().editor.camera3d.distance = 120.0;
    assert!(harness.state_mut().editor.open_path(&scene));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        harness.state_mut().editor.sync();
        harness.run_steps(2);
        let camera = harness.state().editor.camera3d;
        if camera.target == expected_target && camera.distance == expected_distance {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(harness.state().editor.camera3d.target, expected_target);
    assert_eq!(harness.state().editor.camera3d.distance, expected_distance);
}

#[test]
fn changing_scene_clears_the_previous_mesh_before_place_or_play() {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let scene = root_path.join("mesh-navigation.scene.yaml");
    let replacement_scene = root_path.join("replacement.scene.yaml");
    std::fs::write(&replacement_scene, BLANK_SCENE).unwrap();
    let mut harness = setup(&scene);
    click(&mut harness, "Static-mesh navigation");
    click(&mut harness, "Create ramp and bridge");
    click(&mut harness, "Place mesh agent");
    assert!(harness.state().mesh_navigation.model().is_some());

    let mut replacement =
        Editor::open_game(&replacement_scene, EditorGame::MeshNavigation).unwrap();
    replacement.sync();
    harness.state_mut().editor = replacement;
    harness.run_steps(4);

    assert!(harness.state().mesh_navigation.model().is_none());
    assert!(harness
        .get_by_label("Place mesh agent")
        .accesskit_node()
        .is_disabled());
    let empty_checksum = harness.state().editor.checksum();
    click(&mut harness, "Place mesh agent");
    assert_eq!(harness.state().editor.checksum(), empty_checksum);
    click(&mut harness, LBL_PLAY);
    assert_eq!(harness.state().editor.mode(), Mode::Edit);
}

#[test]
fn scene_switch_clears_the_previous_scene_mesh_before_place_or_play() {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let scene = root_path.join("mesh-navigation.scene.yaml");
    let replacement_scene = root_path.join("replacement.scene.yaml");
    std::fs::write(&replacement_scene, BLANK_SCENE).unwrap();
    let mut harness = setup(&scene);
    click(&mut harness, "Static-mesh navigation");
    click(&mut harness, "Create ramp and bridge");
    click(&mut harness, "Place mesh agent");
    assert!(harness.state().mesh_navigation.model().is_some());

    let mut replacement =
        Editor::open_game(&replacement_scene, EditorGame::MeshNavigation).unwrap();
    replacement.sync();
    harness.state_mut().editor = replacement;
    harness.run_steps(4);

    assert!(harness.state().mesh_navigation.model().is_none());
    assert!(harness
        .get_by_label("Place mesh agent")
        .accesskit_node()
        .is_disabled());
    let empty_checksum = harness.state().editor.checksum();
    click(&mut harness, "Place mesh agent");
    assert_eq!(harness.state().editor.checksum(), empty_checksum);
    click(&mut harness, LBL_PLAY);
    assert_eq!(harness.state().editor.mode(), Mode::Edit);
}

#[test]
fn model_import_saves_canonical_mesh_without_frame_mutation_and_reopens() {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let scene = root_path.join("mesh-navigation.scene.yaml");
    let model = root_path.join("ramp.orrmodel");
    let output = root_path.join("ramp.orrmesh");
    let existing = root_path.join("existing.orrmesh");
    std::fs::write(&model, ramp_bridge_model_bytes()).unwrap();
    std::fs::write(&existing, b"preserve existing mesh").unwrap();
    let mut harness = setup(&scene);
    click(&mut harness, "Static-mesh navigation");
    harness.state_mut().mesh_navigation.model_path = model.display().to_string();
    click(&mut harness, "Open model");
    let before_save = harness.state().editor.checksum();
    harness.state_mut().mesh_navigation.mesh_output_path = output.display().to_string();
    click(&mut harness, "Save mesh asset");
    assert_eq!(harness.state().editor.checksum(), before_save);
    let saved_graph = decode_asset(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(saved_graph.triangles().len(), 6);

    harness.state_mut().mesh_navigation.mesh_output_path = existing.display().to_string();
    click(&mut harness, "Save mesh asset");
    assert_eq!(harness.state().editor.checksum(), before_save);
    assert_eq!(std::fs::read(&existing).unwrap(), b"preserve existing mesh");

    click(&mut harness, "Place mesh agent");
    let admitted = mesh_from_view(harness.state().editor.snapshot().unwrap().predicted()).unwrap();
    assert_eq!(admitted.graph.revision(), saved_graph.revision());
    assert!(harness.state_mut().editor.save());
    let saved_checksum = harness.state().editor.checksum();
    drop(harness);

    let mut reopened = Editor::open_game(&scene, EditorGame::MeshNavigation).unwrap();
    reopened.sync();
    assert_eq!(reopened.checksum(), saved_checksum);
    let restored = mesh_from_view(reopened.snapshot().unwrap().predicted()).unwrap();
    assert_eq!(restored.graph.revision(), saved_graph.revision());
}

#[test]
fn widget_authors_scene_owned_mesh_reopens_and_moves_in_the_real_host() {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let scene = root_path.join("mesh-navigation.scene.yaml");
    let mut harness = setup(&scene);
    click(&mut harness, "Static-mesh navigation");
    click(&mut harness, "Place mesh agent");
    assert_eq!(
        harness.state().editor.mode(),
        Mode::Edit,
        "empty mesh cannot play"
    );
    assert!(harness.state().mesh_navigation.model().is_none());

    let before_placement = harness.state().editor.checksum();
    click(&mut harness, "Create ramp and bridge");
    assert_eq!(
        harness.state().editor.checksum(),
        before_placement,
        "geometry draft is not a host edit"
    );
    harness.state_mut().mesh_navigation.start[0] = "1.5".into();
    click(&mut harness, "Place mesh agent");
    assert!(!harness.state().editor.history().entries.is_empty());
    assert!(
        harness.state().mesh_navigation.model().is_some(),
        "viewport model comes from admitted Frame state"
    );
    let admitted = mesh_from_view(harness.state().editor.snapshot().unwrap().predicted()).unwrap();
    assert_eq!(admitted.graph.triangles().len(), 6);
    let initial_position = admitted.agent.position;
    assert!(harness.state_mut().editor.undo());
    harness.state_mut().mesh_navigation.start[0] = "2".into();
    harness.state_mut().editor.sync();
    harness.run_steps(4);
    assert!(
        harness.state().mesh_navigation.model().is_none(),
        "Undo removes the placed mesh pin from the real host"
    );
    assert_eq!(harness.state().mesh_navigation.start[0], "1");
    assert!(harness.state_mut().editor.redo());
    harness.state_mut().editor.sync();
    harness.run_steps(4);
    assert!(
        harness.state().mesh_navigation.model().is_some(),
        "Redo re-admits the scene-owned graph"
    );
    assert_eq!(harness.state().mesh_navigation.start[0], "1.5");
    assert!(
        harness.state_mut().editor.save(),
        "scene-owned mesh pin saves through the host"
    );
    let edit_checksum = harness.state().editor.checksum();
    drop(harness);

    let mut editor = Editor::open_game(&scene, EditorGame::MeshNavigation).unwrap();
    editor.sync();
    assert_eq!(
        editor.checksum(),
        edit_checksum,
        "reopened document restores its admitted graph and agent"
    );
    let mut harness = Harness::builder()
        .with_size([1280.0, 850.0])
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |_| EditorApp::new(editor, None));
    harness.run_steps(4);
    assert!(harness.state().mesh_navigation.model().is_some());

    click(&mut harness, LBL_PLAY);
    assert_eq!(harness.state().editor.mode(), Mode::Play);
    wait_for_playing_frame(&mut harness);
    let moved = mesh_from_view(harness.state().editor.snapshot().unwrap().predicted()).unwrap();
    assert_ne!(
        moved.agent.position, initial_position,
        "the simulation tick advances the runtime-owned agent"
    );
    click(&mut harness, LBL_PAUSE);
    click(&mut harness, LBL_STEP);
    click(&mut harness, LBL_STOP);
    assert_eq!(harness.state().editor.mode(), Mode::Edit);
}

#[test]
fn invalid_draft_blocks_play_without_mutating_the_scene_pin() {
    let root = tempfile::tempdir().unwrap();
    let scene = root
        .path()
        .canonicalize()
        .unwrap()
        .join("mesh-navigation.scene.yaml");
    let mut harness = setup(&scene);
    click(&mut harness, "Static-mesh navigation");
    click(&mut harness, "Create ramp and bridge");
    let original_checksum = harness.state().editor.checksum();
    harness
        .get_by_role_and_label(Role::TextInput, "Maximum vertical projection")
        .scroll_to_me();
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::TextInput, "Maximum vertical projection")
        .focus();
    harness.run_steps(2);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::TextInput, "Maximum vertical projection")
        .type_text("NaN");
    harness.run_steps(2);
    harness.key_press(Key::Enter);
    harness.run_steps(2);
    click(&mut harness, "Place mesh agent");
    assert_eq!(harness.state().editor.checksum(), original_checksum);
    click(&mut harness, LBL_PLAY);
    assert_eq!(
        harness.state().editor.mode(),
        Mode::Edit,
        "unplaced/invalid draft cannot start play"
    );
}

#[test]
fn changed_agent_draft_blocks_play_until_replaced_in_the_host() {
    let root = tempfile::tempdir().unwrap();
    let scene = root
        .path()
        .canonicalize()
        .unwrap()
        .join("mesh-navigation.scene.yaml");
    let mut harness = setup(&scene);
    click(&mut harness, "Static-mesh navigation");
    click(&mut harness, "Create ramp and bridge");
    click(&mut harness, "Place mesh agent");
    let placed_checksum = harness.state().editor.checksum();

    type_into(&mut harness, "Distance per tick", "0.0625");
    assert_eq!(harness.state().mesh_navigation.distance_per_tick, "0.0625");
    click(&mut harness, LBL_PLAY);
    assert_eq!(harness.state().editor.mode(), Mode::Edit);
    assert_eq!(harness.state().editor.checksum(), placed_checksum);

    click(&mut harness, "Place mesh agent");
    assert_ne!(harness.state().editor.checksum(), placed_checksum);
    click(&mut harness, LBL_PLAY);
    assert_eq!(harness.state().editor.mode(), Mode::Play);
    click(&mut harness, LBL_STOP);
    assert_eq!(harness.state().editor.mode(), Mode::Edit);
}

#[test]
fn pending_agent_draft_survives_host_undo_refresh_and_can_be_placed() {
    let root = tempfile::tempdir().unwrap();
    let scene = root
        .path()
        .canonicalize()
        .unwrap()
        .join("mesh-navigation.scene.yaml");
    let mut harness = setup(&scene);
    click(&mut harness, "Static-mesh navigation");
    click(&mut harness, "Create ramp and bridge");
    click(&mut harness, "Place mesh agent");
    type_into(&mut harness, "Distance per tick", "0.0625");

    assert!(harness.state_mut().editor.undo());
    harness.state_mut().editor.sync();
    harness.run_steps(4);

    assert_eq!(harness.state().mesh_navigation.distance_per_tick, "0.0625");
    assert!(harness.state().mesh_navigation.model().is_none());
    assert_eq!(harness.state().editor.mode(), Mode::Edit);
    click(&mut harness, "Place mesh agent");
    assert_eq!(harness.state().mesh_navigation.distance_per_tick, "0.0625");
    assert!(harness.state().mesh_navigation.model().is_some());
}

#[test]
fn save_as_preserves_drafts_but_opening_another_scene_clears_them() {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let scene = root_path.join("mesh-navigation.scene.yaml");
    let saved_as = root_path.join("mesh-navigation-copy.scene.yaml");
    let replacement = root_path.join("replacement.scene.yaml");
    std::fs::write(&replacement, BLANK_SCENE).unwrap();
    let mut harness = setup(&scene);
    click(&mut harness, "Static-mesh navigation");
    click(&mut harness, "Create ramp and bridge");
    type_into(&mut harness, "Distance per tick", "0.0625");

    assert!(harness.state_mut().editor.save_as(&saved_as));
    harness.state_mut().editor.sync();
    harness.run_steps(4);

    assert_eq!(
        harness.state().editor.path().as_deref(),
        Some(saved_as.as_path())
    );
    assert_eq!(harness.state().mesh_navigation.distance_per_tick, "0.0625");
    click(&mut harness, "Place mesh agent");
    assert!(harness.state().mesh_navigation.model().is_some());

    assert!(harness.state_mut().editor.open_path(&replacement));
    harness.state_mut().editor.sync();
    harness.run_steps(4);

    assert_eq!(harness.state().mesh_navigation.distance_per_tick, "0.03125");
    assert!(harness.state().mesh_navigation.model().is_none());
    assert!(harness
        .get_by_label("Place mesh agent")
        .accesskit_node()
        .is_disabled());
}
