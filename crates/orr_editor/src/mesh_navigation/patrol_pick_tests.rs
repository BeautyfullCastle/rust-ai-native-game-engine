use super::*;
fn p(x: i32, z: i32) -> [FP; 3] {
    [FP::from_int(x), FP::ZERO, FP::from_int(z)]
}
fn fixture() -> (State, Context, MeshScenePin) {
    let g = StaticMeshGraph::bake(
        &[p(0, 0), p(0, 4), p(4, 0)],
        &[[0, 1, 2]],
        AgentProfile::default(),
    )
    .unwrap();
    let pin = MeshScenePin::new(
        &g,
        AgentSpec {
            start: p(1, 1),
            goal: p(2, 1),
            max_vertical: FP::ONE,
            distance_per_tick: FP::ONE,
        },
    )
    .unwrap();
    let context = Context {
        scene: "scene.yaml".into(),
        epoch: 1,
        checksum: 2,
        revision: g.revision(),
    };
    let mut s = State::default();
    s.context = Some(context.clone());
    s.form.enabled = true;
    s.form.points = vec![
        ["1".into(), "0".into(), "1".into()],
        ["2".into(), "0".into(), "1".into()],
    ];
    (s, context, pin)
}
fn token(s: &State, c: Context, pin: MeshScenePin, row: usize) -> Token {
    capture(
        s,
        c,
        row,
        3,
        pin,
        ["mesh.asset".into(), "model.asset".into()],
    )
    .unwrap()
}
#[test]
fn chosen_xyz_row_only_stages_and_retires_whole_route_inspection() {
    let (mut s, c, pin) = fixture();
    s.form.looped = true;
    s.form.wait = "19".into();
    s.show_inspection = true;
    let before = s.form.clone();
    let t = token(&s, c, pin, 1);
    stage(&mut s, t.clone(), &t, p(3, 0)).unwrap();
    assert_eq!(s.form.points[0], before.points[0]);
    assert_eq!(s.form.points[1], ["3", "0", "0"].map(str::to_owned));
    assert_eq!(s.form.looped, before.looped);
    assert_eq!(s.form.wait, before.wait);
    assert!(s.dirty);
    assert_eq!(s.generation, 1);
    assert!(!s.show_inspection);
    assert!(s.inspection.is_none());
}
#[test]
fn full_form_changes_cannot_retarget_an_old_row() {
    for change in 0..6 {
        let (mut s, c, pin) = fixture();
        let old = token(&s, c.clone(), pin, 1);
        match change {
            0 => {
                s.form.points.remove(0);
            }
            1 => s.form.points.reverse(),
            2 => s.form.points[0][0] = "new".into(),
            3 => s.form.looped = true,
            4 => s.form.wait = "1".into(),
            _ => s.form.enabled = false,
        };
        let before = s.form.clone();
        let current = capture(
            &s,
            c,
            1,
            3,
            pin,
            ["mesh.asset".into(), "model.asset".into()],
        );
        if let Ok(current) = current {
            assert!(stage(&mut s, current, &old, p(1, 1)).is_err());
        }
        assert_eq!(s.form, before);
        assert!(!s.dirty);
    }
}
#[test]
fn scene_host_frame_graph_source_and_entire_pin_are_part_of_identity() {
    for change in 0..8 {
        let (mut s, c, pin) = fixture();
        let old = token(&s, c, pin, 0);
        let mut current = old.clone();
        match change {
            0 => current.context.scene = "other.yaml".into(),
            1 => current.context.epoch += 1,
            2 => current.host_epoch += 1,
            3 => current.context.checksum += 1,
            4 => current.context.revision[0] ^= 1,
            5 => current.source[0] = "other.asset".into(),
            6 => current.pin.max_vertical = FP::ZERO,
            _ => current.pin.goal = p(1, 2),
        };
        let before = s.form.clone();
        assert!(stage(&mut s, current, &old, p(1, 1)).is_err());
        assert_eq!(s.form, before);
        assert!(!s.dirty);
    }
}
#[test]
fn identical_form_after_row_replacement_or_success_never_revives_old_token() {
    let (mut s, c, pin) = fixture();
    let old = token(&s, c.clone(), pin, 0);
    let form = s.form.clone();
    s.form.points.remove(0);
    s.form = form;
    s.generation += 1;
    let current = token(&s, c.clone(), pin, 0);
    assert!(stage(&mut s, current.clone(), &old, p(1, 1)).is_err());
    stage(&mut s, current.clone(), &current, p(1, 1)).unwrap();
    let now = token(&s, c, pin, 0);
    assert!(stage(&mut s, now, &current, p(1, 1)).is_err());
}
#[test]
fn invalid_rows_stale_disabled_exhausted_and_out_of_bounds_fail_closed() {
    let (mut s, c, pin) = fixture();
    let source = ["mesh.asset".into(), "model.asset".into()];
    assert!(capture(&s, c.clone(), 2, 3, pin, source.clone()).is_err());
    let t = token(&s, c.clone(), pin, 0);
    let before = s.form.clone();
    assert!(stage(&mut s, t.clone(), &t, p(257, 0)).is_err());
    assert_eq!(s.form, before);
    s.stale = true;
    assert!(capture(&s, c.clone(), 0, 3, pin, source.clone()).is_err());
    s.stale = false;
    s.form.enabled = false;
    assert!(capture(&s, c.clone(), 0, 3, pin, source.clone()).is_err());
    s.form.enabled = true;
    s.generation = u64::MAX;
    assert!(capture(&s, c, 0, 3, pin, source).is_err());
}
