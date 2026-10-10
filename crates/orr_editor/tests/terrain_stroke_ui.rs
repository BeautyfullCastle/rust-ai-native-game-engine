//! Actual EditorApp pointer ownership, frozen preview and cancellation contracts.
#![cfg(feature = "terrain")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{game::EditorGame, terrain_document::TerrainSession, Editor, EditorApp};
use orr_fp::FP;
use orr_terrain::Edit;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
const FIXTURE: &str = include_str!("../../../scenes/yard3d_authoring.scene.yaml");
fn setup(root: &Path, side: u32, create: bool) -> Harness<'static, EditorApp> {
    let scene = root.join("yard.scene.yaml");
    std::fs::write(&scene, FIXTURE).unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::Yard3D).unwrap();
    editor.sync();
    editor.camera3d =
        orr_render::OrbitCamera::new([0.0; 3], 0.0, 1.1, if side > 9 { 180.0 } else { 16.0 });
    let mut h = Harness::builder()
        .with_size([1280.0, 1000.0])
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |_| EditorApp::new(editor, None));
    h.state_mut().terrain.dimensions = [side, side];
    h.state_mut().terrain.origin = [format!("-{}", side / 2), format!("-{}", side / 2)];
    h.state_mut().terrain.sculpt_radius = 1;
    h.state_mut().terrain.sculpt_delta = "0.5".into();
    click(&mut h, "Terrain authoring");
    if create {
        click(&mut h, "Create terrain");
        click(&mut h, "Save terrain");
        click(&mut h, "Sculpt");
    }
    h
}
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(3);
    h.get_by_label(label).click();
    h.run_steps(3);
    h.state_mut().editor.sync();
    h.run_steps(2);
}
fn world_point(h: &Harness<'_, EditorApp>, world: [f32; 3]) -> Pos2 {
    let rect = h.state().ui.viewport_rect.unwrap();
    let size = h.state().ui.viewport_px;
    let at = h
        .state()
        .editor
        .camera3d
        .camera()
        .world_to_screen(world, size)
        .unwrap();
    let at = rect.min
        + egui::vec2(
            at[0] * rect.width() / size.0 as f32,
            at[1] * rect.height() / size.1 as f32,
        );
    assert!(rect.contains(at));
    at
}
fn vertex(h: &Harness<'_, EditorApp>, [x, z]: [u32; 2]) -> Pos2 {
    let terrain = h.state().terrain.document.sculpt_pick_terrain().unwrap();
    world_point(
        h,
        terrain
            .vertex_position(z * terrain.width() + x)
            .unwrap()
            .map(FP::to_f32),
    )
}
fn move_to(h: &mut Harness<'_, EditorApp>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.step();
}
fn button(h: &mut Harness<'_, EditorApp>, at: Pos2, button: PointerButton, pressed: bool) {
    h.event(Event::PointerButton {
        pos: at,
        button,
        pressed,
        modifiers: Modifiers::NONE,
    });
    h.step();
}
fn press(h: &mut Harness<'_, EditorApp>, at: Pos2) {
    move_to(h, at);
    button(h, at, PointerButton::Primary, true);
    assert!(
        h.state().terrain.document.stroke_active(),
        "{:?}",
        h.state().terrain.error()
    );
}
fn bytes(h: &Harness<'_, EditorApp>) -> Vec<u8> {
    h.state().terrain.document.bytes().unwrap().to_vec()
}
fn footprint_positions(shapes: &[egui::epaint::ClippedShape]) -> Vec<Pos2> {
    fn collect(shape: &egui::epaint::Shape, out: &mut Vec<Pos2>) {
        match shape {
            egui::epaint::Shape::Circle(circle)
                if circle.fill == egui::Color32::from_rgb(110, 220, 240)
                    && circle.radius == 2.5 =>
            {
                out.push(circle.center)
            }
            egui::epaint::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in shapes {
        collect(&shape.shape, &mut out);
    }
    out
}
fn assert_hover(h: &Harness<'_, EditorApp>, center: [u32; 2]) {
    let expected = orr_editor::terrain_document::brush::footprint(
        h.state().terrain.document.terrain().unwrap(),
        center,
        h.state().terrain.sculpt_radius,
    )
    .unwrap();
    let actual = footprint_positions(&h.output().shapes);
    assert_eq!(actual.len(), expected.len(), "hover footprint vertex count");
    for v in expected {
        let at = vertex(h, v);
        assert!(
            actual.iter().any(|p| p.distance(at) < 0.05),
            "missing projected vertex {v:?} at {at:?}; {actual:?}"
        );
    }
}
fn idle_hover_is_transient_and_matches_proposed_stroke() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = setup(dir.path(), 9, true);
    with_redo(&mut h);
    h.state_mut()
        .terrain
        .document
        .apply(&[Edit::SetHole {
            x: 6,
            z: 6,
            hole: true,
        }])
        .unwrap();
    h.state_mut()
        .terrain
        .document
        .apply(&[Edit::SetHeight {
            x: 8,
            z: 0,
            height: FP::ONE,
        }])
        .unwrap();
    h.state_mut().terrain.document.undo().unwrap();
    assert!(h.state().terrain.document.can_redo());
    let proof = Proof::of(&h.state().terrain.document);
    let selected = h.state().terrain.selected_vertex;
    let drafts = (
        h.state().terrain.height.clone(),
        h.state().terrain.sculpt_delta.clone(),
        h.state().terrain.sculpt_height.clone(),
    );
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().clone();
    for center in [[2, 2], [4, 4], [1, 5]] {
        let at = vertex(&h, center);
        move_to(&mut h, at);
        assert_hover(&h, center);
        proof.assert_restored(&h.state().terrain.document);
        assert_eq!(h.state().terrain.selected_vertex, selected);
        assert_eq!(
            (
                h.state().terrain.height.clone(),
                h.state().terrain.sculpt_delta.clone(),
                h.state().terrain.sculpt_height.clone()
            ),
            drafts
        );
    }
    // The hole's interior cannot preview a stamp even though its hidden vertices exist.
    let hole = world_point(&h, [2.25, 0.0, 2.25]);
    move_to(&mut h, hole);
    assert!(footprint_positions(&h.output().shapes).is_empty());
    proof.assert_restored(&h.state().terrain.document);
    move_to(&mut h, Pos2::new(1.0, 1.0));
    assert!(footprint_positions(&h.output().shapes).is_empty());
    let at = vertex(&h, [2, 2]);
    move_to(&mut h, at);
    assert_hover(&h, [2, 2]);
    h.event(Event::PointerGone);
    h.step();
    assert!(footprint_positions(&h.output().shapes).is_empty());
    proof.assert_restored(&h.state().terrain.document);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    // A missing current render frame clears the preview even if the pointer is valid.
    move_to(&mut h, at);
    assert_hover(&h, [2, 2]);
    let ctx = egui::Context::default();
    let camera = h.state().editor.camera3d.camera();
    let mut pointer = None;
    for (pass, frame_available) in [true, true, false].into_iter().enumerate() {
        let mut input = egui::RawInput::default();
        input.events.extend(pointer.map(Event::PointerMoved));
        let output = ctx.run_ui(input, |root| {
            egui::CentralPanel::default().show(root, |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(710.0, 904.0), egui::Sense::click_and_drag());
                let pixel = camera
                    .world_to_screen([-2.0, 0.0, -2.0], (710, 904))
                    .unwrap();
                pointer = Some(rect.min + egui::vec2(pixel[0], pixel[1]));
                let app = h.state_mut();
                app.terrain.sculpt_pointer(
                    &app.editor,
                    &response,
                    &camera,
                    rect,
                    (710, 904),
                    frame_available,
                );
                app.terrain
                    .paint_selection(&app.editor, ui.painter(), &camera, rect);
            });
        });
        if pass == 1 {
            assert_eq!(footprint_positions(&output.shapes).len(), 5);
        }
        if !frame_available {
            assert!(footprint_positions(&output.shapes).is_empty());
        }
    }
    proof.assert_restored(&h.state().terrain.document);
    for guard in [
        "escape",
        "focus",
        "modal",
        "play",
        "preview",
        "scene",
        "selection",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut h = setup(dir.path(), 9, true);
        with_redo(&mut h);
        h.state_mut()
            .terrain
            .document
            .apply(&[Edit::SetHeight {
                x: 0,
                z: 1,
                height: FP::ONE,
            }])
            .unwrap();
        let proof = Proof::of(&h.state().terrain.document);
        let selected = h.state().terrain.selected_vertex;
        let at = vertex(&h, [2, 2]);
        move_to(&mut h, at);
        assert_hover(&h, [2, 2]);
        match guard {
            "escape" => h.key_down(Key::Escape),
            "focus" => {
                h.input_mut().focused = false;
                h.event(Event::WindowFocused(false));
            }
            "modal" => {
                h.state_mut().ui.dialog = Some(orr_editor::app::Dialog {
                    kind: orr_editor::app::DialogKind::Open,
                    text: String::new(),
                })
            }
            "play" => {
                h.state_mut().editor.play();
                h.state_mut().editor.sync();
            }
            "preview" => {
                let proposal = h
                    .state_mut()
                    .editor
                    .agent_client("idle-hover-preview")
                    .unwrap()
                    .call(
                        "proposal.begin",
                        serde_json::json!({"label":"idle hover guard"}),
                    )
                    .unwrap()["id"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                h.state_mut().editor.sync();
                assert!(h.state_mut().editor.set_preview(Some(proposal)));
                h.state_mut().editor.sync();
            }
            "scene" => {
                let path = dir.path().join("other.scene.yaml");
                std::fs::write(&path, FIXTURE).unwrap();
                assert!(h.state_mut().editor.open_path(&path));
                h.state_mut().editor.sync();
            }
            "selection" => {
                h.state_mut().terrain.selection_mode =
                    orr_editor::terrain_pick::TerrainSelectionMode::Vertex
            }
            _ => unreachable!(),
        }
        h.step();
        assert!(
            footprint_positions(&h.output().shapes).is_empty(),
            "idle guard {guard}"
        );
        proof.assert_restored(&h.state().terrain.document);
        assert_eq!(h.state().terrain.selected_vertex, selected);
        if guard == "escape" {
            // Observe cancellation on key-down, then normal hover on key-up.
            // Harness::step runs one frame per queued event.
            h.key_up(Key::Escape);
            h.step();
            assert_hover(&h, [2, 2]);
            proof.assert_restored(&h.state().terrain.document);
        }
    }
}

struct Proof {
    bytes: Vec<u8>,
    revision: [u8; 32],
    dirty: bool,
    undo: bool,
    redo: bool,
    scene: PathBuf,
    query_and_region: String,
    model: Arc<orr_model::StaticModel>,
}
impl Proof {
    fn of(s: &TerrainSession) -> Self {
        Self {
            bytes: s.bytes().unwrap().to_vec(),
            revision: s.revision().unwrap(),
            dirty: s.dirty(),
            undo: s.can_undo(),
            redo: s.can_redo(),
            scene: s.scene().unwrap().to_path_buf(),
            query_and_region: format!(
                "{:?}",
                (s.query(), s.surface(), s.marker(), s.dirty_region())
            ),
            model: s.model().unwrap().clone(),
        }
    }
    fn assert_restored(&self, s: &TerrainSession) {
        assert!(!s.stroke_active());
        assert_eq!(s.bytes().unwrap(), self.bytes);
        assert_eq!(s.revision(), Some(self.revision));
        assert_eq!(
            (s.dirty(), s.can_undo(), s.can_redo()),
            (self.dirty, self.undo, self.redo)
        );
        assert_eq!(s.scene(), Some(self.scene.as_path()));
        assert_eq!(
            format!(
                "{:?}",
                (s.query(), s.surface(), s.marker(), s.dirty_region())
            ),
            self.query_and_region
        );
        assert!(Arc::ptr_eq(s.model().unwrap(), &self.model));
    }
}
fn with_redo(h: &mut Harness<'_, EditorApp>) -> Vec<u8> {
    let s = &mut h.state_mut().terrain.document;
    s.apply(&[Edit::SetHeight {
        x: 0,
        z: 0,
        height: FP::ONE,
    }])
    .unwrap();
    s.save().unwrap();
    s.apply(&[Edit::SetHeight {
        x: 1,
        z: 0,
        height: FP::from_int(2),
    }])
    .unwrap();
    let future = s.bytes().unwrap().to_vec();
    s.undo().unwrap();
    s.set_query(Some([FP::ZERO, FP::ZERO])).unwrap();
    future
}

#[test]
fn actual_drag_uses_frozen_path_and_commits_one_undo_save_reopen() {
    idle_hover_is_transient_and_matches_proposed_stroke();
    let dir = tempfile::tempdir().unwrap();
    let mut h = setup(dir.path(), 9, true);
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().clone();
    let original = bytes(&h);
    h.state_mut()
        .terrain
        .document
        .set_query(Some([FP::ZERO, FP::from_int(-2)]))
        .unwrap();
    let points = [[2, 2], [4, 2], [6, 2]].map(|p| vertex(&h, p));
    press(&mut h, points[0]);
    let first = bytes(&h);
    h.run_steps(8);
    assert_eq!(bytes(&h), first);
    h.state_mut().terrain.sculpt_delta = "9".into();
    h.state_mut().terrain.sculpt_radius = 3;
    move_to(&mut h, points[1]);
    assert_eq!(h.state().terrain.selected_vertex, [4, 2]);
    h.run_steps(8);
    assert_eq!(h.state().terrain.selected_vertex, [4, 2]);
    assert_eq!(
        h.state().terrain.document.surface().unwrap().height,
        FP::from_raw(32768)
    );
    button(&mut h, points[2], PointerButton::Primary, false);
    assert!(!h.state().terrain.document.stroke_active());
    let edited = bytes(&h);
    let terrain = h.state().terrain.document.terrain().unwrap();
    for z in 0_u32..9 {
        for x in 0_u32..9 {
            let covered = (2_u32..=6).any(|cx| x.abs_diff(cx).pow(2) + z.abs_diff(2).pow(2) <= 1);
            assert_eq!(
                terrain.heights()[(z * 9 + x) as usize],
                if covered {
                    FP::from_raw(32768)
                } else {
                    FP::ZERO
                }
            );
        }
    }
    click(&mut h, "Terrain Undo");
    assert_eq!(bytes(&h), original);
    assert!(!h.state().terrain.document.can_undo());
    click(&mut h, "Terrain Redo");
    assert_eq!(bytes(&h), edited);
    click(&mut h, "Save terrain");
    click(&mut h, "Close terrain");
    click(&mut h, "Open terrain");
    assert_eq!(bytes(&h), edited);
    assert!(!h.state().terrain.document.dirty());
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
}
#[test]
fn escape_focus_outside_play_and_context_replacement_restore_exact_history() {
    for cancellation in [
        "escape",
        "focus",
        "outside",
        "play",
        "scene",
        "modal",
        "selection",
        "preview",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut h = setup(dir.path(), 9, true);
        let future = with_redo(&mut h);
        let proof = Proof::of(&h.state().terrain.document);
        let saved = std::fs::read(dir.path().join("terrain.orrt")).unwrap();
        let at = vertex(&h, [3, 3]);
        move_to(&mut h, at);
        assert_hover(&h, [3, 3]);
        proof.assert_restored(&h.state().terrain.document);
        press(&mut h, at);
        assert_ne!(bytes(&h), proof.bytes);
        match cancellation {
            "escape" => {
                h.key_press(Key::Escape);
                h.step();
            }
            "focus" => {
                h.input_mut().focused = false;
                h.event(Event::WindowFocused(false));
                h.step();
            }
            "outside" => move_to(&mut h, Pos2::new(1.0, 1.0)),
            "play" => {
                h.state_mut().editor.play();
                h.state_mut().editor.sync();
                h.step();
            }
            "scene" => {
                let path = dir.path().join("other.scene.yaml");
                std::fs::write(&path, FIXTURE).unwrap();
                assert!(h.state_mut().editor.open_path(&path));
                h.state_mut().editor.sync();
                h.step();
            }
            "preview" => {
                let proposal = h
                    .state_mut()
                    .editor
                    .agent_client("terrain-hover-preview")
                    .unwrap()
                    .call(
                        "proposal.begin",
                        serde_json::json!({"label":"hover preview guard"}),
                    )
                    .unwrap()["id"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                h.state_mut().editor.sync();
                assert!(h.state_mut().editor.set_preview(Some(proposal)));
                h.state_mut().editor.sync();
                h.step();
            }
            "modal" => {
                h.state_mut().ui.dialog = Some(orr_editor::app::Dialog {
                    kind: orr_editor::app::DialogKind::Open,
                    text: String::new(),
                });
                h.step();
            }
            "selection" => {
                h.state_mut().terrain.selection_mode =
                    orr_editor::terrain_pick::TerrainSelectionMode::Vertex;
                h.step();
            }
            _ => unreachable!(),
        }
        proof.assert_restored(&h.state().terrain.document);
        assert!(
            footprint_positions(&h.output().shapes).is_empty(),
            "stale hover after {cancellation}"
        );
        assert_eq!(
            std::fs::read(dir.path().join("terrain.orrt")).unwrap(),
            saved
        );
        h.state_mut().terrain.document.redo().unwrap();
        assert_eq!(bytes(&h), future);
    }
}
#[test]
fn sparse_pointer_hole_crossing_and_vertex_limit_restore_the_entire_stroke() {
    for side in [9, 129] {
        let dir = tempfile::tempdir().unwrap();
        let mut h = setup(dir.path(), side, true);
        with_redo(&mut h);
        let (start, end) = if side == 9 {
            h.state_mut()
                .terrain
                .document
                .apply(&[Edit::SetHole {
                    x: 4,
                    z: 2,
                    hole: true,
                }])
                .unwrap();
            ([2, 2], [6, 2])
        } else {
            h.state_mut().terrain.sculpt_radius = 32;
            ([32, 64], [96, 64])
        };
        let proof = Proof::of(&h.state().terrain.document);
        let start = vertex(&h, start);
        let end = vertex(&h, end);
        press(&mut h, start);
        move_to(&mut h, end);
        proof.assert_restored(&h.state().terrain.document);
        let error = h.state().terrain.error().unwrap();
        assert!(
            error.contains(if side == 9 { "hole" } else { "limit" }),
            "{error}"
        );
        button(&mut h, end, PointerButton::Primary, false);
        proof.assert_restored(&h.state().terrain.document);
    }
}
#[test]
fn sculpt_preserves_camera_selection_ownership_and_installed_package_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = setup(dir.path(), 9, true);
    click(&mut h, "Entity");
    let at = world_point(&h, [2.0, 3.0, 0.0]);
    move_to(&mut h, at);
    button(&mut h, at, PointerButton::Primary, true);
    button(&mut h, at, PointerButton::Primary, false);
    let selected = h.state().editor.selection().cloned();
    assert!(selected.is_some());
    // The viewport selects after this frame's inspector has already been drawn.
    // Rebuild that inspector before resolving any of its accessibility IDs.
    h.state_mut().editor.sync();
    h.run_steps(3);
    // Selecting an entity can reflow the inspector while it scrolls. Address
    // this mode widget by identity; the viewport pick below still uses real
    // pointer press/release events.
    h.get_by_label("Vertex").click_accesskit();
    h.run_steps(3);
    assert_eq!(
        h.state().terrain.selection_mode,
        orr_editor::terrain_pick::TerrainSelectionMode::Vertex,
    );
    let at = vertex(&h, [2, 2]);
    move_to(&mut h, at);
    button(&mut h, at, PointerButton::Primary, true);
    button(&mut h, at, PointerButton::Primary, false);
    assert_eq!(
        h.state().terrain.selected_vertex,
        [2, 2],
        "mode={:?}, rect={:?}, pixel={:?}, camera={:?}, error={:?}",
        h.state().terrain.selection_mode,
        h.state().ui.viewport_rect,
        at,
        h.state().editor.camera3d.camera(),
        h.state().terrain.error(),
    );
    assert_eq!(h.state().editor.selection(), selected.as_ref());
    click(&mut h, "Sculpt");
    let before = bytes(&h);
    let yaw = h.state().editor.camera3d.yaw;
    let at = h.state().ui.viewport_rect.unwrap().center();
    move_to(&mut h, at);
    button(&mut h, at, PointerButton::Secondary, true);
    move_to(&mut h, at + egui::vec2(40.0, 10.0));
    button(
        &mut h,
        at + egui::vec2(40.0, 10.0),
        PointerButton::Secondary,
        false,
    );
    assert_ne!(h.state().editor.camera3d.yaw, yaw);
    assert_eq!(bytes(&h), before);
    assert!(!h.state().terrain.document.stroke_active());
    let project = dir.path().join("project");
    let source = dir.path().join("source");
    let installed = orr_terrain_view::package::install_and_reload(
        &orr_terrain_view::fixture(),
        &project,
        &source,
        "1.0.0",
    )
    .unwrap();
    let package = project
        .join(".orr/packages/objects")
        .join(installed.package_digest)
        .join("lab.orrt");
    let package_bytes = std::fs::read(&package).unwrap();
    click(&mut h, "Close terrain");
    {
        let app = h.state_mut();
        app.terrain.project = project.to_str().unwrap().into();
        app.terrain.package = "heightfield-lab".into();
        app.terrain.asset = "lab.orrt".into();
        app.terrain.open_package_for_editor(&app.editor).unwrap();
    }
    h.step();
    click(&mut h, "Sculpt");
    let proof = Proof::of(&h.state().terrain.document);
    let at = vertex(&h, [2, 2]);
    move_to(&mut h, at);
    assert!(footprint_positions(&h.output().shapes).is_empty());
    button(&mut h, at, PointerButton::Primary, true);
    button(&mut h, at, PointerButton::Primary, false);
    proof.assert_restored(&h.state().terrain.document);
    assert!(h.state().terrain.error().unwrap().contains("read-only"));
    assert_eq!(std::fs::read(package).unwrap(), package_bytes);
}

#[test]
fn batched_and_stepped_pointer_paths_share_commit_and_cancellation_results() {
    for path in ["valid", "outside", "hole", "press-release"] {
        let mut outcomes = Vec::new();
        for batched in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut h = setup(dir.path(), 9, true);
            with_redo(&mut h);
            if path == "hole" {
                h.state_mut()
                    .terrain
                    .document
                    .apply(&[Edit::SetHole {
                        x: 4,
                        z: 2,
                        hole: true,
                    }])
                    .unwrap();
            }
            let proof = Proof::of(&h.state().terrain.document);
            let start = vertex(&h, [2, 2]);
            let middle_vertex = if path == "valid" { [2, 6] } else { [4, 2] };
            let end_vertex = if path == "valid" { [6, 6] } else { [6, 2] };
            let middle = if path == "outside" {
                Pos2::new(1.0, 1.0)
            } else {
                vertex(&h, middle_vertex)
            };
            let end = vertex(&h, end_vertex);
            let mut events = Vec::new();
            if path == "press-release" {
                events.push(Event::PointerMoved(start));
                events.push(Event::PointerButton {
                    pos: start,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                });
            } else {
                press(&mut h, start);
            }
            events.extend([
                Event::PointerMoved(middle),
                Event::PointerMoved(end),
                Event::PointerButton {
                    pos: end,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                },
            ]);
            if batched {
                // Harness::event/step dispatches one frame per queued event.
                // RawInput is required to exercise a real single-frame batch.
                h.input_mut().events.extend(events);
                h.step();
            } else {
                for event in events {
                    h.event(event);
                    h.step();
                }
            }
            let s = &h.state().terrain.document;
            assert!(!s.stroke_active());
            if path == "outside" || path == "hole" {
                proof.assert_restored(s);
            } else {
                assert_ne!(s.bytes().unwrap(), proof.bytes);
                assert_eq!(
                    s.terrain().unwrap().heights()[2 * 9 + 2],
                    FP::from_raw(32768),
                    "the press center must participate even in a single-frame gesture"
                );
                for [x, z] in [middle_vertex, end_vertex] {
                    assert_eq!(
                        s.terrain().unwrap().heights()[(z * 9 + x) as usize],
                        FP::from_raw(32768)
                    );
                }
                if path == "valid" {
                    assert_eq!(
                        s.terrain().unwrap().heights()[4 * 9 + 4],
                        FP::ZERO,
                        "the corner sample must prevent a diagonal shortcut"
                    );
                }
            }
            outcomes.push((
                s.bytes().unwrap().to_vec(),
                s.dirty(),
                s.can_undo(),
                s.can_redo(),
            ));
        }
        assert_eq!(
            outcomes[0], outcomes[1],
            "event batching changed the {path} path"
        );
    }
    // Idle admission obeys focus and camera ownership even when a primary
    // press/release arrives in one frame with no new camera-button event.
    for blocked in [
        "secondary",
        "middle",
        "secondary-release",
        "middle-release",
        "unfocused",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut h = setup(dir.path(), 9, true);
        with_redo(&mut h);
        let proof = Proof::of(&h.state().terrain.document);
        let at = vertex(&h, [3, 3]);
        move_to(&mut h, at);
        match blocked {
            "secondary" | "secondary-release" => button(&mut h, at, PointerButton::Secondary, true),
            "middle" | "middle-release" => button(&mut h, at, PointerButton::Middle, true),
            "unfocused" => h.input_mut().focused = false,
            _ => unreachable!(),
        }
        for pressed in [true, false] {
            h.input_mut().events.push(Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
        if blocked.ends_with("-release") {
            h.input_mut().events.push(Event::PointerButton {
                pos: at,
                button: if blocked.starts_with("secondary") {
                    PointerButton::Secondary
                } else {
                    PointerButton::Middle
                },
                pressed: false,
                modifiers: Modifiers::NONE,
            });
        }
        h.step();
        proof.assert_restored(&h.state().terrain.document);
    }
    // A frame-wide interruption of an already held stroke cannot be revived by
    // earlier queued press/move/release events after the prepass restored it.
    let dir = tempfile::tempdir().unwrap();
    let mut h = setup(dir.path(), 9, true);
    with_redo(&mut h);
    let proof = Proof::of(&h.state().terrain.document);
    let start = vertex(&h, [2, 2]);
    let end = vertex(&h, [6, 2]);
    press(&mut h, start);
    h.input_mut().focused = false;
    h.input_mut().events.push(Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.input_mut().events.push(Event::PointerMoved(end));
    h.input_mut().events.push(Event::PointerButton {
        pos: end,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    proof.assert_restored(&h.state().terrain.document);

    // A lost-and-restored focus batch must fence the entire frame even though
    // its final focus state is true and a fresh press/release follows the loss.
    for held in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut h = setup(dir.path(), 9, true);
        with_redo(&mut h);
        let proof = Proof::of(&h.state().terrain.document);
        let start = vertex(&h, [2, 2]);
        let end = vertex(&h, [6, 2]);
        if held {
            press(&mut h, start);
        }
        h.input_mut().focused = true;
        h.input_mut().events.push(Event::WindowFocused(false));
        h.input_mut().events.push(Event::WindowFocused(true));
        h.input_mut().events.push(Event::PointerMoved(end));
        for pressed in [true, false] {
            h.input_mut().events.push(Event::PointerButton {
                pos: end,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
        h.step();
        assert!(h.ctx.input(|i| i.focused));
        proof.assert_restored(&h.state().terrain.document);
    }
}
