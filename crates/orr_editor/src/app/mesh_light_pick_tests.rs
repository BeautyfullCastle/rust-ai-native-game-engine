//! CPU App adapters with real mission/Hazards scenes. Supplied cameras and
//! `frame_available=true` do not exercise rendered pixels, GPU, or physical input.
#![cfg(feature = "mesh-mission")]
use super::*;
use crate::{game::EditorGame, mesh_lighting::Document};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use std::path::PathBuf;

const SIZE: (u32, u32) = (800, 600);
fn rect() -> egui::Rect {
    egui::Rect::from_min_size(egui::pos2(80.0, 90.0), egui::vec2(400.0, 300.0))
}
fn camera() -> orr_render::Camera3D {
    orr_render::Camera3D::perspective([0.0, 10.0, 10.0], [0.0; 3], 60.0)
}
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
struct Fixture {
    _temp: tempfile::TempDir,
    scene: PathBuf,
    sidecar: PathBuf,
    app: EditorApp,
    ctx: egui::Context,
}
fn wait_for_coherent_editor(editor: &mut crate::editor::Editor) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        editor.sync();
        if editor.can_mutate() && editor.yard_rows_coherent() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Mesh fixture did not receive an editable coherent scene"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}
impl Fixture {
    fn new(game: EditorGame) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let scene = temp.path().join("light-pick.scene.yaml");
        let editor = crate::mesh_workspace::mission::prepare(
            &scene,
            &crate::mesh_workspace::mission::Settings::default(),
        )
        .unwrap();
        #[cfg(feature = "mesh-hazards")]
        let editor = if game.is_mesh_hazards() {
            let yaml = std::fs::read_to_string(&scene).unwrap();
            let mut hazards =
                orr_remote::mesh_navigation::hazards::hazards_doc_from_yaml(&yaml).unwrap();
            std::fs::write(&scene, hazards.save_yaml()).unwrap();
            let mut editor = crate::editor::Editor::open_game(&scene, game).unwrap();
            editor.sync();
            editor
        } else {
            editor
        };
        let mut editor = editor;
        wait_for_coherent_editor(&mut editor);
        assert_eq!(editor.game(), game);
        let revision =
            orr_remote::mesh_navigation::mesh_from_view(editor.snapshot().unwrap().predicted())
                .unwrap()
                .graph
                .revision();
        let mut document =
            Document::from_lighting(orr_sample::yard3d_view::yard_lighting()).unwrap();
        document.point_light = Some(orr_render::PointLight {
            position: [0.0, 4.0, 0.0],
            color: [1.0, 0.9, 0.7],
            intensity: 3.0,
            range: 12.0,
        });
        crate::mesh_lighting_store::save(&scene, Some(&document), None, revision).unwrap();
        let sidecar = crate::mesh_lighting_store::sidecar_path(&scene).unwrap();
        let mut app = EditorApp::new(editor, None);
        app.mesh_lighting.sync(&app.editor);
        Self {
            _temp: temp,
            scene,
            sidecar,
            app,
            ctx: egui::Context::default(),
        }
    }
    fn arm(&mut self) {
        self.step(vec![], None, false);
        self.app
            .mesh_lighting
            .begin_viewport_pick(&self.app.editor)
            .unwrap();
    }
    fn step(&mut self, events: Vec<egui::Event>, frame: Option<bool>, viewport: bool) -> bool {
        let mut consumed = false;
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
                self.app.shortcuts(ui.ctx());
                if viewport {
                    // Exercises real F/orbit/pan/wheel/selection/other-picker
                    // dispatch. Missing GPU is an expected final-frame failure.
                    self.app.yard_viewport(ui);
                } else if let Some(available) = frame {
                    let response = ui.allocate_rect(rect(), egui::Sense::click());
                    consumed = self.app.mesh_light_viewport_pick(
                        ui.ctx(),
                        &response,
                        &camera(),
                        SIZE,
                        available,
                    );
                }
            },
        );
        output.textures_delta.clear();
        consumed
    }
    fn click(&mut self, available: bool) -> bool {
        self.click_at(rect().center(), available)
    }
    fn click_at(&mut self, pos: egui::Pos2, available: bool) -> bool {
        // egui hit-tests the previous pass's widget tree. Install the synthetic
        // viewport response before pressing, including after a no-response frame.
        self.step(vec![egui::Event::PointerMoved(pos)], Some(available), false);
        self.step(vec![button_at(pos, true)], Some(available), false);
        self.step(vec![button_at(pos, false)], Some(available), false)
    }
}
fn button(pressed: bool) -> egui::Event {
    button_at(rect().center(), pressed)
}
fn button_at(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}
fn unchanged(f: &Fixture, checksum: u64, history: &crate::model::History, files: &[Vec<u8>; 2]) {
    assert_eq!(f.app.editor.checksum(), checksum);
    assert_eq!(f.app.editor.history(), history);
    assert_eq!(std::fs::read(&f.scene).unwrap(), files[0]);
    assert_eq!(std::fs::read(&f.sidecar).unwrap(), files[1]);
}

#[test]
fn final_frame_gate_and_failed_draw_cancel_before_draft_staging() {
    for game in games() {
        for available in [false, true] {
            let mut f = Fixture::new(game);
            let checksum = f.app.editor.checksum();
            let history = f.app.editor.history().clone();
            let files = [
                std::fs::read(&f.scene).unwrap(),
                std::fs::read(&f.sidecar).unwrap(),
            ];
            let applied = f.app.point_light_settings_for_frame().unwrap();
            f.arm();
            // The press cannot stage before a final successful frame exists.
            f.step(
                vec![egui::Event::PointerMoved(rect().center()), button(true)],
                Some(available),
                false,
            );
            assert!(!f.app.mesh_lighting.dirty());
            if available {
                assert!(f.app.mesh_light_pick_pending());
            } else {
                assert!(!f.app.mesh_light_pick_pending());
            }
            // An explicitly failed final draw always retires the ticket.
            f.step(vec![button(false)], Some(false), false);
            assert!(!f.app.mesh_light_pick_pending());
            assert!(!f.app.mesh_lighting.dirty());
            f.step(vec![], None, false);
            assert!(
                !f.click(true),
                "a later successful frame cannot revive the ticket"
            );
            assert_eq!(f.app.point_light_settings_for_frame().unwrap(), applied);
            unchanged(&f, checksum, &history, &files);
            // Positive control uses the same App click helper with a fresh ticket
            // and a projected, noncentral ground point in the offset viewport.
            f.arm();
            let pixel = camera().world_to_screen([2.0, 0.0, -1.0], SIZE).unwrap();
            let pos = rect().min
                + egui::vec2(
                    pixel[0] * rect().width() / SIZE.0 as f32,
                    pixel[1] * rect().height() / SIZE.1 as f32,
                );
            assert!(rect().contains(pos));
            assert!(
                f.click_at(pos, true),
                "fresh successful stage consumes click"
            );
            assert!(!f.app.mesh_light_pick_pending());
            assert!(f.app.mesh_lighting.dirty());
            assert_eq!(f.app.point_light_settings_for_frame().unwrap(), applied);
            unchanged(&f, checksum, &history, &files);
            let mut h = Harness::builder()
                .with_size([1400.0, 2800.0])
                .build_ui_state(
                    |ui, f: &mut Fixture| {
                        f.app.mesh_lighting.show_with_navigation(
                            ui,
                            &f.app.editor,
                            &f.app.mesh_navigation,
                        );
                    },
                    f,
                );
            h.run_steps(3);
            h.get_by_label("Mesh game lighting").click_accesskit();
            h.run_steps(3);
            h.get_by_label("Point light settings").click_accesskit();
            h.run_steps(3);
            let height = applied.point_light.unwrap().position[1];
            for (label, expected) in [
                ("Point light position X", 2.0),
                ("Point light position Y", height),
                ("Point light position Z", -1.0),
            ] {
                let actual: f32 = h
                    .get_by_label(label)
                    .accesskit_node()
                    .value()
                    .unwrap()
                    .replace('\u{2212}', "-")
                    .parse()
                    .unwrap();
                assert!((actual - expected).abs() < 0.002, "{label}: {actual}");
            }
            assert_eq!(
                h.state().app.point_light_settings_for_frame().unwrap(),
                applied
            );
            unchanged(h.state(), checksum, &history, &files);
        }
    }
}

#[test]
fn same_batch_focus_regain_press_fences_real_dispatch_until_release_and_quiet() {
    for game in games() {
        let mut f = Fixture::new(game);
        let checksum = f.app.editor.checksum();
        let history = f.app.editor.history().clone();
        let files = [
            std::fs::read(&f.scene).unwrap(),
            std::fs::read(&f.sidecar).unwrap(),
        ];
        let selection = f.app.editor.selected_guid();
        let before = f.app.editor.presentation_camera3d(SIZE).unwrap();
        f.arm();
        f.step(
            vec![
                egui::Event::WindowFocused(false),
                egui::Event::WindowFocused(true),
                button(true),
            ],
            None,
            true,
        );
        assert!(!f.app.mesh_light_pick_pending());
        assert!(f.app.mesh_light_pick_owns_input());
        for secondary in [egui::PointerButton::Secondary, egui::PointerButton::Middle] {
            f.step(
                vec![
                    egui::Event::Key {
                        key: egui::Key::F,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default(),
                    },
                    egui::Event::PointerButton {
                        pos: rect().center(),
                        button: secondary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                    egui::Event::PointerMoved(rect().center() + egui::vec2(40.0, 20.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, 120.0),
                        phase: egui::TouchPhase::Move,
                        modifiers: Default::default(),
                    },
                ],
                None,
                true,
            );
            assert!(f.app.mesh_light_pick_owns_input());
            assert!(
                f.app
                    .mesh_lighting
                    .begin_viewport_pick(&f.app.editor)
                    .is_err(),
                "cancelled-owned input must forbid rearm"
            );
            f.step(
                vec![egui::Event::PointerButton {
                    pos: rect().center(),
                    button: secondary,
                    pressed: false,
                    modifiers: Default::default(),
                }],
                None,
                true,
            );
        }
        assert_eq!(f.app.editor.presentation_camera3d(SIZE).unwrap(), before);
        assert_eq!(f.app.editor.selected_guid(), selection);
        assert!(!f.app.mesh_navigation.agent_pick_busy());
        assert!(!f.app.mesh_mission_pick_busy());
        assert!(!f.app.hazard_pick_busy());
        f.step(vec![button(false)], None, true);
        assert!(
            f.app.mesh_light_pick_owns_input(),
            "physical release frame remains owned"
        );
        assert!(f
            .app
            .mesh_lighting
            .begin_viewport_pick(&f.app.editor)
            .is_err());
        f.step(vec![], None, true);
        assert!(!f.app.mesh_light_pick_owns_input());
        f.app
            .mesh_lighting
            .begin_viewport_pick(&f.app.editor)
            .unwrap();
        f.app.cancel_mesh_light_pick();
        assert!(
            f.ctx.input(
                |input| input.is_scrolling() || input.smooth_scroll_delta != egui::Vec2::ZERO
            ),
            "owned wheel input must leave real smoothing to drain after release"
        );
        assert_eq!(f.app.editor.presentation_camera3d(SIZE).unwrap(), before);
        let mut drained = false;
        for _ in 0..240 {
            f.step(vec![], None, true);
            assert!(!f.app.mesh_light_pick_owns_input());
            assert_eq!(
                f.app.editor.presentation_camera3d(SIZE).unwrap(),
                before,
                "cancelled wheel smoothing cannot become viewport zoom"
            );
            if !f.ctx.input(|input| {
                input.is_scrolling() || input.smooth_scroll_delta != egui::Vec2::ZERO
            }) {
                drained = true;
                break;
            }
        }
        assert!(drained, "wheel smoothing did not settle within 240 frames");
        f.step(vec![], None, true);
        assert_eq!(f.app.editor.presentation_camera3d(SIZE).unwrap(), before);
        f.app
            .mesh_lighting
            .begin_viewport_pick(&f.app.editor)
            .unwrap();
        f.app.cancel_mesh_light_pick();
        f.step(
            vec![
                egui::Event::PointerMoved(rect().center()),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, 120.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: Default::default(),
                },
            ],
            None,
            true,
        );
        let mut zoomed = f.app.editor.presentation_camera3d(SIZE).unwrap() != before;
        for _ in 0..8 {
            if zoomed {
                break;
            }
            f.step(vec![], None, true);
            zoomed = f.app.editor.presentation_camera3d(SIZE).unwrap() != before;
        }
        assert!(
            zoomed,
            "a fresh wheel gesture must zoom after quarantine drains"
        );
        unchanged(&f, checksum, &history, &files);
    }
}

#[test]
fn draft_source_and_context_aba_irreversibly_retire_app_ticket_without_writes() {
    for game in games() {
        let mut f = Fixture::new(game);
        let checksum = f.app.editor.checksum();
        let history = f.app.editor.history().clone();
        let files = [
            std::fs::read(&f.scene).unwrap(),
            std::fs::read(&f.sidecar).unwrap(),
        ];
        f.arm();
        let mut h = Harness::builder()
            .with_size([1400.0, 2800.0])
            .build_ui_state(
                |ui, f: &mut Fixture| {
                    f.app.sync_mesh_light_pick(ui.ctx());
                    f.app.mesh_lighting.show_with_navigation(
                        ui,
                        &f.app.editor,
                        &f.app.mesh_navigation,
                    );
                },
                f,
            );
        h.run_steps(3);
        h.get_by_label("Mesh game lighting").click_accesskit();
        h.run_steps(3);
        // Off and back on restores the exact default light values, but cannot
        // restore the generation of the armed draft.
        for _ in 0..2 {
            h.get_by_label("Enable point light").click_accesskit();
            h.run_steps(3);
        }
        assert!(!h.state().app.mesh_light_pick_pending());
        assert!(!h.state_mut().click(true));
        unchanged(h.state(), checksum, &history, &files);
        {
            let f = h.state_mut();
            f.step(vec![], None, false);
            f.app
                .mesh_lighting
                .begin_viewport_pick(&f.app.editor)
                .unwrap();
            let applied = f.app.point_light_settings_for_frame().unwrap();
            // Source admission runs at Begin and valid Stage, not every draw.
            // Observe the external edit with an actual staging attempt before
            // restoring the bytes; restoration cannot revive that ticket.
            let mut changed = files[1].clone();
            changed.push(b'\n');
            std::fs::write(&f.sidecar, changed).unwrap();
            assert!(f.click(true), "rejected staging still consumes the click");
            assert!(!f.app.mesh_light_pick_pending());
            assert_eq!(f.app.point_light_settings_for_frame().unwrap(), applied);
            assert_eq!(f.app.editor.checksum(), checksum);
            assert_eq!(f.app.editor.history(), &history);
            assert_eq!(std::fs::read(&f.scene).unwrap(), files[0]);
            std::fs::write(&f.sidecar, &files[1]).unwrap();
            f.step(vec![], None, false);
            assert!(!f.click(true));
            assert_eq!(f.app.point_light_settings_for_frame().unwrap(), applied);
            unchanged(f, checksum, &history, &files);
        }
        // Reload identical source bytes: checksum equality cannot restore scene
        // identity. The App must observe the source/context transition.
        {
            let f = h.state_mut();
            f.step(vec![], None, false);
            f.app
                .mesh_lighting
                .begin_viewport_pick(&f.app.editor)
                .unwrap();
            assert!(f.app.editor.open_path(&f.scene));
            wait_for_coherent_editor(&mut f.app.editor);
            let history = f.app.editor.history().clone();
            f.step(vec![], None, false);
            assert!(!f.app.mesh_light_pick_pending());
            assert!(!f.click(true));
            unchanged(f, checksum, &history, &files);
        }
    }
}
