use super::*;
use crate::model_package_import::source_inspection::{ClipEntry, MaterialEntry};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
fn panel() -> Panel {
    Panel {
        source: "/input.animated.json".into(),
        source_format: SourceFormat::CookedAnimated,
        kind: ModelKind::Animated,
        color_variant_enabled: true,
        ..Default::default()
    }
}
fn catalog() -> Catalog {
    Catalog {
        source_sha256: "a".repeat(64),
        materials: vec![
            MaterialEntry {
                index: 0,
                used_primitive_count: 2,
                base_color: [0.2, 0.3, 0.4, 0.5],
                image: 0,
                width: 4,
                height: 2,
                linear_filter: false,
                wrap_s: orr_model::Wrap::Clamp,
                wrap_t: orr_model::Wrap::Repeat,
            },
            MaterialEntry {
                index: 1,
                used_primitive_count: 0,
                base_color: [1.0; 4],
                image: 0,
                width: 4,
                height: 2,
                linear_filter: true,
                wrap_s: orr_model::Wrap::Mirror,
                wrap_t: orr_model::Wrap::Clamp,
            },
        ],
        clips: vec![ClipEntry {
            index: 0,
            name: "walk".into(),
            duration: 2.0,
        }],
    }
}
fn harness(p: Panel) -> Harness<'static, Panel> {
    let mut h = Harness::builder()
        .with_size([1600.0, 2000.0])
        .build_ui_state(
            |ui, p: &mut Panel| p.show(ui, true, Some(Path::new("/project")), 4, 7),
            p,
        );
    h.run_steps(3);
    h.get_by_label("Import a model package").click_accesskit();
    h.run_steps(3);
    h
}
#[test]
fn inspection_button_is_explicit_compiled_animated_only_and_does_not_grant_rights() {
    let mut h = harness(Panel::default());
    assert!(h.query_by_label("Inspect animated source").is_none());
    if cfg!(feature = "room-animated-models") {
        h.get_by_label("Cooked animated model").click_accesskit();
        h.run_steps(3);
        assert!(h
            .get_by_label("Inspect animated source")
            .accesskit_node()
            .is_disabled());
    }
    assert!(!h.state().license_acknowledged);
    assert!(!h.state().busy());
}
#[test]
fn invalid_inspection_context_never_starts_worker() {
    let mut p = Panel::default();
    p.start_source_inspection(Some(Path::new("/project")), 4, 7);
    assert!(!p.busy());
    let mut p = panel();
    p.start_source_inspection(None, 4, 7);
    assert!(!p.busy());
    #[cfg(not(feature = "room-animated-models"))]
    {
        p.start_source_inspection(Some(Path::new("/project")), 4, 7);
        assert!(!p.busy());
    }
}
#[cfg(feature = "room-animated-models")]
#[test]
fn used_catalog_row_selects_material_and_hash_only_after_explicit_action() {
    let mut p = panel();
    p.inspection.identity = p.inspection_identity(Some(Path::new("/project")), 4, 7);
    p.inspection.catalog = Some(catalog());
    let mut h = harness(p);
    assert!(h.state().inspected_hash.is_none());
    assert!(h
        .get_by_label("Use material 1")
        .accesskit_node()
        .is_disabled());
    assert!(h.query_by_label("Clip 0: walk (2.000s)").is_some());
    h.get_by_label("Use material 0").click_accesskit();
    h.run_steps(3);
    assert_eq!(h.state().color_variant.material, 0);
    assert_eq!(h.state().inspected_hash, Some("a".repeat(64)));
    assert!(!h.state().license_acknowledged);
    assert!(!h.state().busy());
    assert!(h.state().destination_identity.is_none());
    h.get_by_label("Raw GLB").click_accesskit();
    h.run_steps(3);
    assert!(h.state().inspection.catalog.is_none());
    assert!(h.state().inspected_hash.is_none());
}
#[cfg(feature = "room-animated-models")]
#[test]
fn inspection_busy_and_stale_project_source_generation_results_cannot_be_selected() {
    for changed in ["project", "source", "generation", "error"] {
        let mut p = panel();
        let identity = p
            .inspection_identity(Some(Path::new("/project")), 4, 7)
            .unwrap();
        p.inspection.identity = Some(identity.clone());
        let (sender, receiver) = mpsc::channel();
        p.inspection.pending = Some(receiver);
        let mut h = harness(p);
        assert!(h.state().busy());
        assert!(h
            .get_by_label("Inspect animated source")
            .accesskit_node()
            .is_disabled());
        if changed == "source" {
            h.state_mut().source = "/other.animated.json".into();
        }
        sender
            .send((
                identity,
                if changed == "error" {
                    Err("invalid animated data".into())
                } else {
                    Ok(catalog())
                },
            ))
            .unwrap();
        let root = if changed == "project" {
            "/other-project"
        } else {
            "/project"
        };
        let generation = if changed == "generation" { 8 } else { 7 };
        // poll must service read-only completion even though no import is pending.
        assert!(h
            .state_mut()
            .poll(Some(Path::new(root)), 4, generation)
            .is_none());
        assert!(!h.state().busy());
        assert!(h.state().inspection.catalog.is_none());
        assert!(h.state().inspected_hash.is_none());
        if changed == "error" {
            assert!(h
                .state()
                .inspection
                .status
                .contains("invalid animated data"));
        }
    }
}
#[cfg(all(feature = "room-animated-models", target_os = "linux"))]
#[test]
fn real_readonly_worker_error_does_not_publish_or_grant_license() {
    use std::time::{Duration, Instant};
    let temp = tempfile::tempdir().unwrap();
    let mut p = panel();
    p.source = temp
        .path()
        .join("missing.animated.json")
        .display()
        .to_string();
    p.start_source_inspection(Some(temp.path()), 4, 7);
    let deadline = Instant::now() + Duration::from_secs(10);
    while p.busy() {
        assert!(p.poll(Some(temp.path()), 4, 7).is_none());
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(p.inspection.status.contains("inspection failed"));
    assert!(p.inspection.catalog.is_none());
    assert!(!p.license_acknowledged);
    assert!(p.destination_identity.is_none());
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
}
