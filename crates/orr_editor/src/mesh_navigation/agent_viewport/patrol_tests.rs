//! Actual CPU adapter tests; no OS window, GPU or physical mouse input.
use super::*;
use crate::mesh_navigation::{Panel, PatrolPickRequest};
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_navigation::AgentProfile;
use orr_remote::mesh_navigation::{authoring, AgentSpec};
pub(super) fn click(h: &mut Harness<'_, (Panel, Editor)>, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(3);
}
pub(super) fn text(h: &mut Harness<'_, (Panel, Editor)>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    h.run_steps(3);
}
pub(super) fn camera() -> Camera3D {
    Camera3D::perspective([2.0, 8.0, 9.0], [2.0, 1.0, 1.0], 55.0)
}
pub(super) fn focused(panel: &mut Panel, editor: &Editor, input: egui::RawInput) {
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(input, |ui| {
        panel.sync_patrol_picking(editor, ui.ctx(), false)
    });
    output.textures_delta.clear();
}
pub(super) fn fixture(game: EditorGame) -> (tempfile::TempDir, Harness<'static, (Panel, Editor)>) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("patrol-pick.scene.yaml");
    let p = |x, y, z| [FP::from_int(x), FP::from_int(y), FP::from_int(z)];
    let g = StaticMeshGraph::bake(
        &[p(0, 0, 0), p(0, 0, 4), p(4, 2, 0), p(4, 2, 4)],
        &[[0, 1, 2], [2, 1, 3]],
        AgentProfile {
            max_slope: FP::ONE,
            ..Default::default()
        },
    )
    .unwrap();
    let mut doc = authoring::create_mesh_scene(
        &g,
        AgentSpec {
            start: p(0, 0, 0),
            goal: p(4, 2, 0),
            max_vertical: FP::ONE,
            distance_per_tick: FP::ONE,
        },
    )
    .unwrap();
    let yaml = doc.save_yaml();
    #[cfg(feature = "mesh-hazards")]
    let yaml = if game.is_mesh_hazards() {
        orr_remote::mesh_navigation::hazards::hazards_doc_from_yaml(&yaml)
            .unwrap()
            .save_yaml()
    } else {
        yaml
    };
    std::fs::write(&path, yaml).unwrap();
    let mut editor = Editor::open_game(&path, game).unwrap();
    editor.sync();
    let mut panel = Panel::default();
    panel.sync(&editor);
    let mut h = Harness::builder()
        .with_size([1600.0, 4600.0])
        .build_ui_state(
            |ui, state: &mut (Panel, Editor)| {
                state.0.sync_patrol_picking(&state.1, ui.ctx(), false);
                state.0.show(ui, &mut state.1);
                match state.0.take_patrol_pick_request() {
                    Some(PatrolPickRequest::Pick(row)) => {
                        let _ = state.0.begin_patrol_pick(&state.1, row);
                    }
                    Some(PatrolPickRequest::Cancel) => state.0.cancel_patrol_pick(),
                    None => {}
                }
            },
            (panel, editor),
        );
    h.run_steps(3);
    if game.is_mesh_navigation() {
        click(&mut h, "Static-mesh navigation");
    }
    click(&mut h, "Waypoint patrol");
    click(&mut h, "Enable waypoint patrol");
    click(&mut h, "Add patrol waypoint");
    (temp, h)
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
fn quiet() -> egui::RawInput {
    egui::RawInput {
        focused: true,
        ..Default::default()
    }
}
#[test]
fn fresh_frame_gate_cancels_and_legacy_adapter_cannot_stage_owned_pick() {
    for game in games() {
        let (_temp, mut h) = fixture(game);
        let (panel, editor) = h.state_mut();
        focused(panel, editor, quiet());
        panel.begin_patrol_pick(editor, 0).unwrap();
        let before = editor.snapshot().unwrap().predicted().checksum();
        let history = editor.history().entries.len();
        let start = panel.start.clone();
        let goal = panel.goal.clone();
        let cam = camera();
        let pixel = cam.world_to_screen([2.0, 1.0, 1.0], (800, 600)).unwrap();
        let input = ViewInput {
            pixel: Some(pixel),
            clicked: true,
            cancel: false,
        };
        assert!(panel.agent_viewport_input(editor, &cam, (800, 600), input));
        assert!(panel.patrol_pick_busy());
        assert!(panel.patrol_viewport_input(editor, &cam, (800, 600), input, false));
        assert!(!panel.patrol_pick_busy());
        assert!(panel.patrol_pick_owns_input());
        assert_eq!(panel.start, start);
        assert_eq!(panel.goal, goal);
        assert_eq!(editor.snapshot().unwrap().predicted().checksum(), before);
        assert_eq!(editor.history().entries.len(), history);
        focused(panel, editor, quiet());
        assert!(!panel.patrol_pick_owns_input());
    }
}
#[test]
fn focus_loss_with_same_batch_regain_owns_press_through_release() {
    let (_temp, mut h) = fixture(EditorGame::MeshNavigation);
    let (panel, editor) = h.state_mut();
    focused(panel, editor, quiet());
    panel.begin_patrol_pick(editor, 0).unwrap();
    let before = editor.snapshot().unwrap().predicted().checksum();
    let event = |pressed| egui::Event::PointerButton {
        pos: egui::pos2(10.0, 10.0),
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    let mut input = quiet();
    input.events = vec![
        egui::Event::WindowFocused(false),
        egui::Event::WindowFocused(true),
        event(true),
    ];
    focused(panel, editor, input);
    assert!(!panel.patrol_pick_busy());
    assert!(panel.patrol_pick_owns_input());
    focused(panel, editor, quiet());
    assert!(
        panel.patrol_pick_owns_input(),
        "focus reset is not a physical release"
    );
    let mut release = quiet();
    release.events = vec![event(false)];
    focused(panel, editor, release);
    assert!(panel.patrol_pick_owns_input());
    focused(panel, editor, quiet());
    assert!(!panel.patrol_pick_owns_input());
    assert_eq!(editor.snapshot().unwrap().predicted().checksum(), before);
}
#[test]
fn draft_aba_and_host_frame_change_irreversibly_retire_armed_row() {
    let (_temp, mut h) = fixture(EditorGame::MeshNavigation);
    click(&mut h, "Pick patrol waypoint 1");
    assert!(h.state().0.patrol_pick_busy());
    text(&mut h, "Patrol wait ticks", "1");
    assert!(!h.state().0.patrol_pick_busy());
    text(&mut h, "Patrol wait ticks", "0");
    assert!(!h.state().0.patrol_pick_busy());
    let (panel, editor) = h.state_mut();
    focused(panel, editor, quiet());
    panel.begin_patrol_pick(editor, 0).unwrap();
    let view = orr_remote::mesh_navigation::mesh_from_view(editor.snapshot().unwrap().predicted())
        .unwrap();
    let mut spec = view.pin.spec();
    spec.distance_per_tick = FP::from_raw(spec.distance_per_tick.raw() + 1);
    let pin = orr_remote::mesh_navigation::MeshScenePin::new(&view.graph, spec).unwrap();
    editor
        .host_call(
            "world.singleton.patch",
            crate::mesh_navigation::pin_patch(&pin),
        )
        .unwrap();
    editor.sync();
    let before = editor.snapshot().unwrap().predicted().checksum();
    focused(panel, editor, quiet());
    assert!(!panel.patrol_pick_busy());
    assert_eq!(editor.snapshot().unwrap().predicted().checksum(), before);
}
