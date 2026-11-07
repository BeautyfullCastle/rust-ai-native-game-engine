//! Pure CPU widgets; no Editor, Frame, filesystem or GPU operation.
use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
fn context() -> Context {
    Context {
        scene: "scene.yaml".into(),
        epoch: 1,
        checksum: 2,
        pin: NavigationScenePin {
            source_len: 0,
            identity_len: 0,
            source_path: [0; 256],
            asset_id: [0; 256],
            terrain_revision: [3; 32],
            graph_revision: [4; 32],
            agent: NavigationAgentSpec {
                start: [FP::ZERO; 2],
                goal: [FP::ONE; 2],
                max_slope: FP::ONE,
                distance_per_tick: FP::ONE,
            },
        },
    }
}

fn ready() -> State {
    let mut state = State::default();
    state.load(context(), None);
    state.form.enabled = true;
    state.form.points = vec![
        ["01.2500".into(), "-0.000".into()],
        ["2.50".into(), "3.000".into()],
        ["-4.0".into(), "5.1250".into()],
    ];
    state.form.looped = true;
    state.form.wait = "002".into();
    state
}
fn harness(state: State, editable: bool) -> Harness<'static, (State, usize)> {
    Harness::builder().with_size([1500., 3000.]).build_ui_state(
        move |ui, state: &mut (State, usize)| {
            if ui.button("Dispatch probe").clicked() {
                state.1 += 1;
            }
            state.0.controls(ui, editable);
        },
        (state, 0),
    )
}
fn click(h: &mut Harness<'_, (State, usize)>, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(2);
}
#[test]
fn actual_insert_and_moves_preserve_raw_coordinates_and_stage_only() {
    let state = ready();
    let old = state.form.clone();
    let context = state.context.clone();
    let generation = state.pick_generation;
    let mut h = harness(state, true);
    h.run_steps(2);
    click(&mut h, "Move patrol waypoint 3 up");
    click(&mut h, "Move patrol waypoint 2 up");
    click(&mut h, "Insert before patrol waypoint 2");
    assert_eq!(
        h.state().0.form.points,
        vec![
            old.points[2].clone(),
            old.points[0].clone(),
            old.points[0].clone(),
            old.points[1].clone()
        ]
    );
    assert_eq!(h.state().0.form.looped, old.looped);
    assert_eq!(h.state().0.form.wait, old.wait);
    assert_eq!(h.state().0.context, context);
    assert_eq!(h.state().0.pick_generation, generation + 3);
    assert!(h.state().0.dirty);
    assert!(!h.state().0.stale);
}
#[test]
fn structural_controls_obey_first_last_and_maximum_eight_bounds() {
    let mut state = ready();
    state.form.points.truncate(1);
    let mut h = harness(state, true);
    h.run_steps(2);
    for label in ["Move patrol waypoint 1 up", "Move patrol waypoint 1 down"] {
        assert!(h.get_by_label(label).accesskit_node().is_disabled());
    }
    h.state_mut().0.form.points = vec![["0".into(), "0".into()]; 8];
    h.run_steps(2);
    let before = h.state().0.form.clone();
    let generation = h.state().0.pick_generation;
    for label in [
        "Insert before patrol waypoint 1",
        "Insert before patrol waypoint 8",
        "Move patrol waypoint 1 up",
        "Move patrol waypoint 8 down",
    ] {
        assert!(h.get_by_label(label).accesskit_node().is_disabled());
        click(&mut h, label);
    }
    assert_eq!(h.state().0.form, before);
    assert_eq!(h.state().0.pick_generation, generation);
    assert!(!h.state().0.dirty);
}
#[test]
fn identical_row_swap_advances_identity_and_clears_preview_and_queued_pick() {
    let mut state = ready();
    state.form.points = vec![["1.000".into(), "2.0".into()]; 2];
    state.pick_request = Some((
        PatrolPickRequest::Pick(0),
        state.form.clone(),
        state.pick_generation,
    ));
    let model = orr_model::import::import_with_resolver(
        "test.glb",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../orr_model/tests/fixtures/model.glb"
        )),
        |_| panic!("embedded"),
    )
    .unwrap();
    state.inspection = Some((
        InspectionPin {
            context: context(),
            form: state.form.clone(),
            generation: state.pick_generation,
            terrain_model: std::sync::Arc::new(model),
        },
        Err("old report".into()),
    ));
    state.show_inspection = true;
    let form = state.form.clone();
    let generation = state.pick_generation;
    let mut h = harness(state, true);
    h.run_steps(2);
    click(&mut h, "Move patrol waypoint 1 down");
    assert_eq!(h.state().0.form, form);
    assert_eq!(h.state().0.pick_generation, generation + 1);
    assert!(h.state().0.dirty);
    assert!(h.state().0.pick_request.is_none());
    assert!(h.state().0.inspection.is_none());
    assert!(!h.state().0.show_inspection);
}
fn action(h: &Harness<'_, (State, usize)>, label: &str) -> egui::Event {
    let (target_node, target_tree) = h.get_by_label(label).accesskit_node().locate();
    egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
        target_node,
        target_tree,
        action: egui::accesskit::Action::Click,
        data: None,
    })
}
#[test]
fn stale_busy_saturated_and_same_frame_focus_loss_refuse_structural_commands() {
    for reason in 0..4 {
        let mut state = ready();
        if reason == 0 {
            state.stale = true;
        }
        if reason == 1 {
            state.order_busy = true;
        }
        if reason == 2 {
            state.pick_generation = u64::MAX;
        }
        let original = state.form.clone();
        let generation = state.pick_generation;
        let mut h = harness(state, reason != 3);
        h.run_steps(2);
        let label = "Move patrol waypoint 2 up";
        assert!(h.get_by_label(label).accesskit_node().is_disabled());
        click(&mut h, label);
        assert_eq!(h.state().0.form, original);
        assert_eq!(h.state().0.pick_generation, generation);
        assert!(!h.state().0.dirty);
    }
    let state = ready();
    let form = state.form.clone();
    let generation = state.pick_generation;
    let mut h = harness(state, true);
    h.run_steps(2);
    let target = action(&h, "Move patrol waypoint 2 up");
    let probe = action(&h, "Dispatch probe");
    h.input_mut().focused = true;
    h.input_mut().events.extend([
        egui::Event::WindowFocused(false),
        egui::Event::WindowFocused(true),
        probe,
        target,
    ]);
    h.step();
    assert_eq!(
        h.state().1,
        1,
        "same-frame accessibility actions reached the UI"
    );
    assert_eq!(h.state().0.form, form);
    assert_eq!(h.state().0.pick_generation, generation);
    assert!(!h.state().0.dirty);
}
