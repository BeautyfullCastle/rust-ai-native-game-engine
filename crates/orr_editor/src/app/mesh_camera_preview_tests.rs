//! CPU App camera selection and viewport dispatch. No GPU or OS input is used.
#![cfg(feature = "mesh-mission")]
use super::*;
use crate::{
    editor::Editor,
    game::EditorGame,
    mesh_camera::{Document, Projection},
};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use std::path::PathBuf;

const SIZE: (u32, u32) = (800, 600);
fn games() -> Vec<EditorGame> {
    let games = vec![EditorGame::MeshNavigation];
    #[cfg(feature = "mesh-hazards")]
    let games = {
        let mut games = games;
        games.push(EditorGame::MeshHazardsV1);
        games
    };
    games
}
fn coherent(editor: &mut Editor) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        editor.sync();
        if editor.can_mutate() && editor.yard_rows_coherent() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "incoherent Mesh fixture"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    scene: PathBuf,
    camera_file: PathBuf,
    light_file: PathBuf,
    app: EditorApp,
    ctx: egui::Context,
    checksum: u64,
    history: crate::model::History,
    files: [Vec<u8>; 3],
}
impl Fixture {
    fn new(game: EditorGame) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let scene = temp.path().join("camera-preview.scene.yaml");
        let editor = crate::mesh_workspace::mission::prepare(
            &scene,
            &crate::mesh_workspace::mission::Settings::default(),
        )
        .unwrap();
        #[cfg(feature = "mesh-hazards")]
        let editor = if game.is_mesh_hazards() {
            let yaml = std::fs::read_to_string(&scene).unwrap();
            let mut doc =
                orr_remote::mesh_navigation::hazards::hazards_doc_from_yaml(&yaml).unwrap();
            std::fs::write(&scene, doc.save_yaml()).unwrap();
            Editor::open_game(&scene, game).unwrap()
        } else {
            editor
        };
        let mut editor = editor;
        coherent(&mut editor);
        assert_eq!(editor.game(), game);
        let revision =
            orr_remote::mesh_navigation::mesh_from_view(editor.snapshot().unwrap().predicted())
                .unwrap()
                .graph
                .revision();
        let camera = Document::from_orbit(editor.camera3d).unwrap();
        crate::mesh_camera_store::save(&scene, Some(&camera), None, revision).unwrap();
        let mut light =
            crate::mesh_lighting::Document::from_lighting(orr_sample::yard3d_view::yard_lighting())
                .unwrap();
        light.point_light = Some(orr_render::PointLight {
            position: [0.0, 4.0, 0.0],
            color: [1.0; 3],
            intensity: 3.0,
            range: 12.0,
        });
        crate::mesh_lighting_store::save(&scene, Some(&light), None, revision).unwrap();
        let camera_file = crate::mesh_camera_store::sidecar_path(&scene).unwrap();
        let light_file = crate::mesh_lighting_store::sidecar_path(&scene).unwrap();
        let files = [
            std::fs::read(&scene).unwrap(),
            std::fs::read(&camera_file).unwrap(),
            std::fs::read(&light_file).unwrap(),
        ];
        let checksum = editor.checksum();
        let history = editor.history().clone();
        let mut app = EditorApp::new(editor, None);
        app.mesh_camera.sync(&mut app.editor);
        app.mesh_lighting.sync(&app.editor);
        Self {
            _temp: temp,
            scene,
            camera_file,
            light_file,
            app,
            ctx: egui::Context::default(),
            checksum,
            history,
            files,
        }
    }
    fn preview(&mut self, enabled: bool) {
        self.app
            .mesh_camera
            .set_static_preview(&self.app.editor, enabled)
            .unwrap();
    }
    fn unchanged(&self) {
        assert_eq!(self.app.editor.checksum(), self.checksum);
        assert_eq!(self.app.editor.history(), &self.history);
        self.files_unchanged();
    }
    fn files_unchanged(&self) {
        assert_eq!(std::fs::read(&self.scene).unwrap(), self.files[0]);
        assert_eq!(std::fs::read(&self.camera_file).unwrap(), self.files[1]);
        assert_eq!(std::fs::read(&self.light_file).unwrap(), self.files[2]);
    }
    fn step(&mut self, events: Vec<egui::Event>, viewport: bool) {
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                focused: true,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                self.app.sync_mesh_light_pick(ui.ctx());
                self.app.sync_mesh_camera_preview(ui.ctx());
                self.app.shortcuts(ui.ctx());
                if viewport {
                    self.app.yard_viewport(ui);
                }
            },
        );
        output.textures_delta.clear();
    }
    fn scrolling(&self) -> bool {
        self.ctx
            .input(|i| i.is_scrolling() || i.smooth_scroll_delta != egui::Vec2::ZERO)
    }
}
fn harness(f: Fixture) -> Harness<'static, Fixture> {
    let mut h = Harness::builder()
        .with_size([1400.0, 2800.0])
        .build_ui_state(
            |ui, f: &mut Fixture| {
                f.app.sync_mesh_camera_preview(ui.ctx());
                f.app.mesh_camera.show(ui, &mut f.app.editor);
            },
            f,
        );
    h.run_steps(3);
    h.get_by_label("Mesh game camera").click_accesskit();
    h.run_steps(3);
    h
}
fn text(h: &mut Harness<'_, Fixture>, label: &str, value: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, label)
        .focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, label)
        .type_text(value);
    h.run_steps(3);
}
fn draft(h: &mut Harness<'_, Fixture>, orthographic: bool) -> Document {
    for (label, value) in [
        ("Game camera target X", "1"),
        ("Game camera target Y", "3"),
        ("Game camera target Z", "-2"),
        ("Game camera yaw (degrees)", "35"),
        ("Game camera pitch (degrees)", "25"),
        ("Game camera distance", "21"),
        ("Game camera field of view (degrees)", "65"),
        ("Game camera near plane", "0.25"),
        ("Game camera far plane", "250"),
        ("Game camera orthographic half height", "5"),
    ] {
        text(h, label, value);
    }
    if orthographic {
        h.get_by_label("Orthographic game camera").click_accesskit();
        h.run_steps(3);
    }
    let doc = Document {
        target: [1.0, 3.0, -2.0],
        follow_offset: None,
        yaw: 35.0_f32.to_radians(),
        pitch: 25.0_f32.to_radians(),
        distance: 21.0,
        fov_y_degrees: 65.0,
        projection: if orthographic {
            Projection::Orthographic { half_height: 5.0 }
        } else {
            Projection::Perspective
        },
        near: 0.25,
        far: 250.0,
    };
    doc.camera(&doc.orbit(), SIZE).unwrap();
    doc
}
fn pos() -> egui::Pos2 {
    egui::pos2(300.0, 300.0)
}
fn button(button: egui::PointerButton, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: pos(),
        button,
        pressed,
        modifiers: Default::default(),
    }
}
fn wheel() -> egui::Event {
    egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, 120.0),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
    }
}
fn orbit(f: &Fixture) -> orr_render::OrbitCamera {
    f.app.editor.camera3d
}

#[test]
fn static_renderer_selection_matches_validated_perspective_and_orthographic_drafts_at_two_sizes() {
    for game in games() {
        for orthographic in [false, true] {
            let mut h = harness(Fixture::new(game));
            let doc = draft(&mut h, orthographic);
            let f = h.state_mut();
            let before_orbit = orbit(f);
            let dirty = f.app.mesh_camera.dirty();
            assert!(dirty);
            assert!(!f.app.mesh_camera.static_preview_enabled());
            for size in [SIZE, (300, 900)] {
                assert_eq!(
                    f.app.mesh_camera_for_frame(size).unwrap(),
                    f.app.editor.presentation_camera3d(size).unwrap()
                );
            }
            f.preview(true);
            for size in [SIZE, (300, 900)] {
                assert_eq!(
                    f.app.mesh_camera_for_frame(size).unwrap(),
                    doc.camera(&doc.orbit(), size).unwrap()
                );
            }
            assert_eq!(orbit(f), before_orbit);
            assert_eq!(f.app.mesh_camera.dirty(), dirty);
            f.unchanged();
            f.preview(false);
            for size in [SIZE, (300, 900)] {
                assert_eq!(
                    f.app.mesh_camera_for_frame(size).unwrap(),
                    f.app.editor.presentation_camera3d(size).unwrap()
                );
            }
            assert_eq!(orbit(f), before_orbit);
            f.unchanged();
            f.preview(true);
            let mut editor = Editor::open_game(&f.scene, game).unwrap();
            coherent(&mut editor);
            let mut restarted = EditorApp::new(editor, None);
            restarted.mesh_camera.sync(&mut restarted.editor);
            assert!(!restarted.mesh_camera.static_preview_enabled());
            assert_eq!(
                restarted.mesh_camera_for_frame(SIZE).unwrap(),
                restarted.editor.presentation_camera3d(SIZE).unwrap()
            );
            f.unchanged();
        }
    }
}

#[test]
fn follow_invalid_and_stale_observations_retire_static_preview_without_revival() {
    for game in games() {
        for cause in ["follow", "invalid", "scene", "focus", "mode"] {
            let mut h = harness(Fixture::new(game));
            let doc = draft(&mut h, false);
            h.state_mut().preview(true);
            assert_eq!(
                h.state_mut().app.mesh_camera_for_frame(SIZE).unwrap(),
                doc.camera(&doc.orbit(), SIZE).unwrap()
            );
            match cause {
                "follow" => {
                    h.get_by_label("Follow mesh agent").click_accesskit();
                    h.run_steps(3);
                }
                "invalid" => text(&mut h, "Game camera distance", "invalid"),
                "scene" => {
                    let f = h.state_mut();
                    assert!(f.app.editor.open_path(&f.scene));
                    coherent(&mut f.app.editor);
                    f.step(vec![], false);
                }
                "focus" => h.state_mut().step(
                    vec![
                        egui::Event::WindowFocused(false),
                        egui::Event::WindowFocused(true),
                    ],
                    false,
                ),
                "mode" => {
                    let f = h.state_mut();
                    f.app.editor.step(0);
                    f.app.editor.sync();
                    assert_eq!(f.app.editor.mode(), crate::model::Mode::Play);
                    f.step(vec![], false);
                }
                _ => unreachable!(),
            }
            let f = h.state_mut();
            assert!(!f.app.mesh_camera.static_preview_enabled(), "{cause}");
            let checksum = f.app.editor.checksum();
            let history = f.app.editor.history().clone();
            assert_eq!(
                f.app.mesh_camera_for_frame(SIZE).unwrap(),
                f.app.editor.presentation_camera3d(SIZE).unwrap()
            );
            assert_eq!(f.app.editor.checksum(), checksum);
            assert_eq!(f.app.editor.history(), &history);
            match cause {
                "follow" => {
                    h.get_by_label("Follow mesh agent").click_accesskit();
                    h.run_steps(3);
                }
                "invalid" => text(&mut h, "Game camera distance", "21"),
                "mode" => {
                    let f = h.state_mut();
                    f.app.editor.stop();
                    coherent(&mut f.app.editor);
                }
                _ => {}
            }
            let f = h.state_mut();
            f.step(vec![], false);
            assert!(
                !f.app.mesh_camera.static_preview_enabled(),
                "repair cannot opt in: {cause}"
            );
            f.files_unchanged();
            if matches!(cause, "follow" | "invalid" | "focus") {
                f.unchanged();
            }
        }
    }
}

#[test]
fn active_preview_fences_actual_viewport_navigation_selection_pickers_and_capture() {
    for game in games() {
        let mut h = harness(Fixture::new(game));
        let doc = draft(&mut h, false);
        h.state_mut().preview(true);
        h.run_steps(3);
        assert!(h
            .get_by_label("Capture viewport as game camera")
            .accesskit_node()
            .is_disabled());
        let f = h.state_mut();
        let before_orbit = orbit(f);
        let selection = f.app.editor.selected_guid();
        f.step(vec![egui::Event::PointerMoved(pos())], true);
        for b in [
            egui::PointerButton::Secondary,
            egui::PointerButton::Middle,
            egui::PointerButton::Primary,
        ] {
            f.step(vec![button(b, true)], true);
            f.step(
                vec![
                    egui::Event::PointerMoved(pos() + egui::vec2(50.0, 30.0)),
                    wheel(),
                    egui::Event::Key {
                        key: egui::Key::F,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default(),
                    },
                ],
                true,
            );
            f.step(vec![button(b, false)], true);
            assert!(f.app.mesh_camera.static_preview_enabled());
            assert_eq!(
                f.app.mesh_camera_for_frame(SIZE).unwrap(),
                doc.camera(&doc.orbit(), SIZE).unwrap()
            );
            assert_eq!(orbit(f), before_orbit);
            assert_eq!(f.app.editor.selected_guid(), selection);
        }
        assert!(f
            .app
            .mesh_lighting
            .begin_viewport_pick(&f.app.editor)
            .is_err());
        assert!(!f.app.mesh_navigation.agent_pick_busy());
        assert!(!f.app.mesh_mission_pick_busy());
        assert!(!f.app.hazard_pick_busy());
        let capture_path = f._temp.path().join("refused-preview.png");
        let ctx = f.ctx.clone();
        // The ordinary per-frame screenshot hook with no job must leave a
        // preview alone. A real job is refused before requesting OS capture.
        f.app.screenshot(&ctx);
        assert!(f.app.mesh_camera.static_preview_enabled());
        f.app.shot = Some(crate::app::ScreenshotJob::new(capture_path.clone(), 0));
        f.app.screenshot(&ctx);
        assert!(f.app.shot.is_none());
        assert!(!capture_path.exists());
        assert!(f.app.mesh_camera.static_preview_enabled());
        assert_eq!(orbit(f), before_orbit);
        assert_eq!(
            f.app.mesh_camera_for_frame(SIZE).unwrap(),
            doc.camera(&doc.orbit(), SIZE).unwrap()
        );
        f.unchanged();
        // The consuming builder is another public admission path. Use an
        // owned fixture so moving the App does not require a test backdoor.
        let mut builder = Fixture::new(game);
        builder.preview(true);
        let before_orbit = orbit(&builder);
        let capture_path = builder._temp.path().join("refused-builder.png");
        builder.app = builder
            .app
            .with_screenshot(crate::app::ScreenshotJob::new(capture_path.clone(), 0));
        assert!(builder.app.shot.is_none());
        assert!(builder.app.mesh_camera.static_preview_enabled());
        assert!(!capture_path.exists());
        assert_eq!(orbit(&builder), before_orbit);
        builder.unchanged();
    }
}

#[test]
fn cancelled_held_drag_and_wheel_remain_quarantined_until_release_and_drain_then_navigation_works()
{
    for game in games() {
        for b in [
            egui::PointerButton::Primary,
            egui::PointerButton::Secondary,
            egui::PointerButton::Middle,
        ] {
            let mut h = harness(Fixture::new(game));
            draft(&mut h, false);
            let f = h.state_mut();
            f.preview(true);
            let before_orbit = orbit(f);
            f.step(vec![egui::Event::PointerMoved(pos())], true);
            f.step(vec![button(b, true), wheel()], true);
            f.preview(false);
            f.step(
                vec![egui::Event::PointerMoved(pos() + egui::vec2(40.0, 20.0))],
                true,
            );
            assert!(f.app.mesh_camera_preview_owns_input());
            assert!(f
                .app
                .mesh_camera
                .set_static_preview(&f.app.editor, true)
                .is_err());
            assert_eq!(orbit(f), before_orbit);
            f.step(vec![button(b, false)], true);
            assert!(
                f.app.mesh_camera_preview_owns_input(),
                "physical release frame stays owned"
            );
            f.step(vec![], true);
            assert!(
                !f.app.mesh_camera_preview_owns_input(),
                "quiet frame releases pointer fence"
            );
            assert!(f.scrolling(), "real smoothing remains after release");
            let mut drained = false;
            for _ in 0..240 {
                f.step(vec![], true);
                assert_eq!(
                    orbit(f),
                    before_orbit,
                    "cancelled smoothing cannot leak into orbit"
                );
                if !f.scrolling() {
                    drained = true;
                    break;
                }
            }
            assert!(drained);
            f.step(vec![], true);
            assert_eq!(orbit(f), before_orbit);
            f.unchanged();
            f.step(vec![egui::Event::PointerMoved(pos()), wheel()], true);
            for _ in 0..8 {
                if orbit(f) != before_orbit {
                    break;
                }
                f.step(vec![], true);
            }
            assert_ne!(orbit(f), before_orbit, "fresh wheel must zoom after drain");
            // Drain this intentional wheel too, so each drag's positive control
            // cannot pass merely because zoom smoothing still moves the orbit.
            let mut fresh_wheel_drained = false;
            for _ in 0..240 {
                f.step(vec![], true);
                if !f.scrolling() {
                    fresh_wheel_drained = true;
                    break;
                }
            }
            assert!(fresh_wheel_drained);
            for drag in [egui::PointerButton::Secondary, egui::PointerButton::Middle] {
                let before = orbit(f);
                f.step(vec![egui::Event::PointerMoved(pos())], true);
                f.step(vec![button(drag, true)], true);
                f.step(
                    vec![egui::Event::PointerMoved(pos() + egui::vec2(80.0, 50.0))],
                    true,
                );
                f.step(vec![button(drag, false)], true);
                assert_ne!(
                    orbit(f),
                    before,
                    "fresh {drag:?} drag must navigate after off"
                );
            }
            f.unchanged();
        }
    }
}
