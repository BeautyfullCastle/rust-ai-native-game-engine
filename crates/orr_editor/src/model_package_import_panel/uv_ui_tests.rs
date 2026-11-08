//! Actual CPU widgets plus strict decimal admission; no window/GPU.
use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
fn harness(p: Panel) -> Harness<'static, Panel> {
    let mut h = Harness::builder()
        .with_size([1500.0, 3200.0])
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
fn uv_default_off_feature_gate_exclusive_modes_and_format_reset() {
    let mut h = harness(Panel::default());
    assert!(!h.state().uv.enabled);
    assert!(h.query_by_label("Create animated UV variant").is_none());
    if cfg!(feature = "room-animated-models") {
        click(&mut h, "Cooked animated model");
        for other in [
            "Create animated color variant",
            "Create animated PNG skin",
            "Create trimmed animation clip",
            "Create animated sampler variant",
            "Create animation clip sequence",
        ] {
            click(&mut h, "Create animated UV variant");
            assert!(h.state().uv.enabled);
            click(&mut h, other);
            assert!(!h.state().uv.enabled);
        }
        click(&mut h, "Create animated UV variant");
        assert!(
            !h.state().sequence.enabled
                && !h.state().sampler.enabled
                && !h.state().trim.enabled
                && !h.state().color_variant_enabled
                && !h.state().texture_variant_enabled
        );
        click(&mut h, "Raw GLB");
        assert_eq!(h.state().uv, Controls::default());
    } else {
        assert!(h
            .get_by_label("Cooked animated model")
            .accesskit_node()
            .is_disabled());
    }
}
#[cfg(feature = "room-animated-models")]
#[test]
fn decimal_bounds_underflow_missing_catalog_and_busy_fields_fail_closed() {
    for raw in ["", "NaN", "inf", "3.5e38", "-3.5e38"] {
        assert!(number(raw, false).is_err());
    }
    for raw in ["0", "-1", "-1e-99", "1e-99", "3.5e38"] {
        assert!(number(raw, true).is_err());
    }
    assert_eq!(number("65536", true).unwrap(), 65536.0);
    assert_eq!(number("1000000", true).unwrap(), 1000000.0);
    assert_eq!(number("-1000000", false).unwrap(), -1000000.0);
    assert!(number(&f64::from(f32::MAX).to_string(), true).is_ok());
    assert_eq!(number("-65536", false).unwrap(), -65536.0);
    let mut p = Panel::default();
    p.source_format = SourceFormat::CookedAnimated;
    p.kind = ModelKind::Animated;
    p.source = "/input.animated.json".into();
    p.uv.enabled = true;
    p.uv.material = Some(0);
    assert!(p.uv_request().is_err());
    p.start_import("/project".into(), 4, 7);
    assert!(!p.busy());
    let (sender, receiver) = mpsc::channel();
    p.pending = Some(receiver);
    let mut h = harness(p);
    for label in [
        "Create animated UV variant",
        "UV offset U",
        "UV offset V",
        "UV scale U",
        "UV scale V",
        "UV quarter-turn rotation",
        "UV variant asset ID",
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
fn inspected_used_material_uv_widgets_pin_hash_generation_and_reject_bad_ids_rotations() {
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
    click(&mut h, "Create animated UV variant");
    click(&mut h, "Inspect animated source");
    let deadline = Instant::now() + Duration::from_secs(5);
    while h.state().busy() {
        let _ = h.state_mut().poll(Some(Path::new("/project")), 4, 7);
        h.run_steps(2);
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    click(&mut h, "Use material 0");
    let generation = h.state().authoring_generation;
    let hash = h.state().inspected_hash.clone();
    for (label, value) in [
        ("UV offset U", "0.25"),
        ("UV offset V", "-0.5"),
        ("UV scale U", "2"),
        ("UV scale V", "0.5"),
    ] {
        h.get_by_label(label).focus();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.run_steps(2);
        h.get_by_label(label).type_text(value);
        h.run_steps(2);
        assert_eq!(
            h.get_by_label(label).accesskit_node().value().as_deref(),
            Some(value)
        );
    }
    click(&mut h, "UV quarter-turn rotation");
    click(&mut h, "90 degrees");
    let request = h.state().uv_request().unwrap().unwrap();
    assert_eq!(request.offset, [0.25, -0.5]);
    assert_eq!(request.scale, [2.0, 0.5]);
    assert_eq!(request.quarter_turns, 1);
    assert_eq!(h.state().inspected_hash, hash);
    assert!(h.state().authoring_generation > generation);
    assert!(!h.state().license_acknowledged);
    assert!(h.state().destination_identity.is_none());
    assert_eq!(std::fs::read(source).unwrap().as_slice(), bytes);
    h.state_mut().uv.quarter_turns = 4;
    assert!(h.state().uv_request().is_err());
    h.state_mut().uv.quarter_turns = 1;
    h.state_mut().uv.asset_id = "../bad.uv.json".into();
    assert!(h.state().uv_request().is_err());
    h.state_mut().uv.asset_id = "valid.uv.json".into();
    h.state_mut().uv.material = Some(u32::MAX);
    assert!(h.state().uv_request().is_err());
}
