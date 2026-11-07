//! CPU widgets queue explicit requests only; no viewport/GPU/host/file mutation.
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

fn harness(editable: bool) -> Harness<'static, (State, usize)> {
    let mut state = State::default();
    state.load(context(), None);
    state.form.enabled = true;
    state.form.points = vec![["1".into(), "2".into()], ["3".into(), "4".into()]];
    let mut h = Harness::builder()
        .with_size([1200.0, 2200.0])
        .build_ui_state(
            move |ui, state: &mut (State, usize)| {
                state.1 += usize::from(state.0.controls(ui, editable));
            },
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
fn explicit_row_buttons_queue_zero_based_pick_without_mutating_draft_or_requesting_apply() {
    let mut h = harness(true);
    let before = h.state().0.form.clone();
    assert!(h.state_mut().0.take_pick_request().is_none());
    click(&mut h, "Pick terrain patrol waypoint 2");
    assert_eq!(
        h.state_mut().0.take_pick_request(),
        Some(PatrolPickRequest::Pick(1))
    );
    assert!(h.state_mut().0.take_pick_request().is_none());
    assert_eq!(h.state().0.form, before);
    assert!(!h.state().0.dirty);
    assert_eq!(h.state().1, 0);
}
#[test]
fn readonly_stale_missing_context_and_focus_loss_disable_arming_but_cancel_stays_available() {
    for case in 0..4 {
        let mut h = harness(case != 0);
        match case {
            1 => h.state_mut().0.stale = true,
            2 => h.state_mut().0.context = None,
            3 => h.input_mut().focused = false,
            _ => {}
        }
        h.step();
        assert!(
            h.get_by_label("Pick terrain patrol waypoint 1")
                .accesskit_node()
                .is_disabled(),
            "case {case}"
        );
        click(&mut h, "Pick terrain patrol waypoint 1");
        assert!(h.state_mut().0.take_pick_request().is_none());
        assert!(
            !h.get_by_label("Cancel terrain patrol waypoint pick")
                .accesskit_node()
                .is_disabled()
        );
        click(&mut h, "Cancel terrain patrol waypoint pick");
        assert_eq!(
            h.state_mut().0.take_pick_request(),
            Some(PatrolPickRequest::Cancel)
        );
        assert_eq!(h.state().1, 0);
    }
}
#[test]
fn changed_form_or_removed_row_retires_queued_identity_and_reload_clears_request() {
    for case in 0..4 {
        let mut h = harness(true);
        click(&mut h, "Pick terrain patrol waypoint 2");
        match case {
            0 => {
                h.state_mut().0.form.points.remove(0);
            }
            1 => h.state_mut().0.form.points[1][0] = "9".into(),
            2 => h.state_mut().0.form.enabled = false,
            _ => h.state_mut().0.stale = true,
        }
        assert_eq!(
            h.state_mut().0.take_pick_request(),
            Some(PatrolPickRequest::Cancel)
        );
    }
    let mut h = harness(true);
    click(&mut h, "Pick terrain patrol waypoint 1");
    h.state_mut().0.load(context(), None);
    assert!(h.state_mut().0.take_pick_request().is_none());
}

#[test]
fn remove_then_add_identical_row_cannot_reuse_prior_pick_generation() {
    let mut h = harness(true);
    h.state_mut().0.form.points = vec![["1".into(), "2".into()]; 2];
    h.run_steps(2);
    let before = h.state().0.form.clone();
    click(&mut h, "Pick terrain patrol waypoint 2");
    let generation = h.state().0.pick_generation;
    click(&mut h, "Remove terrain patrol waypoint 1");
    click(&mut h, "Add terrain patrol waypoint");
    assert_eq!(h.state().0.form, before);
    assert!(h.state().0.pick_generation > generation);
    assert_eq!(
        h.state_mut().0.take_pick_request(),
        Some(PatrolPickRequest::Cancel)
    );
    assert_eq!(h.state().1, 0);
}
