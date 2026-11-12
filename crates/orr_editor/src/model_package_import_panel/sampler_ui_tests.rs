//! CPU widgets and admission checks. No native window/GPU claim.
use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
fn harness(p: Panel) -> Harness<'static, Panel> {
    let mut h = Harness::builder()
        .with_size([1500.0, 2800.0])
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
fn sampler_default_off_feature_gate_and_exclusive_modes_reset() {
    let mut h = harness(Panel::default());
    assert!(!h.state().sampler.enabled);
    assert!(h
        .query_by_label("Create animated sampler variant")
        .is_none());
    if cfg!(feature = "room-animated-models") {
        click(&mut h, "Cooked animated model");
        click(&mut h, "Create animated sampler variant");
        assert!(h.state().sampler.enabled);
        assert!(h.state().sampler_request().is_err());
        click(&mut h, "Create animated color variant");
        assert!(!h.state().sampler.enabled);
        click(&mut h, "Create animated sampler variant");
        click(&mut h, "Create animated PNG skin");
        assert!(!h.state().sampler.enabled);
        click(&mut h, "Create animated sampler variant");
        click(&mut h, "Create trimmed animation clip");
        assert!(!h.state().sampler.enabled);
        click(&mut h, "Create animated sampler variant");
        assert!(
            !h.state().trim.enabled
                && !h.state().color_variant_enabled
                && !h.state().texture_variant_enabled
        );
        click(&mut h, "Raw GLB");
        assert_eq!(h.state().sampler, Controls::default());
    } else {
        assert!(h
            .get_by_label("Cooked animated model")
            .accesskit_node()
            .is_disabled());
    }
}
#[cfg(feature = "room-animated-models")]
#[test]
fn missing_current_used_catalog_refuses_workers_and_busy_disables_typed_controls() {
    let mut p = Panel::default();
    p.source_format = SourceFormat::CookedAnimated;
    p.kind = ModelKind::Animated;
    p.source = "/source.animated.json".into();
    p.sampler.enabled = true;
    p.sampler.material = Some(0);
    p.inspected_hash = Some("a".repeat(64));
    assert!(p.sampler_request().is_err());
    p.start_import("/project".into(), 4, 7);
    assert!(!p.busy());
    let (sender, receiver) = mpsc::channel();
    p.pending = Some(receiver);
    let mut h = harness(p);
    for label in [
        "Create animated sampler variant",
        "Sampler filtering",
        "Sampler wrap S",
        "Sampler wrap T",
        "Sampler variant asset ID",
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
fn actual_inspection_used_material_typed_sampler_changes_pin_hash_and_generation() {
    use std::time::{Duration, Instant};
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("courier.animated.json");
    std::fs::write(
        &source,
        include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json"),
    )
    .unwrap();
    let original = std::fs::read(&source).unwrap();
    let mut p = Panel::default();
    p.source_format = SourceFormat::CookedAnimated;
    p.kind = ModelKind::Animated;
    p.source = source.to_string_lossy().into();
    let mut h = harness(p);
    click(&mut h, "Create animated sampler variant");
    click(&mut h, "Inspect animated source");
    let deadline = Instant::now() + Duration::from_secs(5);
    while h.state().busy() {
        let _ = h.state_mut().poll(Some(Path::new("/project")), 4, 7);
        h.run_steps(2);
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    click(&mut h, "Use material 0");
    let before = h.state().authoring_generation;
    let hash = h.state().inspected_hash.clone();
    assert!(h.state().sampler_request().unwrap().is_some());
    click(&mut h, "Sampler filtering");
    click(&mut h, "Nearest filtering");
    click(&mut h, "Sampler wrap S");
    click(&mut h, "Clamp S");
    click(&mut h, "Sampler wrap T");
    click(&mut h, "Mirror T");
    let request = h.state().sampler_request().unwrap().unwrap();
    assert!(!request.linear_filter);
    assert_eq!(request.wrap_s, Wrap::Clamp);
    assert_eq!(request.wrap_t, Wrap::Mirror);
    assert_eq!(h.state().inspected_hash, hash);
    assert!(h.state().authoring_generation > before);
    assert!(!h.state().license_acknowledged);
    assert_eq!(std::fs::read(source).unwrap(), original);
    assert!(h.state().destination_identity.is_none());
    h.state_mut().sampler.asset_id = "../bad.sampler.json".into();
    assert!(h.state().sampler_request().is_err());
    h.state_mut().sampler.material = Some(u32::MAX);
    assert!(h.state().sampler_request().is_err());
}
