//! CPU inspection widgets -> one-shot App dispatch -> actual presentation camera.
//! Synthetic viewport dimensions exercise no GPU, native window, or OS pointer.
#![cfg(all(feature = "mesh-mission", feature = "mesh-patrol"))]
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

const FRAME: &str = "Frame inspected patrol route";
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
    app: EditorApp,
    dispatch: Option<(u32, u32)>,
}
impl Fixture {
    fn new(game: EditorGame, projection: Projection, follow: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let scene = temp.path().join("patrol-frame.scene.yaml");
        std::fs::write(
            &scene,
            "schema: orr.scene/1\nsingletons: {}\nentities: {}\n",
        )
        .unwrap();
        let mut editor = Editor::open_game(&scene, EditorGame::MeshNavigation).unwrap();
        coherent(&mut editor);
        let mut navigation = crate::mesh_navigation::Panel::default();
        navigation.sync(&editor);
        navigation.create_ramp_and_bridge().unwrap();
        navigation.place_agent(&mut editor).unwrap();
        coherent(&mut editor);
        assert!(editor.save());
        // Save schedules public state/history refreshes. This CPU harness draws
        // authoring panels without the normal App pump, so settle those receipts
        // before testing the coherent framing admission guard.
        coherent(&mut editor);
        #[cfg(feature = "mesh-hazards")]
        if game.is_mesh_hazards() {
            let yaml = std::fs::read_to_string(&scene).unwrap();
            let mut doc =
                orr_remote::mesh_navigation::hazards::hazards_doc_from_yaml(&yaml).unwrap();
            std::fs::write(&scene, doc.save_yaml()).unwrap();
            editor = Editor::open_game(&scene, game).unwrap();
            coherent(&mut editor);
        }
        assert_eq!(editor.game(), game);
        let revision =
            orr_remote::mesh_navigation::mesh_from_view(editor.snapshot().unwrap().predicted())
                .unwrap()
                .graph
                .revision();
        let document = Document::from_orbit(orr_render::OrbitCamera {
            target: [8.0, 4.0, -7.0],
            yaw: 0.7,
            pitch: 0.55,
            distance: 30.0,
            fov_y_degrees: 52.0,
        })
        .unwrap()
        .with_projection(projection, 0.2, 300.0)
        .unwrap()
        .with_follow_offset(follow.then_some([0.0, 2.0, 0.0]))
        .unwrap();
        crate::mesh_camera_store::save(&scene, Some(&document), None, revision).unwrap();
        let camera_file = crate::mesh_camera_store::sidecar_path(&scene).unwrap();
        let mut app = EditorApp::new(editor, None);
        app.mesh_camera.sync(&mut app.editor);
        app.mesh_navigation.sync(&app.editor);
        Self {
            _temp: temp,
            scene,
            camera_file,
            app,
            dispatch: None,
        }
    }
    fn points(&self) -> Vec<[f32; 3]> {
        self.app
            .mesh_navigation
            .inspected_patrol_frame_points(&self.app.editor)
            .unwrap()
    }
    fn overlay(&self) -> Vec<orr_render::LineInstance3D> {
        let mut list = orr_render::RenderList3D::default();
        self.app
            .mesh_navigation
            .paint_draft_route(&self.app.editor, &mut list);
        list.lines
    }
    fn viewport(&mut self, size: (u32, u32)) {
        self.app.ui.viewport_px = size;
        self.app.ui.viewport_rect = Some(egui::Rect::from_min_size(
            egui::pos2(80.0, 90.0),
            egui::vec2(size.0 as f32, size.1 as f32),
        ));
    }
}
struct Receipt {
    checksum: u64,
    history: crate::model::History,
    selected: Option<orr_reflect::Guid>,
    dirty: bool,
    camera_dirty: bool,
    unplaced: bool,
    files: [Vec<u8>; 2],
}
impl Receipt {
    fn of(f: &Fixture) -> Self {
        Self {
            checksum: f.app.editor.checksum(),
            history: f.app.editor.history().clone(),
            selected: f.app.editor.selected_guid(),
            dirty: f.app.editor.is_dirty(),
            camera_dirty: f.app.mesh_camera.dirty(),
            unplaced: f.app.mesh_navigation.has_unplaced_draft(),
            files: [
                std::fs::read(&f.scene).unwrap(),
                std::fs::read(&f.camera_file).unwrap(),
            ],
        }
    }
    fn unchanged(&self, f: &Fixture) {
        assert_eq!(f.app.editor.checksum(), self.checksum);
        assert_eq!(f.app.editor.history(), &self.history);
        assert_eq!(f.app.editor.selected_guid(), self.selected);
        assert_eq!(f.app.editor.is_dirty(), self.dirty);
        assert_eq!(f.app.mesh_camera.dirty(), self.camera_dirty);
        assert_eq!(f.app.mesh_navigation.has_unplaced_draft(), self.unplaced);
        assert_eq!(std::fs::read(&f.scene).unwrap(), self.files[0]);
        assert_eq!(std::fs::read(&f.camera_file).unwrap(), self.files[1]);
    }
}
fn harness(game: EditorGame, projection: Projection, follow: bool) -> Harness<'static, Fixture> {
    let mut h = Harness::builder()
        .with_size([1600.0, 4400.0])
        .build_ui_state(
            |ui, f: &mut Fixture| {
                f.app.sync_mesh_patrol_frame(ui.ctx());
                f.app
                    .mesh_navigation
                    .sync_patrol_inspection_focus(&f.app.editor, ui.ctx());
                f.app.mesh_navigation.show(ui, &mut f.app.editor);
                if let Some(size) = f.dispatch.take() {
                    f.viewport(size);
                    f.app.dispatch_mesh_patrol_frame(ui.ctx(), size);
                }
            },
            Fixture::new(game, projection, follow),
        );
    h.run_steps(3);
    if game == EditorGame::MeshNavigation {
        click(&mut h, "Static-mesh navigation");
    }
    click(&mut h, "Waypoint patrol");
    click(&mut h, "Enable waypoint patrol");
    click(&mut h, "Add patrol waypoint");
    for (label, value) in [
        ("Patrol waypoint 1 X", "3"),
        ("Patrol waypoint 1 Y", "2"),
        ("Patrol waypoint 1 Z", "1"),
        ("Patrol waypoint 2 X", "1"),
        ("Patrol waypoint 2 Y", "0"),
        ("Patrol waypoint 2 Z", "1"),
    ] {
        text(&mut h, label, value);
    }
    click(&mut h, "Loop patrol");
    click(&mut h, "Inspect patrol draft route");
    click(&mut h, "Show inspected patrol route");
    h
}
fn click(h: &mut Harness<'_, Fixture>, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(3);
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
fn queue(h: &mut Harness<'_, Fixture>) {
    assert!(!h.get_by_label(FRAME).accesskit_node().is_disabled());
    click(h, FRAME);
    assert!(h.state().app.mesh_navigation.has_patrol_frame_request());
}
fn dispatch_raw(
    f: &mut Fixture,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    size: (u32, u32),
    dispatch: bool,
) {
    let mut out = ctx.run_ui(
        egui::RawInput {
            focused: true,
            events,
            ..Default::default()
        },
        |ui| {
            if dispatch {
                f.app.dispatch_mesh_patrol_frame(ui.ctx(), size);
            } else {
                f.app.sync_mesh_patrol_frame(ui.ctx());
            }
        },
    );
    out.textures_delta.clear();
}
fn fit(camera: &orr_render::Camera3D, points: &[[f32; 3]], size: (u32, u32)) {
    for point in points {
        let screen = camera.world_to_screen(*point, size).unwrap();
        assert!(
            screen[0] >= size.0 as f32 * 0.02 && screen[0] <= size.0 as f32 * 0.98,
            "{screen:?}"
        );
        assert!(
            screen[1] >= size.1 as f32 * 0.02 && screen[1] <= size.1 as f32 * 0.98,
            "{screen:?}"
        );
        let clip = camera
            .view_proj(size.0 as f32 / size.1 as f32)
            .transform_point4(*point);
        assert!(
            clip[3] > 0.0 && clip[2] >= 0.0 && clip[2] <= clip[3],
            "{clip:?}"
        );
    }
}

#[test]
fn inspected_bends_and_loop_closure_frame_through_real_button_and_app_at_multiple_aspects() {
    for game in games() {
        for projection in [
            Projection::Perspective,
            Projection::Orthographic { half_height: 8.0 },
        ] {
            for size in [(800, 600), (300, 900), (1600, 400)] {
                let mut h = harness(game, projection, false);
                let f = h.state();
                let points = f.points();
                assert!(points.iter().any(|p| p[1] > 0.0));
                assert!(
                    points
                        .iter()
                        .any(|p| *p != [1.0, 0.0, 1.0] && *p != [3.0, 2.0, 1.0]),
                    "cached report must contain actual path bends"
                );
                let overlay = f.overlay();
                assert!(
                    overlay
                        .iter()
                        .any(|line| line.color == [0.85, 0.35, 1.0, 1.0]),
                    "inspected loop closing leg must be present"
                );
                let before = f.app.editor.camera3d;
                let policy = f.app.editor.presentation_camera3d(size).unwrap().projection;
                let receipt = Receipt::of(f);
                // Clicking only queues the action; it cannot alter the camera.
                queue(&mut h);
                assert_eq!(h.state().app.editor.camera3d, before);
                h.state_mut().dispatch = Some(size);
                h.run_steps(1);
                let f = h.state_mut();
                assert!(!f.app.mesh_navigation.has_patrol_frame_request());
                let after = f.app.editor.camera3d;
                assert_ne!(
                    (after.target, after.distance),
                    (before.target, before.distance),
                    "framing dispatch: {:?}",
                    f.app.editor.status()
                );
                assert_eq!(
                    (after.yaw, after.pitch, after.fov_y_degrees),
                    (before.yaw, before.pitch, before.fov_y_degrees)
                );
                let camera = f.app.editor.presentation_camera3d(size).unwrap();
                match (policy, camera.projection) {
                    (
                        orr_render::Projection::Perspective {
                            fov_y: a,
                            near: b,
                            far: c,
                        },
                        orr_render::Projection::Perspective {
                            fov_y: d,
                            near: e,
                            far: g,
                        },
                    ) => assert_eq!((a, b, c), (d, e, g)),
                    (
                        orr_render::Projection::Orthographic {
                            near: a, far: b, ..
                        },
                        orr_render::Projection::Orthographic {
                            near: c, far: d, ..
                        },
                    ) => assert_eq!((a, b), (c, d)),
                    _ => panic!("framing changed projection kind"),
                }
                fit(&camera, &points, size);
                assert_eq!(f.points(), points);
                assert_eq!(f.overlay(), overlay);
                receipt.unchanged(f);
                let framed = f.app.editor.camera3d;
                h.state_mut().dispatch = Some(size);
                h.run_steps(2);
                assert_eq!(
                    h.state().app.editor.camera3d,
                    framed,
                    "request is consumed exactly once"
                );
            }
        }
        // A stationary inspected loop is still frameable. Reinspection defaults
        // the overlay off: framing must not require the separate Show checkbox.
        let mut h = harness(game, Projection::Perspective, false);
        for row in [1, 2] {
            for (axis, value) in [("X", "1"), ("Y", "0"), ("Z", "1")] {
                text(&mut h, &format!("Patrol waypoint {row} {axis}"), value);
            }
        }
        click(&mut h, "Inspect patrol draft route");
        assert!(h.state().overlay().is_empty());
        let points = h.state().points();
        assert!(points.iter().all(|p| *p == [1.0, 0.0, 1.0]));
        let before = h.state().app.editor.camera3d;
        let receipt = Receipt::of(h.state());
        queue(&mut h);
        h.state_mut().dispatch = Some((800, 600));
        h.run_steps(1);
        let f = h.state();
        assert_eq!(
            f.app.editor.camera3d.target,
            [1.0, 0.0, 1.0],
            "stationary framing dispatch: {:?}",
            f.app.editor.status()
        );
        assert_eq!(f.app.editor.camera3d.distance, before.distance);
        assert_eq!(
            (
                f.app.editor.camera3d.yaw,
                f.app.editor.camera3d.pitch,
                f.app.editor.camera3d.fov_y_degrees
            ),
            (before.yaw, before.pitch, before.fov_y_degrees)
        );
        fit(
            &f.app.editor.presentation_camera3d((800, 600)).unwrap(),
            &points,
            (800, 600),
        );
        assert!(f.overlay().is_empty());
        receipt.unchanged(f);
    }
}

#[test]
fn follow_preview_busy_invalid_camera_and_bad_viewport_reject_queued_action_atomically() {
    for game in games() {
        for cause in [
            "follow", "preview", "dialog", "invalid", "zero", "mismatch", "hidden",
        ] {
            let mut h = harness(game, Projection::Perspective, false);
            queue(&mut h);
            let f = h.state_mut();
            f.viewport((800, 600));
            match cause {
                "follow" => {
                    let doc = Document::from_orbit(f.app.editor.camera3d)
                        .unwrap()
                        .with_follow_offset(Some([0.0, 2.0, 0.0]))
                        .unwrap();
                    f.app.editor.install_mesh_camera(doc).unwrap();
                }
                "preview" => f
                    .app
                    .mesh_camera
                    .set_static_preview(&f.app.editor, true)
                    .unwrap(),
                "dialog" => {
                    f.app.ui.dialog = Some(crate::app::Dialog {
                        kind: crate::app::DialogKind::Open,
                        text: String::new(),
                    })
                }
                "invalid" => f.app.editor.camera3d.distance = f32::NAN,
                "hidden" => f.app.ui.viewport_rect = None,
                _ => {}
            }
            let receipt = Receipt::of(f);
            let before = format!("{:?}", f.app.editor.camera3d);
            let ctx = egui::Context::default();
            let size = match cause {
                "zero" => (0, 600),
                "mismatch" => (801, 600),
                _ => (800, 600),
            };
            dispatch_raw(f, &ctx, vec![], size, true);
            assert!(!f.app.mesh_navigation.has_patrol_frame_request());
            assert_eq!(format!("{:?}", f.app.editor.camera3d), before, "{cause}");
            assert!(
                f.app.mesh_patrol_frame_owns_input(),
                "rejected queued action owns its input batch: {cause}"
            );
            receipt.unchanged(f);
            dispatch_raw(f, &ctx, vec![], (800, 600), true);
            assert!(!f.app.mesh_navigation.has_patrol_frame_request());
            assert_eq!(format!("{:?}", f.app.editor.camera3d), before);
        }
    }
}

#[test]
fn changed_draft_frame_scene_or_path_cannot_frame_a_previous_cached_report() {
    for game in games() {
        for cause in ["draft", "frame", "scene", "path"] {
            let mut h = harness(game, Projection::Perspective, false);
            queue(&mut h);
            if cause == "draft" {
                text(&mut h, "Patrol waypoint 2 X", "invalid");
            }
            let f = h.state_mut();
            match cause {
                "frame" => {
                    let view = orr_remote::mesh_navigation::mesh_from_view(
                        f.app.editor.snapshot().unwrap().predicted(),
                    )
                    .unwrap();
                    let mut spec = view.pin.spec();
                    spec.distance_per_tick = orr_fp::FP::from_raw(spec.distance_per_tick.raw() + 1);
                    assert!(f.app.editor.set_field(
                        &crate::editor::Owner::Singleton(
                            orr_remote::mesh_navigation::PIN_NAME.into()
                        ),
                        "distance_per_tick",
                        orr_reflect::Value::Fixed(spec.distance_per_tick),
                    ));
                    coherent(&mut f.app.editor);
                }
                "scene" => {
                    assert!(f.app.editor.open_path(&f.scene));
                    coherent(&mut f.app.editor);
                }
                "path" => {
                    let path = f._temp.path().join("other.scene.yaml");
                    std::fs::copy(&f.scene, &path).unwrap();
                    f.app.editor = Editor::open_game(&path, game).unwrap();
                    coherent(&mut f.app.editor);
                }
                _ => {}
            }
            f.viewport((800, 600));
            let before = f.app.editor.camera3d;
            let receipt = Receipt::of(f);
            let ctx = egui::Context::default();
            dispatch_raw(f, &ctx, vec![], (800, 600), true);
            assert_eq!(f.app.editor.camera3d, before, "{cause}");
            receipt.unchanged(f);
            dispatch_raw(f, &ctx, vec![], (800, 600), true);
            assert_eq!(
                f.app.editor.camera3d, before,
                "stale action cannot revive: {cause}"
            );
        }
    }
}

#[test]
fn simultaneous_drag_wheel_rejects_once_and_fences_until_physical_release_and_quiet() {
    for game in games() {
        let mut h = harness(game, Projection::Perspective, false);
        queue(&mut h);
        let f = h.state_mut();
        f.viewport((800, 600));
        let before = f.app.editor.camera3d;
        let receipt = Receipt::of(f);
        let ctx = egui::Context::default();
        let button = |pressed| egui::Event::PointerButton {
            pos: egui::pos2(100.0, 100.0),
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Default::default(),
        };
        dispatch_raw(
            f,
            &ctx,
            vec![
                button(true),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, 120.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: Default::default(),
                },
            ],
            (800, 600),
            true,
        );
        assert_eq!(f.app.editor.camera3d, before);
        assert!(f.app.mesh_patrol_frame_owns_input());
        dispatch_raw(f, &ctx, vec![], (800, 600), false);
        assert!(f.app.mesh_patrol_frame_owns_input());
        dispatch_raw(f, &ctx, vec![button(false)], (800, 600), false);
        assert!(f.app.mesh_patrol_frame_owns_input());
        dispatch_raw(f, &ctx, vec![], (800, 600), false);
        assert!(!f.app.mesh_patrol_frame_owns_input());
        assert_eq!(f.app.editor.camera3d, before);
        receipt.unchanged(f);
        for existing_tail in [false, true] {
            let mut h = harness(game, Projection::Perspective, false);
            queue(&mut h);
            let f = h.state_mut();
            f.viewport((800, 600));
            let points = f.points();
            let before = f.app.editor.camera3d;
            let receipt = Receipt::of(f);
            let ctx = egui::Context::default();
            let wheel = egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, 120.0),
                phase: egui::TouchPhase::Move,
                modifiers: Default::default(),
            };
            let events = if existing_tail {
                // Seed only egui's scroll state. No App tool has owned this
                // wheel, so its private quarantine is still unset.
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        focused: true,
                        events: vec![wheel],
                        ..Default::default()
                    },
                    |_| {},
                );
                out.textures_delta.clear();
                assert!(!f.app.mesh_patrol_frame_owns_input());
                vec![]
            } else {
                vec![wheel]
            };
            dispatch_raw(f, &ctx, events, (800, 600), true);
            assert!(!ctx.input(|i| i.pointer.any_down()));
            assert!(ctx.input(|i| i.is_scrolling() || i.smooth_scroll_delta != egui::Vec2::ZERO));
            assert_eq!(
                f.app.editor.camera3d, before,
                "wheel-only tail={existing_tail}"
            );
            assert!(!f.app.mesh_navigation.has_patrol_frame_request());
            assert!(f.app.mesh_patrol_frame_owns_input());
            receipt.unchanged(f);
            dispatch_raw(f, &ctx, vec![], (800, 600), true);
            assert_eq!(
                f.app.editor.camera3d, before,
                "rejected action cannot be replayed"
            );
            let mut drained = false;
            for _ in 0..240 {
                dispatch_raw(f, &ctx, vec![], (800, 600), false);
                assert_eq!(f.app.editor.camera3d, before);
                if !ctx.input(|i| i.is_scrolling() || i.smooth_scroll_delta != egui::Vec2::ZERO) {
                    drained = true;
                    break;
                }
            }
            assert!(drained, "wheel smoothing must settle");
            dispatch_raw(f, &ctx, vec![], (800, 600), false);
            assert!(!f.app.mesh_patrol_frame_owns_input());
            receipt.unchanged(f);
            // Settling alone never frames anything: another explicit widget
            // activation must queue a new, current request.
            h.run_steps(2);
            assert_eq!(h.state().app.editor.camera3d, before);
            queue(&mut h);
            h.state_mut().dispatch = Some((800, 600));
            h.run_steps(1);
            let f = h.state();
            assert_ne!(
                f.app.editor.camera3d,
                before,
                "framing after wheel drain: {:?}",
                f.app.editor.status()
            );
            assert!(!f.app.mesh_navigation.has_patrol_frame_request());
            fit(
                &f.app.editor.presentation_camera3d((800, 600)).unwrap(),
                &points,
                (800, 600),
            );
            receipt.unchanged(f);
        }
    }
}
