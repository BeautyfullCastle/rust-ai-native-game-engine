//! CPU widget dispatch and worker admission; no native window or GPU.
use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
fn harness(p: Panel) -> Harness<'static, Panel> {
    let mut h = Harness::builder()
        .with_size([1500.0, 3000.0])
        .build_ui_state(
            |ui, p: &mut Panel| p.show(ui, true, Some(Path::new("/project")), 4, 7),
            p,
        );
    h.run_steps(2);
    click(&mut h, "Import a model package");
    h
}
fn click(h: &mut Harness<'_, Panel>, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(2);
}
#[test]
fn sequence_default_off_feature_gate_exclusive_modes_and_format_reset() {
    let mut h = harness(Panel::default());
    assert!(!h.state().sequence.enabled);
    assert!(h.query_by_label("Create animation clip sequence").is_none());
    if cfg!(feature = "room-animated-models") {
        click(&mut h, "Cooked animated model");
        for other in [
            "Create animated color variant",
            "Create animated PNG skin",
            "Create trimmed animation clip",
            "Create animated sampler variant",
        ] {
            click(&mut h, "Create animation clip sequence");
            assert!(h.state().sequence.enabled);
            click(&mut h, other);
            assert!(!h.state().sequence.enabled);
        }
        click(&mut h, "Create animation clip sequence");
        assert!(
            !h.state().color_variant_enabled
                && !h.state().texture_variant_enabled
                && !h.state().trim.enabled
                && !h.state().sampler.enabled
        );
        assert!(h.state().sequence_request().is_err());
        click(&mut h, "Raw GLB");
        assert_eq!(h.state().sequence, Controls::default());
    } else {
        assert!(h
            .get_by_label("Cooked animated model")
            .accesskit_node()
            .is_disabled());
    }
}
#[cfg(feature = "room-animated-models")]
#[test]
fn missing_catalog_refuses_workers_and_held_worker_disables_sequence_controls() {
    let mut p = Panel::default();
    p.source_format = SourceFormat::CookedAnimated;
    p.kind = ModelKind::Animated;
    p.source = "/input.animated.json".into();
    p.sequence.enabled = true;
    p.sequence.first = Some(0);
    p.sequence.second = Some(0);
    assert!(p.sequence_request().is_err());
    p.start_import("/project".into(), 4, 7);
    assert!(!p.busy());
    let (sender, receiver) = mpsc::channel();
    p.pending = Some(receiver);
    let mut h = harness(p);
    for label in [
        "Create animation clip sequence",
        "Sequence first clip",
        "Sequence second clip",
        "Sequence transition seconds",
        "New sequence clip name",
        "Sequence variant asset ID",
    ] {
        assert!(
            h.get_by_label(label).accesskit_node().is_disabled(),
            "{label}"
        );
    }
    drop(sender);
    let _ = h.state_mut().poll(Some(Path::new("/project")), 4, 7);
    assert!(!h.state().busy());
}
#[cfg(all(feature = "room-animated-models", target_os = "linux"))]
#[test]
fn actual_catalog_two_clip_selection_pins_both_hashes_and_rejects_invalid_ranges_names_and_ids() {
    use std::time::{Duration, Instant};
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("courier.animated.json");
    let bytes = include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json");
    std::fs::write(&source, bytes).unwrap();
    let mut p = Panel::default();
    p.source_format = SourceFormat::CookedAnimated;
    p.kind = ModelKind::Animated;
    p.source = source.to_string_lossy().into();
    let mut h = harness(p);
    click(&mut h, "Create animation clip sequence");
    click(&mut h, "Inspect animated source");
    let deadline = Instant::now() + Duration::from_secs(5);
    while h.state().busy() {
        let _ = h.state_mut().poll(Some(Path::new("/project")), 4, 7);
        h.run_steps(2);
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    click(&mut h, "Sequence first clip");
    click(&mut h, "Sequence first clip 0: searching (2.000s)");
    assert!(h.state().sequence_request().is_err());
    click(&mut h, "Sequence second clip");
    click(&mut h, "Sequence second clip 2: escaped (1.000s)");
    let request = h.state().sequence_request().unwrap().unwrap();
    assert_eq!((request.first_clip, request.second_clip), (0, 2));
    assert_eq!(request.transition_seconds, 0.25);
    assert_eq!(h.state().sequence.first_hash, h.state().inspected_hash);
    assert_eq!(h.state().sequence.second_hash, h.state().inspected_hash);
    let before = h.state().authoring_generation;
    for value in ["0.5", "0.25"] {
        h.get_by_label("Sequence transition seconds").focus();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.run_steps(2);
        h.get_by_label("Sequence transition seconds")
            .type_text(value);
        h.run_steps(2);
        assert_eq!(
            h.get_by_label("Sequence transition seconds")
                .accesskit_node()
                .value()
                .as_deref(),
            Some(value)
        );
    }
    assert!(h.state().authoring_generation > before);
    let original = h.state().sequence.clone();
    for value in ["", "NaN", "inf", "0", "-1", "1e-99", "60.00000001"] {
        h.state_mut().sequence.transition = value.into();
        assert!(h.state().sequence_request().is_err(), "{value}");
    }
    h.state_mut().sequence = original.clone();
    for value in ["", "searching", "bad\nname"] {
        h.state_mut().sequence.name = value.into();
        assert!(h.state().sequence_request().is_err());
    }
    h.state_mut().sequence = original.clone();
    h.state_mut().sequence.asset_id = "../bad.sequence.json".into();
    assert!(h.state().sequence_request().is_err());
    h.state_mut().sequence = original;
    h.state_mut().sequence.first_hash = Some("b".repeat(64));
    assert!(h.state().sequence_request().is_err());
    assert_eq!(std::fs::read(source).unwrap().as_slice(), bytes);
    assert!(!h.state().license_acknowledged);
    assert!(h.state().destination_identity.is_none());
}
