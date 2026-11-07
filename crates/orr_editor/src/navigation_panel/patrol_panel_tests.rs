//! GPU-free staged form tests. Source/Editor transaction acceptance is Linux-only.
use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
fn harness(editable: bool) -> Harness<'static, (State, usize)> {
    Harness::builder()
        .with_size([1200.0, 1800.0])
        .build_ui_state(
            move |ui, state: &mut (State, usize)| {
                state.1 += usize::from(state.0.controls(ui, editable))
            },
            (State::default(), 0usize),
        )
}
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
#[test]
fn actual_controls_only_stage_waypoints_loop_and_explicit_apply_request() {
    let mut h = harness(true);
    h.run_steps(2);
    assert!(!h.state().0.form.enabled);
    h.get_by_label("Enable terrain waypoint patrol")
        .click_accesskit();
    h.run_steps(2);
    h.get_by_label("Add terrain patrol waypoint")
        .click_accesskit();
    h.run_steps(2);
    assert_eq!(h.state().0.form.points.len(), 2);
    h.get_by_label("Loop terrain patrol").click_accesskit();
    h.run_steps(2);
    assert!(h.state().0.form.looped);
    h.get_by_label("Remove terrain patrol waypoint 1")
        .click_accesskit();
    h.run_steps(2);
    assert_eq!(h.state().0.form.points.len(), 1);
    h.get_by_label("Apply terrain waypoint patrol")
        .click_accesskit();
    h.run_steps(2);
    assert_eq!(h.state().1, 1);
    assert!(h.state().0.context.is_none());
}
#[test]
fn readonly_and_stale_form_disables_all_mutation_requests() {
    for (editable, stale, focused) in [
        (false, false, true),
        (true, true, true),
        (true, false, false),
    ] {
        let mut h = harness(editable);
        h.state_mut().0.stale = stale;
        if !focused {
            h.event(egui::Event::WindowFocused(false));
        }
        h.run_steps(2);
        assert!(h
            .get_by_label("Enable terrain waypoint patrol")
            .accesskit_node()
            .is_disabled());
        h.get_by_label("Enable terrain waypoint patrol")
            .click_accesskit();
        h.run_steps(2);
        assert!(!h.state().0.form.enabled);
        assert_eq!(h.state().1, 0);
    }
}
#[test]
fn strict_wait_count_and_fixed_point_coordinate_parsing_preserves_invalid_drafts() {
    let mut form = Form {
        enabled: true,
        ..Default::default()
    };
    for value in ["-1", "3601", "1.5", "bad"] {
        form.wait = value.into();
        let before = form.clone();
        assert!(parse(&form, [3; 32]).is_err());
        assert_eq!(form, before);
    }
    form.wait = "3600".into();
    form.points[0] = ["-1.25".into(), "0.5".into()];
    let parsed = parse(&form, [3; 32]).unwrap();
    assert_eq!(parsed.wait_ticks, 3600);
    assert_eq!(
        parsed.points[0],
        [parse_fixed("-1.25").unwrap(), parse_fixed("0.5").unwrap()]
    );
    form.points[0][0] = "NaN".into();
    assert!(parse(&form, [3; 32]).is_err());
    form.points.clear();
    assert!(parse(&form, [3; 32]).is_err());
    form.points = vec![["0".into(), "0".into()]; 9];
    assert!(parse(&form, [3; 32]).is_err());
}
#[test]
fn disabled_form_is_canonical_zero_and_eight_point_form_disables_add() {
    let form = Form {
        wait: "bad".into(),
        ..Default::default()
    };
    let config = parse(&form, [3; 32]).unwrap();
    assert_eq!(
        (
            config.enabled,
            config.count,
            config.looped,
            config.wait_ticks
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(config.points, [[FP::ZERO; 2]; 8]);
    assert_eq!(config.graph_revision, [0; 32]);
    let mut h = harness(true);
    h.state_mut().0.form.enabled = true;
    h.state_mut().0.form.points = vec![["0".into(), "0".into()]; 8];
    h.run_steps(2);
    assert!(h
        .get_by_label("Add terrain patrol waypoint")
        .accesskit_node()
        .is_disabled());
}
#[test]
fn dirty_scene_epoch_checksum_terrain_graph_source_and_agent_changes_require_reload() {
    let original = context();
    for change in 0..7 {
        let mut state = State::default();
        state.load(original.clone(), None);
        state.form.wait = "12".into();
        state.dirty = true;
        let before = state.form.clone();
        let mut next = original.clone();
        match change {
            0 => next.scene = "other.yaml".into(),
            1 => next.epoch += 1,
            2 => next.checksum += 1,
            3 => next.pin.terrain_revision[0] ^= 1,
            4 => next.pin.graph_revision[0] ^= 1,
            5 => next.pin.source_path[0] ^= 1,
            _ => next.pin.agent.start[0] = FP::ONE,
        };
        state.sync_context(Some(&next));
        assert!(state.stale);
        assert_eq!(state.form, before);
        state.sync_context(Some(&original));
        assert!(state.stale);
        state.load(original.clone(), None);
        assert!(!state.dirty && !state.stale);
    }
}
