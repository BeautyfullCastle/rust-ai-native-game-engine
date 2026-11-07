use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
fn harness(panel: Panel) -> Harness<'static, Panel> {
    let mut h = Harness::builder()
        .with_size([1500.0, 2600.0])
        .build_ui_state(
            |ui, p: &mut Panel| p.show(ui, true, Some(Path::new("/project")), 4, 7),
            panel,
        );
    h.run_steps(2);
    h.get_by_label("Import a model package").click_accesskit();
    h.run_steps(2);
    h
}
fn click(h: &mut Harness<'_, Panel>, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(2);
}
#[test]
fn trim_is_explicit_default_off_mutually_exclusive_and_reset_with_source_format() {
    let mut h = harness(Panel::default());
    assert!(!h.state().trim.enabled);
    assert!(h.query_by_label("Create trimmed animation clip").is_none());
    if cfg!(feature = "room-animated-models") {
        click(&mut h, "Cooked animated model");
        click(&mut h, "Create trimmed animation clip");
        assert!(h.state().trim.enabled);
        assert!(!h.state().color_variant_enabled && !h.state().texture_variant_enabled);
        assert!(h.query_by_label("Trim source clip").is_some());
        click(&mut h, "Create animated color variant");
        assert!(!h.state().trim.enabled);
        assert!(h.state().color_variant_enabled);
        click(&mut h, "Create trimmed animation clip");
        click(&mut h, "Create animated PNG skin");
        assert!(!h.state().trim.enabled);
        assert!(h.state().texture_variant_enabled);
        click(&mut h, "Create trimmed animation clip");
        click(&mut h, "Raw GLB");
        assert_eq!(h.state().trim, Controls::default());
    } else {
        assert!(
            h.get_by_label("Cooked animated model")
                .accesskit_node()
                .is_disabled()
        );
    }
}
#[cfg(feature = "room-animated-models")]
fn valid() -> Panel {
    let mut p = Panel::default();
    p.source_format = SourceFormat::CookedAnimated;
    p.kind = ModelKind::Animated;
    p.source = "/input.animated.json".into();
    p.trim = Controls {
        enabled: true,
        clip: Some(0),
        duration: Some(1.0),
        source_hash: Some("a".repeat(64)),
        start: "0.25".into(),
        end: "0.75".into(),
        ..Default::default()
    };
    p.inspected_hash = p.trim.source_hash.clone();
    p
}
#[cfg(feature = "room-animated-models")]
#[test]
fn raw_invalid_trim_values_refuse_import_before_any_worker_or_io() {
    for case in 0..13 {
        let mut p = valid();
        match case {
            0 => p.trim.start.clear(),
            1 => p.trim.start = "NaN".into(),
            2 => p.trim.start = "-1e-99".into(),
            3 => p.trim.end = "1.0000000000000002".into(),
            4 => p.trim.end = p.trim.start.clone(),
            5 => p.trim.name = "\n".into(),
            6 => p.trim.name = "x".repeat(129),
            7 => p.trim.asset_id = "../x.trim.json".into(),
            8 => p.trim.clip = None,
            9 => p.inspected_hash = None,
            10 => p.color_variant_enabled = true,
            11 => p.source_format = SourceFormat::RawGlb,
            _ => p.trim.end = "inf".into(),
        };
        assert!(p.trim_request().is_err(), "case {case}");
        p.start_import(PathBuf::from("/project"), 4, 7);
        assert!(!p.busy(), "case {case}");
    }
}
#[cfg(feature = "room-animated-models")]
#[test]
fn held_worker_disables_trim_fields_and_out_and_back_generation_rejects_completion() {
    let mut p = valid();
    let original_controls = p.trim.clone();
    let original_generation = p.authoring_generation;
    let submitted = p.trim_request().unwrap();
    p.submitted_trim = Some(submitted);
    p.submitted_color = Some(None);
    p.submitted_texture = Some(None);
    p.submitted_inspected_hash = Some(p.inspected_hash.clone());
    p.submitted_authoring_generation = Some(p.authoring_generation);
    let (sender, receiver) = mpsc::channel();
    p.pending = Some(receiver);
    let mut h = harness(p);
    assert!(
        h.get_by_label("Trim start seconds")
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        h.get_by_label("Create trimmed animation clip")
            .accesskit_node()
            .is_disabled()
    );
    let original = h.state().trim.end.clone();
    h.state_mut().trim.end = "0.8".into();
    h.state_mut().source_inputs_changed();
    h.state_mut().trim.end = original;
    h.state_mut().source_inputs_changed();
    assert_eq!(
        h.state().trim,
        original_controls,
        "authoring values returned exactly to their submitted form"
    );
    assert!(h.state().authoring_generation > original_generation);
    sender
        .send(Completion {
            source_format: SourceFormat::CookedAnimated,
            project_root: "/project".into(),
            expected_destination: Ok("/new".into()),
            project_generation: 4,
            source_generation: 7,
            result: Ok(Report {
                root: "/new".into(),
                package: "trim".into(),
                asset: "model.animated.json".into(),
                kind: ModelKind::Animated,
                source_sha256: "asset".into(),
                license_sha256: "license".into(),
            }),
        })
        .unwrap();
    assert!(
        h.state_mut()
            .poll(Some(Path::new("/project")), 4, 7)
            .is_none()
    );
    assert!(!h.state().busy());
    assert!(h.state().destination_identity.is_none());
}
#[cfg(all(feature = "room-animated-models", target_os = "linux"))]
mod actual {
    use super::*;
    use egui::accesskit::Role;
    use std::time::{Duration, Instant};
    fn text(h: &mut Harness<'_, Panel>, label: &str, value: &str) {
        h.get_by_role_and_label(Role::TextInput, label).focus();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.run_steps(2);
        h.get_by_role_and_label(Role::TextInput, label)
            .type_text(value);
        h.run_steps(2);
        assert_eq!(
            h.get_by_role_and_label(Role::TextInput, label)
                .accesskit_node()
                .value()
                .as_deref(),
            Some(value)
        );
    }
    fn wait(h: &mut Harness<'_, Panel>) -> Option<Report> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut report = None;
        while h.state().busy() {
            assert!(Instant::now() < deadline);
            report = h
                .state_mut()
                .poll(Some(Path::new("/project")), 4, 7)
                .or(report);
            h.run_steps(1);
            std::thread::sleep(Duration::from_millis(2));
        }
        report
    }
    fn fixture() -> (tempfile::TempDir, Harness<'static, Panel>) {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.animated.json");
        std::fs::write(
            &source,
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/room_character_demo/courier.orrmodel.json"
            )),
        )
        .unwrap();
        let license = dir.path().join("LICENSE.txt");
        std::fs::write(&license, b"explicit fixture license").unwrap();
        let p = Panel {
            source: source.display().to_string(),
            license: license.display().to_string(),
            destination: dir.path().join("new-package").display().to_string(),
            name: "trim-test".into(),
            version: "1.0.0".into(),
            source_format: SourceFormat::CookedAnimated,
            kind: ModelKind::Animated,
            ..Default::default()
        };
        let mut h = harness(p);
        click(&mut h, "Inspect animated source");
        wait(&mut h);
        click(&mut h, "Create trimmed animation clip");
        click(&mut h, "Trim source clip");
        click(&mut h, "Trim source clip 0: searching (1.000s)");
        text(&mut h, "Trim start seconds", "0.25");
        text(&mut h, "Trim end seconds", "0.75");
        (dir, h)
    }
    #[test]
    fn actual_catalog_trim_preview_and_import_append_the_same_clip_without_source_writes() {
        use sha2::{Digest, Sha256};
        let (_dir, mut h) = fixture();
        let source = std::fs::read(&h.state().source).unwrap();
        assert!(!h.state().license_acknowledged);
        text(&mut h, "New animation clip name", "searching");
        assert!(
            h.get_by_label("Preview animated variant")
                .accesskit_node()
                .is_disabled(),
            "duplicate source clip names must be rejected before a worker"
        );
        text(&mut h, "New animation clip name", "trimmed");
        click(&mut h, "Preview animated variant");
        wait(&mut h);
        let preview = h.state_mut().preview_frame().expect("actual preview ready");
        assert_eq!(preview.prepared.model.source().clips.len(), 4);
        assert_eq!(preview.prepared.model.source().clips[3].name, "trimmed");
        assert_eq!(preview.prepared.model.source().clips[3].duration(), 0.5);
        text(&mut h, "Trim end seconds", "0.8");
        text(&mut h, "Trim end seconds", "0.75");
        assert!(
            h.state_mut().preview_frame().is_none(),
            "out-and-back edits must retire the old preview"
        );
        click(&mut h, "Preview animated variant");
        wait(&mut h);
        let fresh = h
            .state_mut()
            .preview_frame()
            .expect("fresh explicit preview");
        assert!(fresh.ticket > preview.ticket);
        assert_eq!(fresh.prepared.asset_sha256, preview.prepared.asset_sha256);
        assert!(
            h.get_by_label("Import cooked animated package")
                .accesskit_node()
                .is_disabled()
        );
        click(&mut h, "I have reviewed rights to distribute this content");
        click(&mut h, "Import cooked animated package");
        let report = wait(&mut h).expect("actual import report");
        let published = std::fs::read(report.root.join(&report.asset)).unwrap();
        let hash = Sha256::digest(&published)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        assert_eq!(report.source_sha256, hash);
        assert_eq!(preview.prepared.asset_sha256, hash);
        assert_eq!(std::fs::read(&h.state().source).unwrap(), source);
        assert!(!h.state().license_acknowledged);
    }
    #[test]
    fn changed_inspected_source_is_rejected_by_preview_and_import_without_publication() {
        let (_dir, mut h) = fixture();
        let mut changed = std::fs::read(&h.state().source).unwrap();
        changed.push(b'\n');
        std::fs::write(&h.state().source, &changed).unwrap();
        click(&mut h, "Preview animated variant");
        wait(&mut h);
        assert!(h.state_mut().preview_frame().is_none());
        click(&mut h, "I have reviewed rights to distribute this content");
        click(&mut h, "Import cooked animated package");
        assert!(wait(&mut h).is_none());
        assert!(!Path::new(&h.state().destination).exists());
        assert_eq!(std::fs::read(&h.state().source).unwrap(), changed);
        assert!(h.state().status.contains("failed"));
    }
}
