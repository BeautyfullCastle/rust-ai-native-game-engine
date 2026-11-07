use super::*;
#[cfg(feature = "room-animated-models")]
use egui_kittest::kittest::NodeT;
use egui_kittest::{kittest::Queryable, Harness};
#[cfg(feature = "room-animated-models")]
fn panel() -> Panel {
    Panel {
        source: "/source.animated.json".into(),
        source_format: SourceFormat::CookedAnimated,
        kind: ModelKind::Animated,
        color_variant_enabled: true,
        ..Default::default()
    }
}
#[cfg(feature = "room-animated-models")]
fn prepared() -> Prepared {
    Prepared {
        model: Arc::new(
            orr_model::animation::AnimatedModel::from_bytes(include_bytes!(
                "../../../../assets/room_character_demo/courier.orrmodel.json"
            ))
            .unwrap(),
        ),
        asset_sha256: "a".repeat(64),
        source_sha256: "b".repeat(64),
        png_sha256: None,
    }
}
#[cfg(feature = "room-animated-models")]
fn harness(panel: Panel) -> Harness<'static, Panel> {
    harness_enabled(panel, true)
}
#[cfg(feature = "room-animated-models")]
fn harness_enabled(panel: Panel, enabled: bool) -> Harness<'static, Panel> {
    let mut h = Harness::builder()
        .with_size([1500.0, 2000.0])
        .build_ui_state(
            move |ui, p: &mut Panel| p.show(ui, enabled, Some(Path::new("/project")), 4, 7),
            panel,
        );
    h.run_steps(3);
    h.get_by_label("Import a model package").click_accesskit();
    h.run_steps(3);
    h
}
#[cfg(feature = "room-animated-models")]
#[test]
fn preview_is_enabled_without_license_or_destination_but_requires_explicit_valid_variant() {
    let mut h = harness(panel());
    assert!(!h.state().license_acknowledged);
    assert!(h.state().destination.is_empty());
    assert!(!h
        .get_by_label("Preview animated variant")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("Create animated color variant")
        .click_accesskit();
    h.run_steps(3);
    assert!(h
        .get_by_label("Preview animated variant")
        .accesskit_node()
        .is_disabled());
    assert!(!h.state().busy());
}
#[cfg(feature = "room-animated-models")]
#[test]
fn exact_operation_source_hash_and_project_generation_changes_retire_ready_and_pending() {
    for change in [
        "source",
        "rgb",
        "id",
        "hash",
        "operation",
        "project",
        "generation",
    ] {
        let mut p = panel();
        let identity = p
            .preview_identity(Some(Path::new("/project")), 4, 7)
            .unwrap();
        p.variant_preview.identity = Some(identity.clone());
        p.variant_preview.ready = Some(PreviewFrame {
            ticket: 1,
            prepared: Arc::new(prepared()),
        });
        let (sender, receiver) = mpsc::channel();
        p.variant_preview.pending = Some(receiver);
        match change {
            "source" => p.source = "/other.animated.json".into(),
            "rgb" => p.color_variant.linear_rgb[0] = 0.2,
            "id" => p.color_variant.asset_id = "other.recolor.json".into(),
            "hash" => p.inspected_hash = Some("c".repeat(64)),
            "operation" => p.color_variant_enabled = false,
            _ => {}
        }
        if change != "project" && change != "generation" {
            assert!(p.preview_frame().is_none());
        }
        sender
            .send(Completion {
                identity,
                ticket: 1,
                result: Ok(prepared()),
            })
            .unwrap();
        let root = if change == "project" {
            "/other"
        } else {
            "/project"
        };
        let generation = if change == "generation" { 8 } else { 7 };
        assert!(p.poll(Some(Path::new(root)), 4, generation).is_none());
        assert!(p.preview_frame().is_none(), "{change}");
        assert!(!p.busy());
        assert!(!p.license_acknowledged);
    }
}
#[cfg(feature = "room-animated-models")]
#[test]
fn close_drops_ready_and_rejects_running_completion_while_retaining_single_worker_fence() {
    let mut p = panel();
    let identity = p
        .preview_identity(Some(Path::new("/project")), 4, 7)
        .unwrap();
    let retained = Arc::new(prepared());
    let weak = Arc::downgrade(&retained);
    p.variant_preview.identity = Some(identity.clone());
    p.variant_preview.ready = Some(PreviewFrame {
        ticket: 1,
        prepared: retained,
    });
    let (sender, receiver) = mpsc::channel();
    p.variant_preview.pending = Some(receiver);
    p.close_variant_preview();
    assert!(weak.upgrade().is_none());
    assert!(p.busy());
    assert!(p.preview_frame().is_none());
    p.start_variant_preview(Some(Path::new("/project")), 4, 7);
    assert_eq!(p.variant_preview.ticket, 0);
    sender
        .send(Completion {
            identity,
            ticket: 1,
            result: Ok(prepared()),
        })
        .unwrap();
    p.poll(Some(Path::new("/project")), 4, 7);
    assert!(!p.busy());
    assert!(p.preview_frame().is_none());
}
#[cfg(feature = "room-animated-models")]
#[test]
fn busy_preview_disables_import_inspection_and_variant_controls_and_reports_failure() {
    let mut p = panel();
    let identity = p
        .preview_identity(Some(Path::new("/project")), 4, 7)
        .unwrap();
    p.variant_preview.identity = Some(identity.clone());
    let (sender, receiver) = mpsc::channel();
    p.variant_preview.pending = Some(receiver);
    let mut h = harness(p);
    for label in [
        "Preview animated variant",
        "Inspect animated source",
        "Create animated color variant",
        "Import cooked animated package",
    ] {
        assert!(h.get_by_label(label).accesskit_node().is_disabled());
    }
    sender
        .send(Completion {
            identity,
            ticket: 1,
            result: Err("invalid source snapshot".into()),
        })
        .unwrap();
    assert!(h
        .state_mut()
        .poll(Some(Path::new("/project")), 4, 7)
        .is_none());
    assert!(!h.state().busy());
    assert!(h
        .state()
        .variant_preview
        .status
        .contains("invalid source snapshot"));
    assert!(!h.state().license_acknowledged);
}
#[cfg(all(feature = "room-animated-models", target_os = "linux"))]
#[test]
fn actual_preview_button_worker_is_readonly_and_publishes_no_files_or_license() {
    use std::time::{Duration, Instant};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.animated.json");
    std::fs::write(
        &source,
        include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json"),
    )
    .unwrap();
    let mut p = panel();
    p.source = source.display().to_string();
    p.color_variant.linear_rgb = [0.2, 0.7, 0.4];
    let mut h = harness(p);
    h.get_by_label("Preview animated variant").click_accesskit();
    h.run_steps(1);
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().busy() {
        h.state_mut().poll(Some(Path::new("/project")), 4, 7);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let frame = h.state_mut().preview_frame().unwrap();
    assert_eq!(
        frame.prepared.model.source().materials[0].base_color,
        [0.2, 0.7, 0.4, 1.0]
    );
    assert_eq!(frame.ticket, 1);
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    assert!(!h.state().license_acknowledged);
    assert!(h.state().destination_identity.is_none());
}
#[cfg(not(feature = "room-animated-models"))]
#[test]
fn feature_off_exposes_no_variant_preview_button_or_worker() {
    let mut h = Harness::builder()
        .with_size([1400.0, 1600.0])
        .build_ui_state(
            |ui, p: &mut Panel| p.show(ui, true, Some(Path::new("/project")), 4, 7),
            Panel::default(),
        );
    h.run_steps(3);
    h.get_by_label("Import a model package").click_accesskit();
    h.run_steps(3);
    assert!(h.query_by_label("Preview animated variant").is_none());
    assert!(!h.state().busy());
}
#[cfg(feature = "room-animated-models")]
#[test]
fn actual_cancel_button_retires_pending_preview_without_spawning_or_accepting_completion() {
    let mut p = panel();
    let identity = p
        .preview_identity(Some(Path::new("/project")), 4, 7)
        .unwrap();
    p.variant_preview.identity = Some(identity.clone());
    let (sender, receiver) = mpsc::channel();
    p.variant_preview.pending = Some(receiver);
    let mut h = harness_enabled(p, false);
    assert!(!h
        .get_by_label("Cancel variant preview")
        .accesskit_node()
        .is_disabled());
    assert!(h
        .get_by_label("Preview animated variant")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("Cancel variant preview").click_accesskit();
    h.run_steps(3);
    assert!(h.state().variant_preview.retired);
    assert!(h.state().busy());
    assert!(h.state_mut().preview_frame().is_none());
    h.state_mut()
        .start_variant_preview(Some(Path::new("/project")), 4, 7);
    assert_eq!(h.state().variant_preview.ticket, 0);
    sender
        .send(Completion {
            identity,
            ticket: 1,
            result: Ok(prepared()),
        })
        .unwrap();
    h.state_mut().poll(Some(Path::new("/project")), 4, 7);
    assert!(!h.state().busy());
    assert!(h.state_mut().preview_frame().is_none());
    assert!(!h.state().license_acknowledged);
}
