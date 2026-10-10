//! Sculpt stamps exercised through the production panel and local Yard host.
#![cfg(feature = "terrain")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::{accesskit::Role, Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_editor::{
    game::EditorGame, terrain_document::TerrainSession, terrain_panel::SculptMode,
    terrain_pick::TerrainSelectionMode, Editor, EditorApp,
};
use orr_fp::FP;
use std::{path::Path, sync::Arc};

const FIXTURE: &str = include_str!("../../../scenes/yard3d_authoring.scene.yaml");

fn setup(root: &Path) -> Harness<'static, EditorApp> {
    let scene = root.join("yard.scene.yaml");
    std::fs::write(&scene, FIXTURE).unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::Yard3D).unwrap();
    editor.sync();
    assert!(editor.yard_rows_coherent());
    Harness::builder()
        .with_size([1280.0, 1000.0])
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
fn type_number(h: &mut Harness<'_, EditorApp>, label: &str, text: &str) {
    h.get_by_role_and_label(Role::SpinButton, label)
        .scroll_to_me();
    h.run_steps(2);
    h.get_by_role_and_label(Role::SpinButton, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::SpinButton, label)
        .type_text(text);
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(2);
}
fn footprint_circles(h: &Harness<'_, EditorApp>) -> usize {
    fn count(shape: &egui::epaint::Shape) -> usize {
        match shape {
            egui::epaint::Shape::Circle(circle)
                if circle.fill == egui::Color32::from_rgb(110, 220, 240)
                    && circle.radius == 2.5 =>
            {
                1
            }
            egui::epaint::Shape::Vec(shapes) => shapes.iter().map(count).sum(),
            _ => 0,
        }
    }
    h.output()
        .shapes
        .iter()
        .map(|shape| count(&shape.shape))
        .sum()
}
fn bytes(h: &Harness<'_, EditorApp>) -> Vec<u8> {
    h.state().terrain.document.bytes().unwrap().to_vec()
}
fn new_stamp(h: &mut Harness<'_, EditorApp>) {
    click(h, "Terrain authoring");
    click(h, "Create terrain");
    click(h, "Vertex");
    click(h, "Terrain sculpt stamp");
}
fn assert_model_matches_grid(session: &TerrainSession) {
    let terrain = session.terrain().unwrap();
    let model = session.model().unwrap();
    let mesh = &model.source().primitives[0];
    assert_eq!(mesh.vertices.len(), terrain.triangles().len() * 3);
    for (vertex, index) in mesh
        .vertices
        .iter()
        .zip(terrain.triangles().into_iter().flatten())
    {
        assert_eq!(
            vertex.position,
            terrain.vertex_position(index).unwrap().map(FP::to_f32)
        );
    }
}

#[test]
fn real_stamp_widgets_raise_query_undo_redo_save_reopen_without_host_edits() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut h = setup(&root);
    h.state_mut().editor.camera3d = orr_render::OrbitCamera::new([0.0; 3], 0.45, 0.75, 16.0);
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().entries.len();
    new_stamp(&mut h);
    click(&mut h, "Save terrain");
    let original = bytes(&h);
    assert_eq!(footprint_circles(&h), 0);
    type_number(&mut h, "Sculpt radius in grid steps", "1");
    assert_eq!(h.state().terrain.sculpt_radius, 1);
    assert!(h.query_by_label("Stamp affects 3 vertices").is_some());
    click(&mut h, "Show sculpt footprint");
    assert!(h.state().terrain.show_sculpt_footprint);
    assert_eq!(footprint_circles(&h), 3);
    type_number(&mut h, "Sculpt radius in grid steps", "2");
    assert_eq!(h.state().terrain.sculpt_radius, 2);
    assert!(h.query_by_label("Stamp affects 6 vertices").is_some());
    assert_eq!(footprint_circles(&h), 6);
    click(&mut h, "Apply sculpt stamp");
    assert!(
        h.state().terrain.error().is_none(),
        "{:?}",
        h.state().terrain.error()
    );
    let changed = bytes(&h);
    assert_ne!(changed, original);
    let terrain = h.state().terrain.document.terrain().unwrap();
    for z in 0..terrain.depth() {
        for x in 0..terrain.width() {
            let expected = if x * x + z * z <= 4 {
                FP::from_raw(32768)
            } else {
                FP::ZERO
            };
            assert_eq!(
                terrain.heights()[(z * terrain.width() + x) as usize],
                expected
            );
        }
    }
    assert_model_matches_grid(&h.state().terrain.document);
    type_into(&mut h, "Terrain query X", "-4");
    type_into(&mut h, "Terrain query Z", "-4");
    click(&mut h, "Query terrain surface");
    assert_eq!(
        h.state().terrain.document.surface().unwrap().height.raw(),
        32768
    );
    assert_eq!(
        h.state().terrain.document.marker().unwrap().anchor()[1].raw(),
        32768
    );
    click(&mut h, "Terrain Undo");
    assert_eq!(bytes(&h), original);
    assert_eq!(
        h.state().terrain.document.surface().unwrap().height,
        FP::ZERO
    );
    assert!(
        !h.state().terrain.document.can_undo(),
        "one stamp must have one undo entry"
    );
    click(&mut h, "Terrain Redo");
    assert_eq!(bytes(&h), changed);
    assert_model_matches_grid(&h.state().terrain.document);
    click(&mut h, "Save terrain");
    let revision = h.state().terrain.document.revision();
    assert_eq!(std::fs::read(root.join("terrain.orrt")).unwrap(), changed);
    click(&mut h, "Close terrain");
    click(&mut h, "Open terrain");
    assert_eq!(bytes(&h), changed);
    assert_eq!(h.state().terrain.document.revision(), revision);
    assert!(!h.state().terrain.document.dirty());
    assert!(!h.state().terrain.document.can_undo());
    assert_model_matches_grid(&h.state().terrain.document);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), history);
}

fn sampled_flatten_height_drives_stamp_and_frozen_stroke() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let mut h = setup(&root);
    h.state_mut().editor.camera3d = orr_render::OrbitCamera::new([0.0; 3], 0.0, 1.1, 16.0);
    new_stamp(&mut h);
    type_number(&mut h, "Terrain vertex X", "2");
    type_number(&mut h, "Terrain vertex Z", "2");
    type_into(&mut h, "Terrain height", "-1.25");
    click(&mut h, "Apply terrain height");
    assert_eq!(
        h.state().terrain.document.terrain().unwrap().heights()[2 * 9 + 2],
        FP::from_raw(-81920)
    );
    // Fixture hole and redo branch prove sampling itself is not a terrain edit.
    h.state_mut()
        .terrain
        .document
        .apply(&[orr_terrain::Edit::SetHole {
            x: 4,
            z: 4,
            hole: true,
        }])
        .unwrap();
    h.state_mut()
        .terrain
        .document
        .apply(&[orr_terrain::Edit::SetHeight {
            x: 0,
            z: 0,
            height: FP::ONE,
        }])
        .unwrap();
    click(&mut h, "Terrain Undo");
    click(&mut h, "Flatten");
    type_into(&mut h, "Sculpt target height", "9");
    type_into(&mut h, "Terrain height", "123"); // unapplied draft is not authoritative
    type_into(&mut h, "Terrain query X", "-2");
    type_into(&mut h, "Terrain query Z", "-2");
    click(&mut h, "Query terrain surface");
    assert_eq!(
        h.state().terrain.document.surface().unwrap().height,
        FP::from_raw(-81920)
    );
    let before = bytes(&h);
    let revision = h.state().terrain.document.revision();
    let dirty = h.state().terrain.document.dirty();
    let model = h.state().terrain.document.model().unwrap().clone();
    let query = format!(
        "{:?}",
        (
            h.state().terrain.document.query(),
            h.state().terrain.document.surface(),
            h.state().terrain.document.dirty_region()
        )
    );
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().clone();
    assert!(h.state().terrain.document.can_redo());
    click(&mut h, "Sample selected vertex height");
    assert_eq!(h.state().terrain.sculpt_height, "-1.25");
    assert_eq!(bytes(&h), before);
    assert_eq!(h.state().terrain.document.revision(), revision);
    assert_eq!(h.state().terrain.document.dirty(), dirty);
    assert!(h.state().terrain.document.can_redo());
    assert!(Arc::ptr_eq(
        &model,
        h.state().terrain.document.model().unwrap()
    ));
    assert_eq!(
        format!(
            "{:?}",
            (
                h.state().terrain.document.query(),
                h.state().terrain.document.surface(),
                h.state().terrain.document.dirty_region()
            )
        ),
        query
    );
    h.state_mut().terrain.selected_vertex = [u32::MAX, 0];
    h.run_steps(2);
    click(&mut h, "Sample selected vertex height");
    assert!(h.state().terrain.error().unwrap().contains("outside"));
    assert_eq!(h.state().terrain.sculpt_height, "-1.25");
    assert_eq!(bytes(&h), before);
    assert!(h.state().terrain.document.can_redo());
    type_number(&mut h, "Terrain vertex X", "4");
    type_number(&mut h, "Terrain vertex Z", "4");
    type_number(&mut h, "Sculpt radius in grid steps", "1");
    click(&mut h, "Apply sculpt stamp");
    let stamped = bytes(&h);
    assert_ne!(stamped, before);
    assert!(h.state().terrain.document.terrain().unwrap().holes()[4 * 8 + 4]);
    assert_eq!(
        h.state().terrain.document.terrain().unwrap().heights()[4 * 9 + 4],
        FP::from_raw(-81920)
    );
    click(&mut h, "Terrain Undo");
    assert_eq!(bytes(&h), before);
    click(&mut h, "Terrain Redo");
    assert_eq!(bytes(&h), stamped);
    click(&mut h, "Sculpt");
    let project = |h: &Harness<'_, EditorApp>, [x, z]: [u32; 2]| {
        let grid = h.state().terrain.document.sculpt_pick_terrain().unwrap();
        let point = grid
            .vertex_position(z * grid.width() + x)
            .unwrap()
            .map(FP::to_f32);
        let size = h.state().ui.viewport_px;
        let rect = h.state().ui.viewport_rect.unwrap();
        let pixel = h
            .state()
            .editor
            .camera3d
            .camera()
            .world_to_screen(point, size)
            .unwrap();
        let at = rect.min
            + egui::vec2(
                pixel[0] * rect.width() / size.0 as f32,
                pixel[1] * rect.height() / size.1 as f32,
            );
        assert!(rect.contains(at));
        at
    };
    let start = project(&h, [2, 6]);
    let end = project(&h, [6, 6]);
    let button = |h: &mut Harness<'_, EditorApp>, at: Pos2, pressed| {
        h.event(Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    };
    h.event(Event::PointerMoved(start));
    h.step();
    button(&mut h, start, true);
    h.step(); // redraw controls after the viewport begins the stroke
    assert!(h.state().terrain.document.stroke_active());
    assert!(h
        .get_by_label("Sample selected vertex height")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("Sample selected vertex height")
        .click_accesskit();
    h.step();
    assert_eq!(h.state().terrain.sculpt_height, "-1.25");
    h.state_mut().terrain.sculpt_height = "99".into(); // held operation is frozen
    h.event(Event::PointerMoved(end));
    h.step();
    button(&mut h, end, false);
    assert!(!h.state().terrain.document.stroke_active());
    for x in 2..=6 {
        assert_eq!(
            h.state().terrain.document.terrain().unwrap().heights()[6 * 9 + x],
            FP::from_raw(-81920)
        );
    }
    let stroked = bytes(&h);
    assert_ne!(stroked, stamped);
    assert!(h.state().terrain.document.terrain().unwrap().holes()[4 * 8 + 4]);
    click(&mut h, "Terrain Undo");
    assert_eq!(bytes(&h), stamped);
    click(&mut h, "Terrain Redo");
    assert_eq!(bytes(&h), stroked);
    click(&mut h, "Save terrain");
    click(&mut h, "Close terrain");
    click(&mut h, "Open terrain");
    assert_eq!(bytes(&h), stroked);
    assert!(!h.state().terrain.document.dirty());
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    h.state_mut().editor.play();
    h.state_mut().editor.sync();
    h.run_steps(2);
    assert!(h
        .get_by_label("Sample selected vertex height")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("Sample selected vertex height")
        .click_accesskit();
    h.step();
    assert_eq!(h.state().terrain.sculpt_height, "99");
    assert_eq!(bytes(&h), stroked);
    h.state_mut().editor.stop();
    h.state_mut().editor.sync();
    h.run_steps(2);
    let proposal = h
        .state_mut()
        .editor
        .agent_client("flatten-sample-preview")
        .unwrap()
        .call(
            "proposal.begin",
            serde_json::json!({"label":"flatten sample preview guard"}),
        )
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    h.state_mut().editor.sync();
    assert!(h.state_mut().editor.set_preview(Some(proposal)));
    h.state_mut().editor.sync();
    h.run_steps(2);
    assert!(h
        .get_by_label("Sample selected vertex height")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("Sample selected vertex height")
        .click_accesskit();
    h.step();
    assert_eq!(h.state().terrain.sculpt_height, "99");
    assert_eq!(bytes(&h), stroked);
    assert!(h.state_mut().editor.set_preview(None));
    h.state_mut().editor.sync();
    // Dirty terrain must stay associated with its old scene after replacement.
    h.state_mut()
        .terrain
        .document
        .apply(&[orr_terrain::Edit::SetHeight {
            x: 0,
            z: 0,
            height: FP::from_raw(16384),
        }])
        .unwrap();
    let detached = bytes(&h);
    let other = root.join("other.scene.yaml");
    std::fs::write(&other, FIXTURE).unwrap();
    assert!(h.state_mut().editor.open_path(&other));
    h.state_mut().editor.sync();
    h.run_steps(2);
    assert!(h
        .get_by_label("Sample selected vertex height")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("Sample selected vertex height")
        .click_accesskit();
    h.step();
    assert_eq!(h.state().terrain.sculpt_height, "99");
    assert_eq!(bytes(&h), detached);
    assert!(h
        .state()
        .terrain
        .viewport_model(&h.state().editor)
        .is_none());
}

#[test]
fn flatten_lower_and_snapshot_smooth_use_real_controls_and_preserve_holes() {
    sampled_flatten_height_drives_stamp_and_frozen_stroke();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut h = setup(&root);
    h.state_mut().terrain.dimensions = [3, 3];
    h.state_mut().terrain.origin = ["0".into(), "0".into()];
    new_stamp(&mut h);
    h.state_mut().terrain.selected_vertex = [1, 1];
    click(&mut h, "Flatten");
    type_into(&mut h, "Sculpt target height", "2");
    click(&mut h, "Apply sculpt stamp");
    let flat = bytes(&h);
    assert!(h
        .state()
        .terrain
        .document
        .terrain()
        .unwrap()
        .heights()
        .iter()
        .all(|h| *h == FP::from_int(2)));
    click(&mut h, "Apply sculpt stamp");
    click(&mut h, "Terrain Undo");
    assert!(h
        .state()
        .terrain
        .document
        .terrain()
        .unwrap()
        .heights()
        .iter()
        .all(|h| *h == FP::ZERO));
    assert!(!h.state().terrain.document.can_undo());
    click(&mut h, "Terrain Redo");
    assert_eq!(bytes(&h), flat);
    click(&mut h, "Raise/lower");
    type_into(&mut h, "Sculpt height change", "-2");
    click(&mut h, "Apply sculpt stamp");
    assert!(h
        .state()
        .terrain
        .document
        .terrain()
        .unwrap()
        .heights()
        .iter()
        .all(|h| *h == FP::ZERO));
    {
        let app = h.state_mut();
        app.terrain
            .document
            .apply(&[
                orr_terrain::Edit::SetHeight {
                    x: 1,
                    z: 1,
                    height: FP::from_int(9),
                },
                orr_terrain::Edit::SetHole {
                    x: 0,
                    z: 0,
                    hole: true,
                },
            ])
            .unwrap();
    }
    let spike = bytes(&h);
    click(&mut h, "Smooth");
    click(&mut h, "Apply sculpt stamp");
    assert!(
        h.state().terrain.error().is_none(),
        "{:?}",
        h.state().terrain.error()
    );
    let terrain = h.state().terrain.document.terrain().unwrap();
    assert_eq!(
        terrain
            .heights()
            .iter()
            .map(|h| h.raw())
            .collect::<Vec<_>>(),
        [147456, 98304, 147456, 98304, 65536, 98304, 147456, 98304, 147456]
    );
    assert_eq!(terrain.holes(), [true, false, false, false]);
    assert_eq!(terrain.sample(FP::ZERO, FP::ZERO), None);
    assert_eq!(terrain.sample(FP::ONE, FP::ONE), Some(FP::ONE));
    assert_model_matches_grid(&h.state().terrain.document);
    let smooth = bytes(&h);
    click(&mut h, "Terrain Undo");
    assert_eq!(bytes(&h), spike);
    click(&mut h, "Terrain Redo");
    assert_eq!(bytes(&h), smooth);
}

#[test]
fn invalid_text_bounds_admission_and_play_preserve_redo_mesh_and_query() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut h = setup(&root);
    new_stamp(&mut h);
    click(&mut h, "Save terrain");
    click(&mut h, "Apply sculpt stamp");
    let raised = bytes(&h);
    click(&mut h, "Terrain Undo");
    let before = bytes(&h);
    h.state_mut()
        .terrain
        .document
        .set_query(Some([FP::from_int(-4); 2]))
        .unwrap();
    let model = h.state().terrain.document.model().unwrap().clone();
    let revision = h.state().terrain.document.revision();
    let region = h.state().terrain.document.dirty_region();
    type_into(&mut h, "Sculpt height change", "NaN");
    click(&mut h, "Apply sculpt stamp");
    assert!(h.state().terrain.error().is_some());
    {
        let app = h.state_mut();
        app.terrain.sculpt_delta = "0.5".into();
        app.terrain.sculpt_radius = u32::MAX;
        assert!(app.terrain.apply_sculpt_stamp(&app.editor).is_err());
        app.terrain.sculpt_radius = 2;
        app.terrain.selected_vertex = [u32::MAX, 0];
        assert!(app.terrain.apply_sculpt_stamp(&app.editor).is_err());
        app.terrain.selected_vertex = [0, 0];
        app.terrain.sculpt_delta = "1000001".into();
        assert!(app.terrain.apply_sculpt_stamp(&app.editor).is_err());
        app.terrain.sculpt_delta = "256.0000152587890625".into();
        assert!(app.terrain.apply_sculpt_stamp(&app.editor).is_err());
        app.terrain.sculpt_delta = "0.5".into();
        app.terrain
            .document
            .set_render_admission_error(Some("composed viewport full".into()));
        assert!(app.terrain.apply_sculpt_stamp(&app.editor).is_err());
        app.terrain.document.set_render_admission_error(None);
        app.terrain.selection_mode = TerrainSelectionMode::Cell;
        assert!(app.terrain.apply_sculpt_stamp(&app.editor).is_err());
        app.terrain.selection_mode = TerrainSelectionMode::Vertex;
        let proposal = app
            .editor
            .agent_client("sculpt-preview-test")
            .unwrap()
            .call(
                "proposal.begin",
                serde_json::json!({"label":"sculpt preview guard"}),
            )
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        app.editor.sync();
        assert!(app.editor.set_preview(Some(proposal)));
        app.editor.sync();
        assert!(app.terrain.apply_sculpt_stamp(&app.editor).is_err());
        assert!(app.editor.set_preview(None));
        app.editor.sync();
        app.editor.play();
        app.editor.sync();
        assert!(app.terrain.apply_sculpt_stamp(&app.editor).is_err());
        app.editor.stop();
        app.editor.sync();
    }
    assert_eq!(bytes(&h), before);
    assert_eq!(h.state().terrain.document.revision(), revision);
    assert_eq!(h.state().terrain.document.dirty_region(), region);
    assert!(!h.state().terrain.document.dirty());
    assert!(!h.state().terrain.document.can_undo());
    assert!(h.state().terrain.document.can_redo());
    assert!(Arc::ptr_eq(
        &model,
        h.state().terrain.document.model().unwrap()
    ));
    assert_eq!(
        h.state().terrain.document.surface().unwrap().height,
        FP::ZERO
    );
    assert_eq!(
        h.state().terrain.document.marker().unwrap().anchor()[1],
        FP::ZERO
    );
    click(&mut h, "Terrain Redo");
    assert_eq!(bytes(&h), raised);
    let other = root.join("other.scene.yaml");
    std::fs::write(&other, FIXTURE).unwrap();
    {
        let app = h.state_mut();
        assert!(app.editor.open_path(&other));
        app.editor.sync();
        app.terrain.sync_for_editor(&app.editor);
        assert!(app.terrain.apply_sculpt_stamp(&app.editor).is_err());
        assert_eq!(app.terrain.document.bytes().unwrap(), raised);
        assert!(app.terrain.viewport_model(&app.editor).is_none());
    }
}

#[test]
fn installed_package_stays_read_only_until_copied_to_scene() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let project = root.join("project");
    let source = root.join("source");
    let installed = orr_terrain_view::package::install_and_reload(
        &orr_terrain_view::fixture(),
        &project,
        &source,
        "1.0.0",
    )
    .unwrap();
    let package_file = project
        .join(".orr/packages/objects")
        .join(installed.package_digest)
        .join("lab.orrt");
    let original = std::fs::read(&package_file).unwrap();
    let mut h = setup(&root);
    click(&mut h, "Terrain authoring");
    {
        let app = h.state_mut();
        app.terrain.project = project.to_str().unwrap().into();
        app.terrain.package = "heightfield-lab".into();
        app.terrain.asset = "lab.orrt".into();
        app.terrain.open_package_for_editor(&app.editor).unwrap();
    }
    h.step(); // Draw the admitted package controls before querying accessibility.
    click(&mut h, "Vertex");
    click(&mut h, "Terrain sculpt stamp");
    click(&mut h, "Flatten");
    let draft = h.state().terrain.sculpt_height.clone();
    assert!(h
        .get_by_label("Sample selected vertex height")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("Sample selected vertex height")
        .click_accesskit();
    h.step();
    assert_eq!(h.state().terrain.sculpt_height, draft);
    assert_eq!(bytes(&h), original);
    click(&mut h, "Raise/lower");
    assert!(h
        .get_by_label("Apply sculpt stamp")
        .accesskit_node()
        .is_disabled());
    {
        let app = h.state_mut();
        assert!(app.terrain.apply_sculpt_stamp(&app.editor).is_err());
        assert_eq!(app.terrain.document.bytes().unwrap(), original);
        app.terrain.local_path = "copied.orrt".into();
    }
    click(&mut h, "Copy terrain to scene");
    click(&mut h, "Apply sculpt stamp");
    assert!(
        h.state().terrain.error().is_none(),
        "{:?}",
        h.state().terrain.error()
    );
    assert_ne!(bytes(&h), original);
    click(&mut h, "Save terrain");
    assert_eq!(std::fs::read(root.join("copied.orrt")).unwrap(), bytes(&h));
    assert_eq!(std::fs::read(package_file).unwrap(), original);
    assert!(h.state().editor.history().entries.is_empty());
    assert_eq!(h.state().terrain.sculpt_mode, SculptMode::RaiseLower);
}
