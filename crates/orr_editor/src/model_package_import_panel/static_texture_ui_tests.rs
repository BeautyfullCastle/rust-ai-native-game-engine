//! Static PNG CPU widgets and immutable request identity; no GPU/window.
use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};

fn harness(p: Panel) -> Harness<'static, Panel> {
    let mut h = Harness::builder()
        .with_size([1600.0, 3200.0])
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
fn replace(h: &mut Harness<'_, Panel>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    h.run_steps(2);
}

#[test]
fn static_texture_widgets_default_off_exclusive_reset_and_busy() {
    let mut h = harness(Panel::default());
    assert!(!h.state().static_texture.enabled);
    assert!(h.query_by_label("Create static PNG skin").is_none());
    click(&mut h, "Cooked static model");
    assert!(!h.state().static_texture.enabled);
    assert_eq!(h.state().kind, ModelKind::Static);
    assert!(h.query_by_label("Preview animated variant").is_none());
    click(&mut h, "Create static UV variant");
    click(&mut h, "Create static PNG skin");
    assert!(!h.state().static_uv.enabled);
    assert!(h.state().static_texture.enabled);
    click(&mut h, "Create static UV variant");
    assert!(h.state().static_uv.enabled);
    assert!(!h.state().static_texture.enabled);
    h.state_mut().color_variant_enabled = true;
    h.state_mut().texture_variant_enabled = true;
    h.state_mut().trim.enabled = true;
    h.state_mut().sampler.enabled = true;
    h.state_mut().sequence.enabled = true;
    h.state_mut().uv.enabled = true;
    click(&mut h, "Create static PNG skin");
    let p = h.state();
    assert!(
        !p.static_uv.enabled
            && !p.color_variant_enabled
            && !p.texture_variant_enabled
            && !p.trim.enabled
            && !p.sampler.enabled
            && !p.sequence.enabled
            && !p.uv.enabled
    );
    assert!(p.static_texture_request().is_err());
    assert!(h
        .get_by_label("Import cooked static package")
        .accesskit_node()
        .is_disabled());
    h.state_mut().start_import("/project".into(), 4, 7);
    assert!(!h.state().busy());
    let (sender, receiver) = mpsc::channel();
    h.state_mut().pending = Some(receiver);
    h.run_steps(2);
    for label in [
        "Create static PNG skin",
        "Static variant PNG file",
        "Static texture variant asset ID",
        "Create static UV variant",
        "Inspect static source",
        "Import cooked static package",
    ] {
        assert!(
            h.get_by_label(label).accesskit_node().is_disabled(),
            "{label}"
        );
    }
    drop(sender);
    assert!(h
        .state_mut()
        .poll(Some(Path::new("/project")), 4, 7)
        .is_none());
    h.run_steps(2);
    click(&mut h, "Raw GLB");
    assert_eq!(h.state().static_texture, Controls::default());
    assert!(!h.state().license_acknowledged);
    assert!(h.query_by_label("Create static PNG skin").is_none());
    if cfg!(feature = "room-animated-models") {
        click(&mut h, "Cooked animated model");
        assert!(h.query_by_label("Create static PNG skin").is_none());
    }
}

#[test]
fn static_texture_actual_png_id_and_package_edits_revoke_consent() {
    let mut p = Panel::default();
    p.set_source_format(SourceFormat::CookedStatic);
    p.static_texture.enabled = true;
    let mut h = harness(p);
    let png = std::env::temp_dir().join("skin.png").display().to_string();
    for (label, value) in [
        ("Static variant PNG file", png.as_str()),
        (
            "Static texture variant asset ID",
            "variants/new.retexture.json",
        ),
        ("Package destination directory", "/new-package"),
        ("Package name", "static-skin"),
        ("Package version", "1.0.0"),
        ("License file", "/license.txt"),
    ] {
        h.state_mut().license_acknowledged = true;
        h.state_mut().destination_identity = Some("/old-package".into());
        let generation = h.state().authoring_generation;
        replace(&mut h, label, value);
        assert!(!h.state().license_acknowledged, "{label}");
        assert!(h.state().destination_identity.is_none(), "{label}");
        assert!(h.state().authoring_generation > generation, "{label}");
    }
    assert_eq!(h.state().static_texture.texture, PathBuf::from(png));
    h.state_mut().license_acknowledged = true;
    click(&mut h, "Create static PNG skin");
    assert!(!h.state().license_acknowledged);
    click(&mut h, "Create static PNG skin");
    h.state_mut().license_acknowledged = true;
    replace(
        &mut h,
        "Cooked static model file (.model.json)",
        "/other.model.json",
    );
    assert_eq!(h.state().static_texture, Controls::default());
    assert!(!h.state().license_acknowledged);
}

#[cfg(target_os = "linux")]
fn fixture(path: &Path) {
    let model = orr_model::import::import_with_resolver(
        "props/tile.glb",
        include_bytes!("../../../orr_model/tests/fixtures/model.glb"),
        |_| panic!("embedded fixture"),
    )
    .unwrap();
    let mut source = model.source().clone();
    source.materials.push(source.materials[0].clone());
    std::fs::write(
        path,
        orr_model::StaticModel::new(source)
            .unwrap()
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
}

#[cfg(target_os = "linux")]
fn inspected(source: &Path) -> Harness<'static, Panel> {
    use std::time::{Duration, Instant};
    let mut p = Panel::default();
    p.set_source_format(SourceFormat::CookedStatic);
    p.source = source.display().to_string();
    p.license = "/license.txt".into();
    p.destination = "/package".into();
    p.name = "static-skin".into();
    p.version = "1.0.0".into();
    p.static_texture.texture = source.with_file_name("skin.png");
    let mut h = harness(p);
    click(&mut h, "Create static PNG skin");
    click(&mut h, "Inspect static source");
    let deadline = Instant::now() + Duration::from_secs(5);
    while h.state().busy() {
        h.state_mut().poll(Some(Path::new("/project")), 4, 7);
        h.run_steps(2);
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    click(&mut h, "Use material 0");
    assert!(h.state().static_texture_request().unwrap().is_some());
    h
}

#[cfg(target_os = "linux")]
#[test]
fn static_texture_inspected_used_material_and_shared_path_id_admission() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("tile.model.json");
    fixture(&source);
    let original = std::fs::read(&source).unwrap();
    let mut h = inspected(&source);
    assert!(h
        .get_by_label("Use material 2")
        .accesskit_node()
        .is_disabled());
    assert!(!h.state().license_acknowledged);
    assert_eq!(std::fs::read(&source).unwrap(), original);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    let p = h.state_mut();
    let valid_png = p.static_texture.texture.clone();
    for path in [
        "",
        "relative.png",
        "/tmp/../skin.png",
        "/tmp/./skin.png",
        "/tmp/skin.jpg",
        "/tmp/CON.png",
    ] {
        p.static_texture.texture = path.into();
        assert!(p.static_texture_request().is_err(), "{path}");
    }
    p.static_texture.texture = valid_png;
    for id in [
        "",
        "../bad.retexture.json",
        "/bad.retexture.json",
        "bad.uv.json",
        "CON.retexture.json",
    ] {
        p.static_texture.asset_id = id.into();
        assert!(p.static_texture_request().is_err(), "{id}");
    }
    p.static_texture.asset_id = "variants/new.retexture.json".into();
    assert!(p.static_texture_request().unwrap().is_some());
    for material in [2, u32::MAX] {
        p.static_texture.material = Some(material);
        assert!(p.static_texture_request().is_err());
    }
    p.static_texture.material = Some(0);
    for mode in [
        "color",
        "texture",
        "trim",
        "sampler",
        "sequence",
        "uv",
        "static-uv",
    ] {
        match mode {
            "color" => p.color_variant_enabled = true,
            "texture" => p.texture_variant_enabled = true,
            "trim" => p.trim.enabled = true,
            "sampler" => p.sampler.enabled = true,
            "sequence" => p.sequence.enabled = true,
            "uv" => p.uv.enabled = true,
            _ => p.static_uv.enabled = true,
        }
        assert!(p.static_texture_request().is_err(), "{mode}");
        p.color_variant_enabled = false;
        p.texture_variant_enabled = false;
        p.trim.enabled = false;
        p.sampler.enabled = false;
        p.sequence.enabled = false;
        p.uv.enabled = false;
        p.static_uv.enabled = false;
    }
    p.source = "/other.model.json".into();
    assert!(p.static_texture_request().is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn static_texture_snapshot_rejects_stale_inputs_and_accepts_current_once() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("tile.model.json");
    fixture(&source);
    for changed in [
        "current",
        "project",
        "project-generation",
        "source-generation",
        "authoring-generation",
        "source",
        "license",
        "destination",
        "name",
        "version",
        "kind",
        "format",
        "hash",
        "material",
        "png-path",
        "id",
        "disabled",
        "static-uv",
    ] {
        let mut h = inspected(&source);
        let p = h.state_mut();
        p.submitted_static_texture = Some(
            p.static_texture_snapshot(Path::new("/project"), 4, 7)
                .unwrap(),
        );
        p.submitted_inspected_hash = Some(p.inspected_hash.clone());
        p.submitted_authoring_generation = Some(p.authoring_generation);
        p.license_acknowledged = true;
        match changed {
            "source" => p.source = "/other.model.json".into(),
            "license" => p.license.push('2'),
            "destination" => p.destination.push('2'),
            "name" => p.name.push('2'),
            "version" => p.version.push('2'),
            "kind" => p.kind = ModelKind::Animated,
            "format" => p.source_format = SourceFormat::RawGlb,
            "hash" => p.inspected_hash = Some("b".repeat(64)),
            "material" => p.static_texture.material = Some(1),
            "png-path" => p.static_texture.texture = source.with_file_name("other.png"),
            "id" => p.static_texture.asset_id = "variants/other.retexture.json".into(),
            "disabled" => p.static_texture.enabled = false,
            "static-uv" => p.static_uv.enabled = true,
            "authoring-generation" => p.authoring_generation += 1,
            _ => {}
        }
        let (sender, receiver) = mpsc::channel();
        p.pending = Some(receiver);
        sender
            .send(Completion {
                source_format: SourceFormat::CookedStatic,
                project_root: "/project".into(),
                expected_destination: Ok("/package".into()),
                project_generation: 4,
                source_generation: 7,
                result: Ok(Report {
                    root: "/package".into(),
                    package: "static-skin".into(),
                    asset: "model.orrmodel".into(),
                    kind: ModelKind::Static,
                    source_sha256: "a".repeat(64),
                    license_sha256: "c".repeat(64),
                }),
            })
            .unwrap();
        let root = if changed == "project" {
            "/other-project"
        } else {
            "/project"
        };
        let result = p.poll(
            Some(Path::new(root)),
            if changed == "project-generation" {
                5
            } else {
                4
            },
            if changed == "source-generation" { 8 } else { 7 },
        );
        assert_eq!(result.is_some(), changed == "current", "{changed}");
        assert_eq!(
            p.destination_identity.is_some(),
            changed == "current",
            "{changed}"
        );
        assert!(!p.license_acknowledged);
        assert!(!p.busy());
        assert!(p.poll(Some(Path::new("/project")), 4, 7).is_none());
    }
}
