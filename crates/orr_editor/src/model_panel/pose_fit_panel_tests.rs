//! GPU-free control/validation tests; eligible flags below model the production
//! candidate/selection guard without constructing or publishing a fake asset.
use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
#[test]
fn pose_time_rejects_nonfinite_negative_underflow_and_values_just_beyond_clip_before_narrowing() {
    for text in [
        "",
        "NaN",
        "inf",
        "-1e-99",
        "-1",
        "2.000000000000001",
        "1e100",
    ] {
        assert!(parse_pose_seconds(text, 2.0).is_err(), "{text}");
    }
    assert_eq!(parse_pose_seconds("0", 2.0).unwrap(), 0.0);
    assert_eq!(parse_pose_seconds(" 0.75 ", 2.0).unwrap(), 0.75);
    assert_eq!(parse_pose_seconds("2", 2.0).unwrap(), 2.0);
}
#[test]
fn static_missing_or_mismatched_candidate_cannot_enable_pose_fitting() {
    let mut panel = ModelPanel::default();
    assert_eq!(panel.sample_seconds, "0");
    assert!(!panel.pose_fit_candidate_matches());
    panel.kind = ModelKind::Animated;
    assert!(!panel.pose_fit_candidate_matches());
    panel.package = "changed-source".into();
    assert!(!panel.pose_fit_candidate_matches());
}
#[test]
fn actual_pose_buttons_emit_only_explicit_requests_and_leave_transform_animation_cache_and_bindings_untouched(
) {
    let panel = ModelPanel::default();
    let transform = panel.transform;
    let animation = panel.animation;
    let mut h = Harness::builder()
        .with_size([1000.0, 600.0])
        .build_ui_state(
            |ui, state: &mut (ModelPanel, Vec<bool>)| {
                if let Some(request) = state.0.show_pose_fit_controls(ui, true, true) {
                    state.1.push(request);
                }
            },
            (panel, Vec::new()),
        );
    h.run_steps(2);
    assert!(h.query_by_label("Fit pose seconds").is_some());
    assert!(h.state().1.is_empty());
    for label in ["Fit sampled pose to box", "Fit sampled pose to sphere"] {
        h.get_by_label(label).click_accesskit();
        h.run_steps(2);
    }
    assert_eq!(h.state().1, vec![false, true]);
    assert_eq!(h.state().0.transform, transform);
    assert_eq!(h.state().0.animation, animation);
    assert!(h.state().0.candidate.is_none());
    assert!(h.state().0.loaded.is_empty());
    assert!(h.state().0.bindings.is_none());
}
#[test]
fn missing_selection_or_play_guard_disables_pose_controls_without_mutation() {
    for (eligible, editable) in [(false, true), (true, false)] {
        let mut h = Harness::builder()
            .with_size([1000.0, 600.0])
            .build_ui_state(
                move |ui, state: &mut (ModelPanel, usize)| {
                    state.1 += usize::from(
                        state
                            .0
                            .show_pose_fit_controls(ui, eligible, editable)
                            .is_some(),
                    );
                },
                (ModelPanel::default(), 0),
            );
        h.run_steps(2);
        for label in ["Fit sampled pose to box", "Fit sampled pose to sphere"] {
            assert!(h.get_by_label(label).accesskit_node().is_disabled());
            h.get_by_label(label).click_accesskit();
            h.run_steps(2);
        }
        if !editable {
            assert!(h
                .get_by_label("Fit pose seconds")
                .accesskit_node()
                .is_disabled());
        }
        assert_eq!(h.state().1, 0);
        assert_eq!(h.state().0.transform, LocalTransform::default());
        assert_eq!(h.state().0.sample_seconds, "0");
    }
}
