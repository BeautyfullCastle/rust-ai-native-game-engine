//! CPU widget state only; no Editor, native window, renderer or filesystem.
use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
fn context() -> Context {
    Context {
        scene: "mesh.scene.yaml".into(),
        epoch: 1,
        checksum: 2,
        revision: [3; 32],
    }
}
fn ready_state() -> State {
    let mut state = State::default();
    state.load(context(), None, [FP::ONE; 3]);
    state.form.enabled = true;
    let pin = InspectionPin {
        context: context(),
        form: state.form.clone(),
        generation: state.generation,
        start: [FP::ZERO; 3],
        max_vertical: FP::ONE,
    };
    let report = super::super::patrol_routes::Report {
        revision: [3; 32],
        start: [FP::ZERO; 3],
        waypoints: vec![[FP::ONE; 3]],
        legs: vec![super::super::patrol_routes::Leg {
            points: vec![[FP::ZERO; 3], [FP::ONE; 3]],
            length: FP::ONE,
            corridor_triangles: 1,
        }],
        total_length: FP::ONE,
    };
    state.inspection = Some((pin, Ok(report)));
    state
}
fn harness(state: State, ready: bool) -> Harness<'static, (State, usize)> {
    Harness::builder()
        .with_size([1300.0, 2200.0])
        .build_ui_state(
            move |ui, state: &mut (State, usize)| {
                state.1 += usize::from(state.0.inspection_controls(ui, ready));
            },
            (state, 0),
        )
}
fn click(h: &mut Harness<'_, (State, usize)>, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(2);
}
#[test]
fn explicit_inspect_requests_and_overlay_toggle_are_readonly_and_clear_is_explicit() {
    let mut h = harness(ready_state(), true);
    h.run_steps(2);
    let form = h.state().0.form.clone();
    let generation = h.state().0.generation;
    assert!(!h.state().0.show_inspection);
    assert_eq!(h.state().1, 0);
    click(&mut h, "Show inspected patrol route");
    assert!(h.state().0.show_inspection);
    click(&mut h, "Inspect patrol draft route");
    assert_eq!(h.state().1, 1);
    assert_eq!(h.state().0.form, form);
    assert_eq!(h.state().0.generation, generation);
    assert!(!h.state().0.dirty);
    click(&mut h, "Clear patrol inspection");
    assert!(h.state().0.inspection.is_none());
    assert!(!h.state().0.show_inspection);
    assert_eq!(h.state().0.form, form);
}
#[test]
fn unavailable_or_failed_inspection_disables_display_and_inspect_guard() {
    for failed in [false, true] {
        let mut state = ready_state();
        if failed {
            state.inspection.as_mut().unwrap().1 = Err("leg2 unreachable".into());
        } else {
            state.inspection = None;
        }
        let mut h = harness(state, false);
        h.run_steps(2);
        assert!(
            h.get_by_label("Inspect patrol draft route")
                .accesskit_node()
                .is_disabled()
        );
        assert!(
            h.get_by_label("Show inspected patrol route")
                .accesskit_node()
                .is_disabled()
        );
        click(&mut h, "Inspect patrol draft route");
        assert_eq!(h.state().1, 0);
        click(&mut h, "Clear patrol inspection");
        assert!(h.state().0.inspection.is_none());
    }
}
#[test]
fn retired_observation_stays_hidden_until_explicit_fresh_inspection_or_reload() {
    let mut state = ready_state();
    state.show_inspection = true;
    state.retired.set(true);
    let mut h = harness(state, true);
    h.run_steps(2);
    assert!(h.state().0.retired.get());
    assert!(!h.state().0.show_inspection);
    assert!(
        h.get_by_label("Show inspected patrol route")
            .accesskit_node()
            .is_disabled()
    );
    click(&mut h, "Inspect patrol draft route");
    assert_eq!(h.state().1, 1);
    assert!(h.state().0.retired.get());
    // The production handler alone can replace the report after revalidation.
    h.state_mut().0.load(context(), None, [FP::ONE; 3]);
    assert!(h.state().0.inspection.is_none());
    assert!(!h.state().0.show_inspection);
}
#[test]
fn form_edits_and_reload_discard_report_and_advance_nonwrapping_generation() {
    let mut state = ready_state();
    state.show_inspection = true;
    let mut h = Harness::builder()
        .with_size([1300.0, 2200.0])
        .build_ui_state(
            |ui, state: &mut State| {
                state.controls(ui, true);
            },
            state,
        );
    h.run_steps(2);
    let generation = h.state().generation;
    h.get_by_label("Add patrol waypoint").click_accesskit();
    h.run_steps(2);
    assert!(h.state().inspection.is_none());
    assert!(!h.state().show_inspection);
    assert!(h.state().generation > generation);
    h.state_mut().generation = u64::MAX;
    h.state_mut().load(context(), None, [FP::ONE; 3]);
    assert_eq!(h.state().generation, u64::MAX);
}

#[test]
fn same_frame_focus_loss_and_inspect_action_cannot_request_inspection() {
    let mut h = harness(ready_state(), true);
    h.run_steps(2);
    let (target_node, target_tree) = h
        .get_by_label("Inspect patrol draft route")
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
    assert_eq!(h.state().1, 0);
    assert!(h.state().0.retired.get());
    assert!(
        h.get_by_label("Inspect patrol draft route")
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        h.get_by_label("Show inspected patrol route")
            .accesskit_node()
            .is_disabled()
    );
}
