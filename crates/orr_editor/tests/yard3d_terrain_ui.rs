//! Production EditorApp/egui controls against a real local Yard host, CPU-only.
//! GPU composition has separate mandatory production-viewport coverage.
#![cfg(feature = "terrain")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::{accesskit::Role, Key, Modifiers};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_editor::{
    game::EditorGame,
    terrain_pick::{TerrainQuery, TerrainSelectionMode},
    Editor, EditorApp,
};
use orr_fp::FP;
use std::path::Path;

const FIXTURE: &str = include_str!("../../../scenes/yard3d_authoring.scene.yaml");
fn setup(root: &Path) -> Harness<'static, EditorApp> {
    let scene = root.join("yard.scene.yaml");
    std::fs::write(&scene, FIXTURE).unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::Yard3D).unwrap();
    editor.sync();
    assert!(editor.yard_rows_coherent());
    Harness::builder()
        .with_size([1280.0, 800.0])
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |_| EditorApp::new(editor, None))
}
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(3);
    h.get_by_label(label).click();
    h.run_steps(3);
    h.state_mut().editor.sync();
    h.run_steps(2);
}
fn type_into(h: &mut Harness<'_, EditorApp>, label: &str, text: &str) {
    h.get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    h.run_steps(3);
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(text);
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(2);
}
fn terrain_bytes(h: &Harness<'_, EditorApp>) -> Vec<u8> {
    h.state().terrain.document.bytes().unwrap().to_vec()
}

#[test]
fn real_editor_widgets_create_exact_edit_hole_query_undo_save_reopen_and_guard_discard() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = setup(dir.path());
    let checksum = h.state().editor.checksum();
    let host_history = h.state().editor.history().entries.len();
    click(&mut h, "Terrain authoring");
    click(&mut h, "New terrain settings");
    click(&mut h, "Installed terrain package");
    click(&mut h, "Create terrain");
    assert!(
        h.state().terrain.error().is_none(),
        "{:?}",
        h.state().terrain.error()
    );
    let initial = terrain_bytes(&h);
    click(&mut h, "Vertex");
    type_into(&mut h, "Terrain height", "0.0000152587890625");
    click(&mut h, "Apply terrain height");
    assert_eq!(
        h.state().terrain.document.terrain().unwrap().heights()[0].raw(),
        1
    );
    let raised = terrain_bytes(&h);
    // A repeated Apply creates no extra history entry.
    click(&mut h, "Apply terrain height");
    click(&mut h, "Terrain Undo");
    assert_eq!(terrain_bytes(&h), initial);
    click(&mut h, "Terrain Redo");
    assert_eq!(terrain_bytes(&h), raised);
    type_into(&mut h, "Terrain query X", "-4");
    type_into(&mut h, "Terrain query Z", "-4");
    click(&mut h, "Query terrain surface");
    assert_eq!(
        h.state().terrain.document.marker().unwrap().anchor()[1].raw(),
        1
    );
    assert!(matches!(
        h.state().terrain.query_result(),
        Some(TerrainQuery::Surface { .. })
    ));
    let revision = h.state().terrain.document.revision();
    let model = h.state().terrain.document.model().unwrap().clone();
    // Invalid text must leave history, query marker, bytes and cache untouched.
    type_into(&mut h, "Terrain height", "NaN");
    click(&mut h, "Apply terrain height");
    assert!(h.state().terrain.error().is_some());
    assert_eq!(h.state().terrain.document.revision(), revision);
    assert_eq!(terrain_bytes(&h), raised);
    assert!(std::sync::Arc::ptr_eq(
        &model,
        h.state().terrain.document.model().unwrap()
    ));
    assert_eq!(
        h.state().terrain.document.marker().unwrap().anchor()[1].raw(),
        1
    );
    click(&mut h, "Cell");
    click(&mut h, "Terrain cell is a hole");
    click(&mut h, "Apply terrain hole");
    assert!(h.state().terrain.document.terrain().unwrap().holes()[0]);
    assert!(h.state().terrain.document.marker().is_none());
    assert_eq!(
        h.state().terrain.query_result(),
        Some(TerrainQuery::Hole { cell: [0, 0] })
    );
    click(&mut h, "Terrain Undo");
    assert_eq!(terrain_bytes(&h), raised);
    assert!(h.state().terrain.document.marker().is_some());
    click(&mut h, "Terrain Redo");
    assert!(h.state().terrain.document.marker().is_none());
    click(&mut h, "Save terrain");
    assert!(!h.state().terrain.document.dirty());
    let saved = terrain_bytes(&h);
    assert_eq!(
        std::fs::read(dir.path().join("terrain.orrt")).unwrap(),
        saved
    );
    click(&mut h, "Close terrain");
    assert!(h.state().terrain.document.terrain().is_none());
    click(&mut h, "Open terrain");
    assert_eq!(terrain_bytes(&h), saved);
    click(&mut h, "Terrain cell is a hole");
    click(&mut h, "Apply terrain hole");
    assert!(h.state().terrain.document.dirty());
    click(&mut h, "Close terrain");
    assert!(h.state().terrain.document.terrain().is_some());
    click(&mut h, "Keep terrain open");
    assert!(h.state().terrain.document.dirty());
    // Dirty Open is rejected rather than silently replacing the current edit.
    click(&mut h, "Open terrain");
    assert!(h.state().terrain.error().is_some());
    assert!(h.state().terrain.document.dirty());
    click(&mut h, "Close terrain");
    click(&mut h, "Discard terrain changes and close");
    click(&mut h, "Open terrain");
    assert_eq!(terrain_bytes(&h), saved);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), host_history);
}

#[test]
fn production_panel_read_only_package_copy_keeps_installed_bytes_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    let source = dir.path().join("source");
    let fixture = orr_terrain_view::fixture();
    let installed =
        orr_terrain_view::package::install_and_reload(&fixture, &project, &source, "1.0.0")
            .unwrap();
    let package_file = project
        .join(".orr/packages/objects")
        .join(installed.package_digest)
        .join("lab.orrt");
    let installed_bytes = std::fs::read(&package_file).unwrap();
    let mut h = setup(dir.path());
    let checksum = h.state().editor.checksum();
    click(&mut h, "Terrain authoring");
    click(&mut h, "Installed terrain package");
    type_into(&mut h, "Terrain project root", project.to_str().unwrap());
    type_into(&mut h, "Terrain package name", "heightfield-lab");
    type_into(&mut h, "Terrain asset path in package", "lab.orrt");
    click(&mut h, "Open verified terrain package");
    assert!(
        h.state().terrain.error().is_none(),
        "{:?}",
        h.state().terrain.error()
    );
    assert!(h.state().terrain.document.read_only());
    assert_eq!(terrain_bytes(&h), installed_bytes);
    click(&mut h, "Vertex");
    type_into(&mut h, "Terrain height", "2.5");
    assert!(h
        .get_by_label("Apply terrain height")
        .accesskit_node()
        .is_disabled());
    assert!(h
        .get_by_label("Save terrain")
        .accesskit_node()
        .is_disabled());
    type_into(&mut h, "Terrain scene-relative path", "copied.orrt");
    click(&mut h, "Copy terrain to scene");
    assert!(!h.state().terrain.document.read_only());
    click(&mut h, "Apply terrain height");
    assert_eq!(
        h.state().terrain.document.terrain().unwrap().heights()[0],
        FP::from_raw(163840)
    );
    click(&mut h, "Save terrain");
    assert_eq!(std::fs::read(&package_file).unwrap(), installed_bytes);
    assert_eq!(
        std::fs::read(dir.path().join("copied.orrt")).unwrap(),
        terrain_bytes(&h)
    );
    assert_eq!(h.state().editor.checksum(), checksum);
    assert!(h.state().editor.history().entries.is_empty());
}

#[test]
fn play_preview_scene_switch_and_invalid_numeric_indices_are_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = setup(dir.path());
    click(&mut h, "Terrain authoring");
    click(&mut h, "Create terrain");
    click(&mut h, "Save terrain");
    let before = terrain_bytes(&h);
    {
        let app = h.state_mut();
        app.terrain.selection_mode = TerrainSelectionMode::Vertex;
        app.terrain.selected_vertex = [u32::MAX, 0];
        app.terrain.height = "4".into();
        assert!(app.terrain.apply_height(&app.editor).is_err());
        assert_eq!(app.terrain.document.bytes().unwrap(), before);
        assert!(!app.terrain.document.dirty());
        app.terrain.selected_vertex = [0, 0];
        let proposal = app
            .editor
            .agent_client("terrain-preview-test")
            .unwrap()
            .call(
                "proposal.begin",
                serde_json::json!({"label":"terrain preview guard"}),
            )
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        app.editor.sync();
        assert!(app.editor.set_preview(Some(proposal)));
        app.editor.sync();
        assert!(app.terrain.apply_height(&app.editor).is_err());
        assert!(app.terrain.create_for_editor(&app.editor).is_err());
        assert!(app.terrain.open_for_editor(&app.editor).is_err());
        assert!(app.terrain.viewport_model(&app.editor).is_none());
        assert!(app.terrain.attached_for_editor(&app.editor));
        assert_eq!(app.terrain.document.bytes().unwrap(), before);
        assert!(app.editor.set_preview(None));
        app.editor.sync();
        app.editor.play();
        app.editor.sync();
        assert!(app.terrain.apply_height(&app.editor).is_err());
        assert!(app.terrain.attached_for_editor(&app.editor));
        assert!(!app.terrain.authoring_active(&app.editor));
        app.editor.stop();
        app.editor.sync();
        assert!(app.terrain.apply_height(&app.editor).is_ok());
        assert!(app.terrain.document.dirty());
    }
    let dirty = terrain_bytes(&h);
    let other = dir.path().join("other.scene.yaml");
    std::fs::write(&other, FIXTURE).unwrap();
    {
        let app = h.state_mut();
        assert!(app.editor.open_path(&other));
        app.editor.sync();
        app.terrain.sync_for_editor(&app.editor);
        assert!(!app.terrain.scene_matches(&app.editor));
        assert!(!app.terrain.attached_for_editor(&app.editor));
        assert!(app.terrain.viewport_model(&app.editor).is_none());
        assert_eq!(app.terrain.document.bytes().unwrap(), dirty);
        assert!(app.terrain.document.dirty());
        assert!(app.terrain.apply_height(&app.editor).is_err());
        assert!(app.terrain.open_for_editor(&app.editor).is_err());
        // An explicit Save still targets the retained original scene-owned path.
        app.terrain.save_for_editor(&app.editor).unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("terrain.orrt")).unwrap(),
            dirty
        );
    }
}

fn click_world(h: &mut Harness<'_, EditorApp>, world: [f32; 3]) {
    let rect = h.state().ui.viewport_rect.unwrap();
    let size = h.state().ui.viewport_px;
    let pixel = h
        .state()
        .editor
        .camera3d
        .camera()
        .world_to_screen(world, size)
        .unwrap();
    let at = rect.min
        + egui::vec2(
            pixel[0] * rect.width() / size.0 as f32,
            pixel[1] * rect.height() / size.1 as f32,
        );
    assert!(rect.contains(at));
    h.event(egui::Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    h.run_steps(3);
    h.state_mut().editor.sync();
    h.run_steps(2);
}

#[test]
fn main_viewport_routes_vertex_cell_hole_and_entity_selection_explicitly() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = setup(dir.path());
    h.state_mut().editor.camera3d = orr_render::OrbitCamera::new([0.0; 3], 0.0, 1.0, 15.0);
    click(&mut h, "Terrain authoring");
    click(&mut h, "Create terrain");
    click(&mut h, "box_left");
    let selected = h.state().editor.selection().cloned();
    let checksum = h.state().editor.checksum();
    click(&mut h, "Vertex");
    click_world(&mut h, [-2.0, 0.0, -2.0]);
    assert_eq!(h.state().terrain.selected_vertex, [2, 2]);
    assert_eq!(h.state().editor.selection(), selected.as_ref());
    click(&mut h, "Cell");
    click(&mut h, "Terrain cell is a hole");
    click(&mut h, "Apply terrain hole");
    assert!(h.state().terrain.document.terrain().unwrap().holes()[0]);
    h.state_mut().terrain.selected_cell = [5, 5];
    click_world(&mut h, [-3.5, 0.0, -3.5]);
    assert_eq!(h.state().terrain.selected_cell, [0, 0]);
    assert!(h.state().terrain.hole);
    assert_eq!(h.state().editor.selection(), selected.as_ref());
    click(&mut h, "Entity");
    click_world(&mut h, [2.0, 3.0, 0.0]);
    let right = h
        .state()
        .editor
        .rows()
        .iter()
        .find(|r| r.name.as_deref() == Some("box_right"))
        .unwrap()
        .target();
    assert_eq!(h.state().editor.selection(), Some(&right));
    assert_eq!(h.state().editor.checksum(), checksum);
    assert!(h.state().editor.history().entries.is_empty());
}
