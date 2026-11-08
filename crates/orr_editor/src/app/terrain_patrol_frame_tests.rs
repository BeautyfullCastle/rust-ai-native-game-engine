//! Real CPU inspection widgets and App dispatch; no GPU or physical-input claim.
use super::*;
use crate::{
    editor::Editor, game::EditorGame, navigation_panel::NavigationPanel,
    terrain_panel::TerrainPanel,
};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_fp::FP;
use std::path::PathBuf;

const FRAME: &str = "Frame inspected patrol route";
const INSPECT: &str = "Inspect terrain patrol draft route";
fn coherent(editor: &mut Editor) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        editor.sync();
        if editor.can_mutate()
            && editor.yard_rows_coherent()
            && editor.admitted_navigation().admitted
        {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Terrain fixture did not become coherent"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    scene: PathBuf,
    terrain_file: PathBuf,
    app: EditorApp,
    dispatch: Option<(u32, u32)>,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let scene = temp.path().join("terrain-patrol.scene.yaml");
        std::fs::write(
            &scene,
            include_str!("../../../../scenes/navigation_blank.scene.yaml"),
        )
        .unwrap();
        let mut editor = Editor::open_game(&scene, EditorGame::NavigationYard3D).unwrap();
        // Admission needs a built route; wait for the initial writable scene first.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            editor.sync();
            if editor.can_mutate() && editor.yard_rows_coherent() {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let mut terrain = TerrainPanel::default();
        terrain.local_path = "route.orrt".into();
        terrain.asset_id = "navigation/patrol-fixture".into();
        terrain.dimensions = [9, 9];
        terrain.origin = ["-4".into(), "-4".into()];
        terrain.spacing = "1".into();
        terrain.initial_height = "0".into();
        terrain.create_for_editor(&editor).unwrap();
        let mut edits = Vec::new();
        for z in 0..9 {
            for x in 0..9 {
                edits.push(orr_terrain::Edit::SetHeight {
                    x,
                    z,
                    height: FP::from_raw(i64::from(x) * 16384),
                });
            }
        }
        edits.push(orr_terrain::Edit::SetHole {
            x: 4,
            z: 6,
            hole: true,
        });
        terrain.document.apply(&edits).unwrap();
        terrain.save_for_editor(&editor).unwrap();
        let mut navigation = NavigationPanel::default();
        navigation.start = ["-3".into(), "0".into()];
        navigation.goal = ["3".into(), "0".into()];
        navigation.distance_per_tick = "1".into();
        navigation.build_route(&mut editor, &terrain).unwrap();
        coherent(&mut editor);
        assert!(editor.save());
        // Save schedules state/history refreshes; this panel harness has no App pump.
        coherent(&mut editor);
        navigation.sync_from_editor(&editor);
        editor.camera3d = orr_render::OrbitCamera::new([8., 4., -7.], 0.7, 0.55, 30.);
        let mut app = EditorApp::new(editor, None);
        app.terrain = terrain;
        app.navigation = navigation;
        let terrain_file = temp.path().join("route.orrt");
        Self {
            _temp: temp,
            scene,
            terrain_file,
            app,
            dispatch: None,
        }
    }
    fn points(&self) -> Vec<[f32; 3]> {
        self.app
            .navigation
            .inspected_patrol_frame_points(&self.app.editor, &self.app.terrain)
            .unwrap()
    }
    fn overlay(&self) -> Vec<orr_render::LineInstance3D> {
        let mut list = orr_render::RenderList3D::default();
        self.app
            .navigation
            .paint_patrol_route(&self.app.editor, &self.app.terrain, &mut list);
        list.lines
    }
    fn viewport(&mut self, size: (u32, u32)) {
        self.app.ui.viewport_px = size;
        self.app.ui.viewport_rect = Some(egui::Rect::from_min_size(
            egui::pos2(80., 90.),
            egui::vec2(size.0 as f32, size.1 as f32),
        ));
    }
}
struct Receipt {
    checksum: u64,
    history: crate::model::History,
    selected: Option<orr_reflect::Guid>,
    dirty: bool,
    files: [Vec<u8>; 2],
}
impl Receipt {
    fn of(f: &Fixture) -> Self {
        Self {
            checksum: f.app.editor.checksum(),
            history: f.app.editor.history().clone(),
            selected: f.app.editor.selected_guid(),
            dirty: f.app.editor.is_dirty(),
            files: [
                std::fs::read(&f.scene).unwrap(),
                std::fs::read(&f.terrain_file).unwrap(),
            ],
        }
    }
    fn unchanged(&self, f: &Fixture) {
        assert_eq!(f.app.editor.checksum(), self.checksum);
        assert_eq!(f.app.editor.history(), &self.history);
        assert_eq!(f.app.editor.selected_guid(), self.selected);
        assert_eq!(f.app.editor.is_dirty(), self.dirty);
        assert_eq!(std::fs::read(&f.scene).unwrap(), self.files[0]);
        assert_eq!(std::fs::read(&f.terrain_file).unwrap(), self.files[1]);
    }
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
fn harness() -> Harness<'static, Fixture> {
    let mut h = Harness::builder().with_size([1600., 3200.]).build_ui_state(
        |ui, f: &mut Fixture| {
            f.app.sync_terrain_patrol_frame(ui.ctx());
            f.app
                .navigation
                .sync_patrol_inspection_focus(&f.app.editor, &f.app.terrain, ui.ctx());
            f.app.navigation.show(ui, &mut f.app.editor, &f.app.terrain);
            if let Some(size) = f.dispatch.take() {
                f.viewport(size);
                f.app.dispatch_terrain_patrol_frame(ui.ctx(), size);
            }
        },
        Fixture::new(),
    );
    h.run_steps(3);
    for label in [
        "Point navigation",
        "Terrain waypoint patrol",
        "Enable terrain waypoint patrol",
        "Add terrain patrol waypoint",
    ] {
        click(&mut h, label);
    }
    for (label, value) in [
        ("Terrain patrol waypoint 1 X", "-2"),
        ("Terrain patrol waypoint 1 Z", "2.5"),
        ("Terrain patrol waypoint 2 X", "2"),
        ("Terrain patrol waypoint 2 Z", "2.5"),
    ] {
        text(&mut h, label, value);
    }
    click(&mut h, "Loop terrain patrol");
    click(&mut h, INSPECT);
    h
}
fn queue(h: &mut Harness<'_, Fixture>) {
    assert!(!h.get_by_label(FRAME).accesskit_node().is_disabled());
    click(h, FRAME);
    assert!(h.state().app.navigation.has_patrol_frame_request());
}
fn raw(
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
                f.app.dispatch_terrain_patrol_frame(ui.ctx(), size);
            } else {
                f.app.sync_terrain_patrol_frame(ui.ctx());
            }
        },
    );
    out.textures_delta.clear();
}
fn fit(f: &Fixture, points: &[[f32; 3]], size: (u32, u32)) {
    let camera = f.app.editor.camera3d.camera();
    for p in points {
        let screen = camera.world_to_screen(*p, size).unwrap();
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
            .transform_point4(*p);
        assert!(
            clip[3] > 0. && clip[2] >= 0. && clip[2] <= clip[3],
            "{clip:?}"
        );
    }
}
#[test]
fn inspected_height_bends_approach_and_closure_fit_real_app_at_multiple_aspects() {
    for size in [(800, 600), (300, 900), (1600, 400)] {
        let mut h = harness();
        let points = h.state().points();
        assert!(points.iter().any(|p| p[1] > 0.));
        assert!(
            points.iter().any(|p| p[0] != -2. && p[0] != 2.),
            "actual cached approach/bend points"
        );
        assert!(
            points.iter().any(|p| p[2] != 0. && p[2] != 2.5),
            "cached path must bend around the hole rather than join endpoints"
        );
        assert!(
            h.state().overlay().is_empty(),
            "Show is independent of Frame"
        );
        click(&mut h, "Show inspected terrain patrol route");
        let overlay = h.state().overlay();
        assert!(
            overlay.iter().any(|l| l.color == [0.85, 0.35, 1., 1.]),
            "closing leg"
        );
        let before = h.state().app.editor.camera3d;
        let receipt = Receipt::of(h.state());
        queue(&mut h);
        assert_eq!(h.state().app.editor.camera3d, before);
        h.state_mut().dispatch = Some(size);
        h.run_steps(1);
        let f = h.state();
        let after = f.app.editor.camera3d;
        assert_ne!(
            (after.target, after.distance),
            (before.target, before.distance),
            "{:?}",
            f.app.editor.status()
        );
        assert_eq!(
            (after.yaw, after.pitch, after.fov_y_degrees),
            (before.yaw, before.pitch, before.fov_y_degrees)
        );
        fit(f, &points, size);
        assert_eq!(f.points(), points);
        assert_eq!(f.overlay(), overlay);
        receipt.unchanged(f);
        h.state_mut().dispatch = Some(size);
        h.run_steps(2);
        assert_eq!(h.state().app.editor.camera3d, after, "consumed once");
    }
    // A valid inspection is frameable while its separate overlay is hidden.
    let mut h = harness();
    let points = h.state().points();
    let before = h.state().app.editor.camera3d;
    queue(&mut h);
    h.state_mut().dispatch = Some((800, 600));
    h.run_steps(1);
    assert_ne!(
        h.state().app.editor.camera3d,
        before,
        "{:?}",
        h.state().app.editor.status()
    );
    assert!(h.state().overlay().is_empty());
    fit(h.state(), &points, (800, 600));
}
#[test]
fn invalid_camera_busy_hidden_or_mismatched_viewport_reject_once_without_partial_fit() {
    for cause in ["invalid", "dialog", "hidden", "zero", "mismatch"] {
        let mut h = harness();
        queue(&mut h);
        let f = h.state_mut();
        f.viewport((800, 600));
        match cause {
            "invalid" => f.app.editor.camera3d.distance = f32::NAN,
            "dialog" => {
                f.app.ui.dialog = Some(crate::app::Dialog {
                    kind: crate::app::DialogKind::Open,
                    text: String::new(),
                })
            }
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
        raw(f, &ctx, vec![], size, true);
        assert!(!f.app.navigation.has_patrol_frame_request());
        assert!(f.app.terrain_patrol_frame_owns_input());
        assert_eq!(format!("{:?}", f.app.editor.camera3d), before, "{cause}");
        receipt.unchanged(f);
        f.viewport((800, 600));
        f.app.ui.dialog = None;
        raw(f, &ctx, vec![], (800, 600), true);
        assert_eq!(
            format!("{:?}", f.app.editor.camera3d),
            before,
            "no deferred {cause}"
        );
    }
}
#[test]
fn changed_form_terrain_scene_or_path_cannot_reuse_cached_inspection() {
    for cause in ["form", "terrain", "scene", "path"] {
        let mut h = harness();
        queue(&mut h);
        if cause == "form" {
            text(&mut h, "Terrain patrol waypoint 2 X", "invalid");
        }
        let f = h.state_mut();
        match cause {
            "terrain" => {
                f.app
                    .terrain
                    .document
                    .apply(&[orr_terrain::Edit::SetHeight {
                        x: 0,
                        z: 0,
                        height: FP::ONE,
                    }])
                    .unwrap();
            }
            "scene" => {
                assert!(f.app.editor.open_path(&f.scene));
                coherent(&mut f.app.editor);
            }
            "path" => {
                let path = f._temp.path().join("other.scene.yaml");
                std::fs::copy(&f.scene, &path).unwrap();
                f.app.editor = Editor::open_game(&path, EditorGame::NavigationYard3D).unwrap();
                coherent(&mut f.app.editor);
            }
            _ => {}
        }
        f.viewport((800, 600));
        let before = f.app.editor.camera3d;
        let receipt = Receipt::of(f);
        let ctx = egui::Context::default();
        raw(f, &ctx, vec![], (800, 600), true);
        assert_eq!(f.app.editor.camera3d, before, "{cause}");
        receipt.unchanged(f);
        raw(f, &ctx, vec![], (800, 600), true);
        assert_eq!(f.app.editor.camera3d, before, "no deferred {cause}");
        assert!(!f.app.navigation.has_patrol_frame_request());
    }
}
#[test]
fn focus_drag_wheel_and_unowned_scroll_tail_reject_then_allow_fresh_explicit_action() {
    for cause in ["focus", "drag", "wheel", "tail"] {
        let mut h = harness();
        queue(&mut h);
        let f = h.state_mut();
        f.viewport((800, 600));
        let points = f.points();
        let receipt = Receipt::of(f);
        let before = f.app.editor.camera3d;
        let ctx = egui::Context::default();
        let wheel = || egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0., 120.),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
        };
        let button = |pressed| egui::Event::PointerButton {
            pos: egui::pos2(100., 100.),
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Default::default(),
        };
        if cause == "tail" {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    focused: true,
                    events: vec![wheel()],
                    ..Default::default()
                },
                |_| {},
            );
            out.textures_delta.clear();
            assert!(!f.app.terrain_patrol_frame_owns_input());
        }
        let events = match cause {
            "focus" => vec![
                egui::Event::WindowFocused(false),
                egui::Event::WindowFocused(true),
                button(true),
            ],
            "drag" => vec![button(true)],
            "wheel" => vec![wheel()],
            _ => vec![],
        };
        raw(f, &ctx, events, (800, 600), true);
        assert_eq!(f.app.editor.camera3d, before, "{cause}");
        assert!(!f.app.navigation.has_patrol_frame_request());
        assert!(f.app.terrain_patrol_frame_owns_input());
        if cause == "focus" || cause == "drag" {
            raw(f, &ctx, vec![], (800, 600), false);
            assert!(f.app.terrain_patrol_frame_owns_input());
            raw(f, &ctx, vec![button(false)], (800, 600), false);
            assert!(f.app.terrain_patrol_frame_owns_input());
        }
        let mut drained = false;
        for _ in 0..240 {
            raw(f, &ctx, vec![], (800, 600), false);
            assert_eq!(f.app.editor.camera3d, before);
            if !ctx.input(|i| i.is_scrolling() || i.smooth_scroll_delta != egui::Vec2::ZERO) {
                drained = true;
                break;
            }
        }
        assert!(drained);
        raw(f, &ctx, vec![], (800, 600), false);
        assert!(!f.app.terrain_patrol_frame_owns_input());
        assert!(!f.app.terrain_patrol_frame_blocks_scroll());
        receipt.unchanged(f);
        raw(f, &ctx, vec![], (800, 600), true);
        assert_eq!(f.app.editor.camera3d, before, "settling cannot replay");
        h.run_steps(2);
        // Focus loss retires the report; only a fresh inspection may rearm it.
        if cause == "focus" {
            click(&mut h, INSPECT);
        }
        queue(&mut h);
        h.state_mut().dispatch = Some((800, 600));
        h.run_steps(1);
        assert_ne!(
            h.state().app.editor.camera3d,
            before,
            "fresh {cause}: {:?}",
            h.state().app.editor.status()
        );
        fit(h.state(), &points, (800, 600));
        receipt.unchanged(h.state());
    }
}
