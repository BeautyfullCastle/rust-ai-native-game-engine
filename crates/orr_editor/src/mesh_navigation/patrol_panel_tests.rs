//! CPU-only staged form tests. Real Editor transaction/save/reopen acceptance
//! lives separately; these controls never create a host or write files.
use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
fn harness(editable: bool) -> Harness<'static, (State, usize)> {
    Harness::builder()
        .with_size([1300.0, 1800.0])
        .build_ui_state(
            move |ui, state: &mut (State, usize)| {
                state.1 += usize::from(state.0.controls(ui, editable))
            },
            (State::default(), 0usize),
        )
}
#[test]
fn actual_waypoint_add_remove_loop_and_apply_buttons_only_stage_form_requests() {
    let mut h = harness(true);
    h.run_steps(2);
    assert!(!h.state().0.form.enabled);
    assert_eq!(h.state().1, 0);
    h.get_by_label("Enable waypoint patrol").click_accesskit();
    h.run_steps(2);
    assert!(h.state().0.dirty);
    h.get_by_label("Add patrol waypoint").click_accesskit();
    h.run_steps(2);
    assert_eq!(h.state().0.form.points.len(), 2);
    h.get_by_label("Loop patrol").click_accesskit();
    h.run_steps(2);
    assert!(h.state().0.form.looped);
    h.get_by_label("Remove patrol waypoint 1").click_accesskit();
    h.run_steps(2);
    assert_eq!(h.state().0.form.points.len(), 1);
    h.get_by_label("Apply waypoint patrol").click_accesskit();
    h.run_steps(2);
    assert_eq!(h.state().1, 1);
    assert!(h.state().0.context.is_none());
}
#[test]
fn readonly_or_stale_context_disables_patrol_fields_and_does_not_apply() {
    for stale in [false, true] {
        let mut h = harness(stale);
        h.state_mut().0.stale = stale;
        h.run_steps(2);
        assert!(h
            .get_by_label("Enable waypoint patrol")
            .accesskit_node()
            .is_disabled());
        h.get_by_label("Enable waypoint patrol").click_accesskit();
        h.run_steps(2);
        assert!(!h.state().0.form.enabled);
        assert_eq!(h.state().1, 0);
    }
}
#[test]
fn count_and_wait_bounds_and_fixed_point_errors_reject_without_changing_form() {
    let mut form = Form {
        enabled: true,
        ..Default::default()
    };
    let original = form.clone();
    for wait in ["-1", "3601", "1.5", "bad"] {
        form.wait = wait.into();
        let before = form.clone();
        assert!(parse(&form, [7; 32]).is_err());
        assert_eq!(form, before);
    }
    form = original.clone();
    form.points.clear();
    assert!(parse(&form, [7; 32]).is_err());
    form.points = vec![["1".into(), "0".into(), "1".into()]; 9];
    assert!(parse(&form, [7; 32]).is_err());
    form = original;
    form.wait = "3600".into();
    form.points[0][0] = "-1.25".into();
    let config = parse(&form, [7; 32]).unwrap();
    assert_eq!(config.points[0][0], parse_fixed("-1.25").unwrap());
    assert_eq!(config.wait_ticks, 3600);
    assert_eq!(config.graph_revision, [7; 32]);
    form.points[0][1] = "NaN".into();
    assert!(parse(&form, [7; 32]).is_err());
    form.points[0][1] = "257".into();
    assert!(parse(&form, [7; 32]).is_err());
}
#[test]
fn disabled_patrol_is_canonical_zero_and_eight_point_limit_disables_add() {
    let form = Form {
        enabled: false,
        wait: "invalid".into(),
        ..Default::default()
    };
    let disabled = parse(&form, [8; 32]).unwrap();
    assert_eq!(disabled.enabled, 0);
    assert_eq!(disabled.count, 0);
    assert_eq!(disabled.wait_ticks, 0);
    assert_eq!(disabled.points, [[FP::ZERO; 3]; 8]);
    assert_eq!(disabled.graph_revision, [0; 32]);
    let mut h = harness(true);
    h.state_mut().0.form.enabled = true;
    h.state_mut().0.form.points = vec![["0".into(), "0".into(), "0".into()]; 8];
    h.run_steps(2);
    assert!(h
        .get_by_label("Add patrol waypoint")
        .accesskit_node()
        .is_disabled());
}
#[test]
fn dirty_context_change_requires_explicit_reload_and_never_revives_old_draft() {
    let context = Context {
        scene: "scene.yaml".into(),
        epoch: 1,
        checksum: 2,
        revision: [3; 32],
    };
    for change in 0..5 {
        let mut state = State::default();
        state.load(context.clone(), None, [FP::ZERO; 3]);
        state.form.wait = "12".into();
        state.dirty = true;
        let before = state.form.clone();
        let mut other = context.clone();
        match change {
            0 => other.scene = "other.yaml".into(),
            1 => other.epoch += 1,
            2 => other.checksum += 1,
            3 => other.revision[0] ^= 1,
            _ => {
                state.sync_context(None);
                assert!(state.stale);
                assert_eq!(state.form, before);
                continue;
            }
        }
        state.sync_context(Some(&other));
        assert!(state.stale);
        assert_eq!(state.form, before);
        state.sync_context(Some(&context));
        assert!(state.stale);
        state.load(context.clone(), None, [FP::ZERO; 3]);
        assert!(!state.stale && !state.dirty);
    }
}
