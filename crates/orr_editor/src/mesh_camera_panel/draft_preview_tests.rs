use super::*;
fn context() -> Context {
    Context {
        profile: EditorGame::MeshNavigation,
        scene: PathBuf::from("mesh.scene.yaml"),
        scene_epoch: 3,
        graph_revision: [8; 32],
        host_epoch: 4,
    }
}
fn document() -> Document {
    Document::from_orbit(orr_render::OrbitCamera {
        target: [2., 3., -4.],
        yaw: 0.7,
        pitch: 0.4,
        distance: 12.,
        fov_y_degrees: 55.,
    })
    .unwrap()
}
fn enabled() -> State {
    State {
        context: Some(context()),
        busy: false,
    }
}
fn approx(a: f32, b: f32) {
    assert!((a - b).abs() < 0.0001, "{a} != {b}");
}
#[test]
fn default_off_and_explicit_clear_do_not_change_authored_form_history_or_dirty_state() {
    let mut state = State::default();
    assert!(state
        .sample(Some(context()), Ok(document()), (800, 600))
        .is_none());
    let mut panel = Panel::default();
    let before = (
        panel.target.clone(),
        panel.distance.clone(),
        panel.cursor,
        panel.saved_id,
        panel.dirty(),
    );
    panel.draft_preview = enabled();
    panel.clear_static_preview();
    assert!(!panel.static_preview_enabled());
    assert_eq!(
        before,
        (
            panel.target.clone(),
            panel.distance.clone(),
            panel.cursor,
            panel.saved_id,
            panel.dirty()
        )
    );
}
#[test]
fn perspective_and_orthographic_use_authored_orbit_and_current_aspect_without_mutation() {
    let original = document();
    for projection in [
        super::super::Projection::Perspective,
        super::super::Projection::Orthographic { half_height: 5. },
    ] {
        let mut d = original.clone();
        d.projection = projection;
        let before = d.clone();
        for size in [(800, 600), (300, 900)] {
            let c = camera(&d, size).unwrap();
            let expected = [
                2. + 12. * 0.4f32.cos() * 0.7f32.sin(),
                3. + 12. * 0.4f32.sin(),
                -4. + 12. * 0.4f32.cos() * 0.7f32.cos(),
            ];
            for (a, b) in c.eye.into_iter().zip(expected) {
                approx(a, b);
            }
            assert_eq!(c.target, d.target);
            let center = c.world_to_screen(d.target, size).unwrap();
            approx(center[0], size.0 as f32 / 2.);
            approx(center[1], size.1 as f32 / 2.);
            match (projection, c.projection) {
                (
                    super::super::Projection::Perspective,
                    orr_render::Projection::Perspective { fov_y, near, far },
                ) => {
                    approx(fov_y, 55f32.to_radians());
                    assert_eq!((near, far), (d.near, d.far));
                }
                (
                    super::super::Projection::Orthographic { half_height },
                    orr_render::Projection::Orthographic {
                        half_height: actual,
                        ..
                    },
                ) => {
                    let aspect = size.0 as f32 / size.1 as f32;
                    approx(actual, half_height / aspect.min(1.));
                }
                _ => panic!("projection mismatch"),
            };
            assert_eq!(d, before);
        }
    }
    assert_eq!(original, document());
}
#[test]
fn live_valid_draft_changes_update_preview_without_rearming() {
    let mut state = enabled();
    let first = state
        .sample(Some(context()), Ok(document()), (800, 600))
        .unwrap();
    let mut changed = document();
    changed.target = [-6., 7., 8.];
    changed.yaw = -0.3;
    changed.distance = 20.;
    let next = state
        .sample(Some(context()), Ok(changed.clone()), (800, 600))
        .unwrap();
    assert_ne!(first.eye, next.eye);
    assert_eq!(next.target, changed.target);
    assert!(state.context.is_some());
}
#[test]
fn invalid_draft_follow_or_bad_viewport_retires_until_explicit_reenable() {
    let mut follow = document();
    follow.follow_offset = Some([0.; 3]);
    let mut invalid = document();
    invalid.distance = f32::NAN;
    for (candidate, size) in [
        (Ok(follow), (800, 600)),
        (Ok(invalid), (800, 600)),
        (Err("invalid raw form".into()), (800, 600)),
        (Ok(document()), (0, 600)),
    ] {
        let mut state = enabled();
        assert!(state.sample(Some(context()), candidate, size).is_none());
        assert!(state
            .sample(Some(context()), Ok(document()), (800, 600))
            .is_none());
    }
}
#[test]
fn source_context_aba_retires_and_busy_guard_does_not_mutate_active_preview() {
    let mut changes = Vec::new();
    let mut c = context();
    c.profile = EditorGame::Arena;
    changes.push(c);
    let mut c = context();
    c.scene = PathBuf::from("other");
    changes.push(c);
    let mut c = context();
    c.scene_epoch += 1;
    changes.push(c);
    let mut c = context();
    c.graph_revision[0] += 1;
    changes.push(c);
    let mut c = context();
    c.host_epoch += 1;
    changes.push(c);
    for changed in changes.into_iter().map(Some).chain([None]) {
        let mut state = enabled();
        assert!(state.sample(changed, Ok(document()), (800, 600)).is_none());
        assert!(state
            .sample(Some(context()), Ok(document()), (800, 600))
            .is_none());
    }
    let mut panel = Panel::default();
    panel.draft_preview = enabled();
    panel.set_static_preview_busy(true);
    assert!(panel.static_preview_busy());
    assert!(panel.static_preview_enabled());
    panel.clear_static_preview();
    assert!(panel.static_preview_busy());
    assert!(!panel.static_preview_enabled());
}
