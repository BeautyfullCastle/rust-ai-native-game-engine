//! CPU widgets only: ordering never touches a host, history or files.
use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
fn state() -> State {
    let mut state = State::default();
    state.load(
        Context {
            scene: "order.scene.yaml".into(),
            epoch: 1,
            checksum: 2,
            revision: [3; 32],
        },
        None,
        [FP::ONE; 3],
    );
    state.form.enabled = true;
    state.form.points = vec![
        ["01.00".into(), "-0.0".into(), "1.250".into()],
        ["2".into(), "0".into(), "2".into()],
        ["3".into(), "0".into(), "3".into()],
    ];
    state
}
fn harness(state: State, editable: bool) -> Harness<'static, (State, usize)> {
    let mut h = Harness::builder()
        .with_size([1500.0, 4000.0])
        .build_ui_state(
            move |ui, s: &mut (State, usize)| s.1 += usize::from(s.0.controls(ui, editable)),
            (state, 0),
        );
    h.run_steps(2);
    h
}
fn click(h: &mut Harness<'_, (State, usize)>, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(2);
}
#[test]
fn insert_and_move_preserve_exact_raw_xyz_strings_and_only_stage() {
    let mut h = harness(state(), true);
    let original = h.state().0.form.points.clone();
    click(&mut h, "Insert before patrol waypoint 2");
    assert_eq!(
        h.state().0.form.points,
        vec![
            original[0].clone(),
            original[1].clone(),
            original[1].clone(),
            original[2].clone()
        ]
    );
    click(&mut h, "Remove patrol waypoint 2");
    click(&mut h, "Move patrol waypoint 3 up");
    click(&mut h, "Move patrol waypoint 2 up");
    assert_eq!(
        h.state().0.form.points,
        vec![
            original[2].clone(),
            original[0].clone(),
            original[1].clone()
        ]
    );
    assert!(h.state().0.dirty);
    assert_eq!(h.state().1, 0);
}
#[test]
fn identical_row_swap_is_structural_and_retires_pending_pick_and_preview() {
    let mut state = state();
    state.form.points = vec![["same".into(), "raw".into(), "text".into()]; 2];
    state.pick_request = Some((
        PatrolPickRequest::Pick(1),
        state.form.clone(),
        state.generation,
    ));
    state.show_inspection = true;
    let mut h = harness(state, true);
    let before = h.state().0.form.clone();
    let generation = h.state().0.generation;
    click(&mut h, "Move patrol waypoint 2 up");
    assert_eq!(h.state().0.form, before);
    assert!(h.state().0.generation > generation);
    assert!(h.state().0.dirty);
    assert!(h.state().0.pick_request.is_none());
    assert!(!h.state().0.show_inspection);
    assert_eq!(h.state().1, 0);
}
#[test]
fn edge_and_eight_row_controls_are_disabled_without_changing_form() {
    let mut state = state();
    state.form.points = vec![["0".into(), "0".into(), "0".into()]; 8];
    let before = state.form.clone();
    let mut h = harness(state, true);
    for label in [
        "Insert before patrol waypoint 1",
        "Move patrol waypoint 1 up",
        "Move patrol waypoint 8 down",
    ] {
        assert!(h.get_by_label(label).accesskit_node().is_disabled());
        click(&mut h, label);
    }
    assert_eq!(h.state().0.form, before);
    assert!(!h.state().0.dirty);
    assert_eq!(h.state().1, 0);
}
#[test]
fn readonly_stale_and_same_frame_focus_loss_refuse_structural_commands() {
    for case in 0..3 {
        let mut state = state();
        if case == 1 {
            state.stale = true;
        }
        let before = state.form.clone();
        let generation = state.generation;
        let mut h = harness(state, case != 0);
        if case == 2 {
            let (target_node, target_tree) = h
                .get_by_label("Move patrol waypoint 2 up")
                .accesskit_node()
                .locate();
            h.input_mut().events.extend([
                egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
                    target_node,
                    target_tree,
                    action: egui::accesskit::Action::Click,
                    data: None,
                }),
                egui::Event::WindowFocused(false),
            ]);
            h.step();
        } else {
            assert!(
                h.get_by_label("Move patrol waypoint 2 up")
                    .accesskit_node()
                    .is_disabled()
            );
            click(&mut h, "Move patrol waypoint 2 up");
        }
        assert_eq!(h.state().0.form, before);
        assert_eq!(h.state().0.generation, generation);
        assert!(!h.state().0.dirty);
        assert_eq!(h.state().1, 0);
    }
}
