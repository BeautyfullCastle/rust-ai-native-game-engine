use super::*;
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

fn state() -> State {
    let mut state = State::default();
    state.context = Some(context());
    state.form.enabled = true;
    state.form.points = vec![["1".into(), "2".into()], ["3".into(), "4".into()]];
    state
}
#[test]
fn chosen_row_only_changes_and_other_policy_is_preserved() {
    let mut s = state();
    s.form.looped = true;
    s.form.wait = "19".into();
    let before = s.form.clone();
    let token = capture(&s, context(), 1).unwrap();
    assert_eq!(token.row, 1);
    stage(
        &mut s,
        context(),
        &token,
        [FP::from_raw(-1), FP::from_int(7)],
    )
    .unwrap();
    assert_eq!(s.form.points[0], before.points[0]);
    assert_eq!(
        s.form.points[1],
        [format_fixed(FP::from_raw(-1)), "7".to_owned()]
    );
    assert_eq!(s.form.looped, before.looped);
    assert_eq!(s.form.wait, before.wait);
    assert!(s.dirty);
}
#[test]
fn every_draft_change_invalidates_captured_row_without_mutation() {
    for change in 0..6 {
        let mut s = state();
        let token = capture(&s, context(), 1).unwrap();
        match change {
            0 => {
                s.form.points.remove(0);
            }
            1 => {
                s.form.points.reverse();
            }
            2 => {
                s.form.points[0][0] = "changed".into();
            }
            3 => {
                s.form.looped = true;
            }
            4 => {
                s.form.wait = "1".into();
            }
            _ => {
                s.form.enabled = false;
            }
        };
        let before = s.form.clone();
        assert!(stage(&mut s, context(), &token, [FP::ZERO; 2]).is_err());
        assert_eq!(s.form, before);
        assert!(!s.dirty);
    }
}
#[test]
fn scene_epoch_checksum_terrain_and_graph_changes_reject_old_token() {
    for change in 0..6 {
        let mut s = state();
        let token = capture(&s, context(), 0).unwrap();
        let mut next = context();
        match change {
            0 => next.scene = "other.yaml".into(),
            1 => next.epoch += 1,
            2 => next.checksum += 1,
            3 => next.pin.terrain_revision[0] ^= 1,
            4 => next.pin.graph_revision[0] ^= 1,
            _ => next.pin.agent.goal[0] = FP::from_int(4),
        };
        s.context = Some(next.clone());
        let before = s.form.clone();
        assert!(stage(&mut s, next, &token, [FP::ZERO; 2]).is_err());
        assert_eq!(s.form, before);
    }
}
#[test]
fn missing_stale_disabled_or_removed_rows_cannot_arm_or_stage() {
    let mut s = state();
    assert!(capture(&s, context(), 2).is_err());
    let token = capture(&s, context(), 0).unwrap();
    s.stale = true;
    assert!(capture(&s, context(), 0).is_err());
    assert!(stage(&mut s, context(), &token, [FP::ZERO; 2]).is_err());
    s.stale = false;
    s.context = None;
    assert!(capture(&s, context(), 0).is_err());
    s.context = Some(context());
    s.form.enabled = false;
    assert!(capture(&s, context(), 0).is_err());
    s.form.enabled = true;
    s.form.points.clear();
    assert!(capture(&s, context(), 0).is_err());
}

#[test]
fn identical_form_after_edit_or_reload_cannot_revive_token_and_exhaustion_fails_closed() {
    let mut s = state();
    let token = capture(&s, context(), 0).unwrap();
    let form = s.form.clone();
    s.form.points.remove(0);
    s.form = form;
    s.pick_generation += 1;
    let before = s.form.clone();
    assert!(stage(&mut s, context(), &token, [FP::ZERO; 2]).is_err());
    assert_eq!(s.form, before);
    let token = capture(&s, context(), 0).unwrap();
    stage(&mut s, context(), &token, [FP::ZERO; 2]).unwrap();
    assert!(stage(&mut s, context(), &token, [FP::ZERO; 2]).is_err());
    s.pick_generation = u64::MAX;
    assert!(capture(&s, context(), 0).is_err());
}
