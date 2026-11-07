use super::*;
use crate::model_package_import::variant_preview::Prepared;
use egui_kittest::{kittest::Queryable, Harness};
use std::sync::Arc;
fn frame(ticket: u64) -> PreviewFrame {
    let model = orr_model::animation::AnimatedModel::from_bytes(include_bytes!(
        "../../../../assets/room_character_demo/courier.orrmodel.json"
    ))
    .unwrap();
    PreviewFrame {
        ticket,
        prepared: Arc::new(Prepared {
            model: Arc::new(model),
            asset_sha256: "a".repeat(64),
            source_sha256: "b".repeat(64),
            png_sha256: Some("c".repeat(64)),
        }),
    }
}
#[test]
fn new_ticket_starts_with_admitted_clip_zero_pose_and_finite_fitted_bounds() {
    let frame = frame(1);
    let mut window = PreviewWindow::default();
    assert!(window.sync(Some(frame.clone())));
    let accepted = window.accepted.as_ref().unwrap();
    assert_eq!((accepted.clip, accepted.seconds), (0, 0.0));
    assert_eq!(
        (&window.clip, &window.seconds),
        (&"0".to_string(), &"0".to_string())
    );
    let expected = frame.prepared.model.sample_clip(0, 0.0).unwrap();
    assert_eq!(accepted.pose.skin_matrices(), expected.skin_matrices());
    assert!(accepted.bounds.center.iter().all(|v| v.is_finite()));
    assert!(accepted.bounds.radius.is_finite() && accepted.bounds.radius > 0.0);
}
#[test]
fn invalid_samples_keep_last_accepted_pose_bounds_and_model_bits() {
    let frame = frame(1);
    let bytes = frame.prepared.model.to_bytes().unwrap();
    let mut window = PreviewWindow::default();
    window.sync(Some(frame.clone()));
    window.sample(1, 0.25).unwrap();
    let old = window.accepted.as_ref().unwrap();
    let skin = old.pose.skin_matrices().to_vec();
    let bounds = old.bounds;
    for (clip, time) in [
        (u32::MAX, 0.0),
        (0, f32::NAN),
        (0, f32::INFINITY),
        (0, -0.1),
        (0, 1e30),
    ] {
        assert!(window.sample(clip, time).is_err());
        let current = window.accepted.as_ref().unwrap();
        assert_eq!((current.clip, current.seconds), (1, 0.25));
        assert_eq!(current.pose.skin_matrices(), skin);
        assert_eq!(current.bounds, bounds);
    }
    window.clip = "bad".into();
    window.seconds = "0".into();
    assert!(!window.sample_draft());
    assert!(window.error.is_some());
    assert_eq!(frame.prepared.model.to_bytes().unwrap(), bytes);
}
#[test]
fn same_frame_keeps_view_and_pose_new_ticket_and_none_release_all_state() {
    let frame = frame(1);
    let mut window = PreviewWindow::default();
    window.sync(Some(frame.clone()));
    window.sample(1, 0.25).unwrap();
    window.camera.orbit([12.0, 7.0]);
    window.camera.zoom(2.0);
    let camera = window.camera;
    assert!(!window.sync(Some(frame.clone())));
    assert_eq!(window.camera, camera);
    assert_eq!(window.accepted.as_ref().unwrap().clip, 1);
    let mut next = frame.clone();
    next.ticket = 2;
    assert!(window.sync(Some(next)));
    assert_eq!(window.camera, PreviewCamera::default());
    assert_eq!(window.accepted.as_ref().unwrap().clip, 0);
    window.sync(None);
    assert!(
        window.frame.is_none()
            && window.accepted.is_none()
            && window.texture.is_none()
            && window.gpu.is_none()
    );
    assert_eq!(window.camera, PreviewCamera::default());
}
#[test]
fn replacement_arc_even_with_same_ticket_restarts_and_invalid_model_has_no_old_pose() {
    let mut window = PreviewWindow::default();
    window.sync(Some(frame(1)));
    window.sample(1, 0.25).unwrap();
    let mut invalid = frame(1);
    let mut source = invalid.prepared.model.source().clone();
    source.clips.clear();
    let model = orr_model::animation::AnimatedModel::new(source).unwrap();
    Arc::make_mut(&mut invalid.prepared).model = Arc::new(model);
    assert!(window.sync(Some(invalid)));
    assert!(window.accepted.is_none());
    assert!(window.error.is_some());
}
#[test]
fn actual_cpu_window_controls_sample_views_reset_and_close_without_gpu_or_mutation() {
    let input = frame(8);
    let original = input.prepared.model.to_bytes().unwrap();
    let mut h = Harness::builder()
        .with_size([1000.0, 1000.0])
        .build_ui_state(
            |ui, state: &mut (PreviewWindow, Option<PreviewFrame>, usize)| {
                if state.0.show(ui.ctx(), None, state.1.clone()) {
                    state.1 = None;
                    state.2 += 1;
                }
            },
            (PreviewWindow::default(), Some(input.clone()), 0),
        );
    h.run_steps(3);
    assert!(h
        .query_by_label(
            "A GPU is required to draw the variant preview. Pose controls remain available."
        )
        .is_some());
    for (label, value) in [
        ("Variant preview clip index", "1"),
        ("Variant preview seconds", "0.25"),
    ] {
        h.get_by_label(label).focus();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.run_steps(2);
        h.get_by_label(label).type_text(value);
        h.run_steps(3);
    }
    h.get_by_label("Sample variant pose").click_accesskit();
    h.run_steps(3);
    assert_eq!(
        (
            h.state().0.accepted.as_ref().unwrap().clip,
            h.state().0.accepted.as_ref().unwrap().seconds
        ),
        (1, 0.25)
    );
    for (index, label) in [
        "Variant front view",
        "Variant right view",
        "Variant back view",
        "Variant left view",
    ]
    .into_iter()
    .enumerate()
    {
        h.get_by_label(label).click_accesskit();
        h.run_steps(3);
        let mut expected = PreviewCamera::default();
        expected.horizontal_view(index);
        assert_eq!(h.state().0.camera, expected);
    }
    h.get_by_label("Reset variant view").click_accesskit();
    h.run_steps(3);
    assert_eq!(h.state().0.camera, PreviewCamera::default());
    assert_eq!(input.prepared.model.to_bytes().unwrap(), original);
    assert!(h.state().0.gpu.is_none());
    h.get_by_label("Close variant preview").click_accesskit();
    h.run_steps(3);
    assert_eq!(h.state().2, 1);
    assert!(
        h.state().0.frame.is_none() && h.state().0.accepted.is_none() && h.state().0.gpu.is_none()
    );
}
