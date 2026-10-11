//! Staged widget contracts only: uncompiled, no runtime/GPU claim.
#![cfg(feature = "sprite-clip-editor")]
use egui::accesskit::Role;
use egui_kittest::{Harness, kittest::{NodeT, Queryable}};
use orr_package::{LockedPackage, Manifest};
use orr_sprite::PlaybackMode;
use sha2::{Digest, Sha256};
use orr_editor::sprite_clip_panel::{ClipPanel, SaveOutcome};
use std::collections::{BTreeMap, BTreeSet};

fn source() -> Vec<u8> {
    br#"{"format":"orr_sprite","version":1,"atlas":{"image":"atlas.png","width":2,"height":1},"regions":[{"id":10,"x":0,"y":0,"width":1,"height":1},{"id":20,"x":1,"y":0,"width":1,"height":1}],"clips":[{"id":"walk","mode":"loop","frames":[{"region":10,"duration_ms":100},{"region":20,"duration_ms":200}]},{"id":"idle","mode":"once","frames":[{"region":10,"duration_ms":500}]}]}"#.to_vec()
}
fn admitted() -> (LockedPackage, BTreeMap<String, Vec<u8>>) {
    // This pure-core fixture is not a claim that these placeholder PNG bytes decode.
    // Actual package admission/decode and GPU integration remain mandatory later.
    let files: BTreeMap<String, Vec<u8>> = [
        ("sprite.json".into(), source()), ("atlas.png".into(), vec![1, 2, 3]),
        ("LICENSE.txt".into(), b"fixture license retained exactly\n".to_vec()),
    ].into();
    let manifest = Manifest { schema: 1, name: "fixture.sprite".into(), version: "1.0.0".into(),
        engine: "*".into(), capabilities: BTreeSet::new(), dependencies: BTreeMap::new(), files: files.keys().cloned().collect() };
    let hashes = files.iter().map(|(p, b)| (p.clone(), Sha256::digest(b).iter().map(|byte| format!("{byte:02x}")).collect::<String>())).collect();
    (LockedPackage { manifest, digest: "admitted-fixture-provenance".into(), files: hashes }, files)
}

fn harness() -> Harness<'static, ClipPanel> {
    let (package, files) = admitted();
    let panel = ClipPanel::from_admitted_snapshot(package, files, "sprite.json".into(), "walk", "1.0.1".into()).unwrap();
    let mut h = Harness::builder().with_size([1000.0, 900.0]).with_max_steps(120)
        .build_ui_state(|ui, panel: &mut ClipPanel| { egui::ScrollArea::vertical().show(ui, |ui| panel.show(ui)); }, panel);
    settle(&mut h); h
}
fn settle(h: &mut Harness<'_, ClipPanel>) { h.try_run().expect("bounded widget layout convergence"); }
fn click(h: &mut Harness<'_, ClipPanel>, label: &str) {
    h.get_by_label(label).scroll_to_me(); settle(h);
    h.get_by_label(label).click(); settle(h);
}
fn text(h: &mut Harness<'_, ClipPanel>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label).scroll_to_me(); settle(h);
    h.get_by_role_and_label(Role::TextInput, label).focus(); settle(h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A); settle(h);
    h.get_by_role_and_label(Role::TextInput, label).type_text(value); settle(h);
}
fn choose(h: &mut Harness<'_, ClipPanel>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::ComboBox, label).scroll_to_me(); settle(h);
    h.get_by_role_and_label(Role::ComboBox, label).click(); settle(h);
    click(h, value);
    assert_eq!(h.get_by_role_and_label(Role::ComboBox, label).accesskit_node().value().as_deref(), Some(value));
}
#[test]
fn real_widgets_edit_order_duration_mode_and_undo_redo() {
    let mut h = harness(); let original = h.state().source().clone();
    text(&mut h, "Duration ms", "125");
    choose(&mut h, "Playback", "Once");
    assert!(h.state().has_pending_draft());
    assert!(h.get_by_label("Save As new package").accesskit_node().is_disabled());
    assert_eq!(h.state().source(), &original);
    click(&mut h, "Apply frame draft");
    assert_eq!(h.state().source().clips[0].frames[0].duration_ms, 125);
    assert_eq!(h.state().source().clips[0].mode, PlaybackMode::Once);
    click(&mut h, "Move frame later");
    assert_eq!(h.state().source().clips[0].frames.iter().map(|f| f.region).collect::<Vec<_>>(), vec![20, 10]);
    click(&mut h, "Undo clip edit"); click(&mut h, "Undo clip edit");
    assert_eq!(h.state().source(), &original);
    click(&mut h, "Redo clip edit"); click(&mut h, "Redo clip edit");
    assert_eq!(h.state().source().clips[0].frames[1].duration_ms, 125);
    assert_eq!(h.state().source().clips[1], original.clips[1]);
    assert_eq!(h.state().source().atlas, original.atlas);
    assert_eq!(h.state().source().regions, original.regions);
}
#[test]
fn invalid_widget_drafts_do_not_commit_or_destroy_redo() {
    let mut h = harness(); let original = h.state().source().clone();
    text(&mut h, "Duration ms", "150"); click(&mut h, "Apply frame draft"); click(&mut h, "Undo clip edit");
    for invalid in ["0", "-1", "1.5", "18446744073709551615"] {
        text(&mut h, "Duration ms", invalid); click(&mut h, "Apply frame draft");
        assert_eq!(h.state().source(), &original); assert!(h.state().can_redo());
        assert!(h.state().has_pending_draft());
        assert!(h.get_by_label("Undo clip edit").accesskit_node().is_disabled());
        click(&mut h, "Revert frame draft");
    }
    click(&mut h, "Redo clip edit"); assert_eq!(h.state().source().clips[0].frames[0].duration_ms, 150);
}
#[test]
fn widget_history_is_bounded_and_region_choice_is_transactional() {
    let mut h = harness();
    choose(&mut h, "Region", "Region 20"); assert_eq!(h.state().source().clips[0].frames[0].region, 10);
    click(&mut h, "Apply frame draft"); assert_eq!(h.state().source().clips[0].frames[0].region, 20);
    for value in 101..=140 { text(&mut h, "Duration ms", &value.to_string()); click(&mut h, "Apply frame draft"); }
    for _ in 0..32 { assert!(h.state().can_undo()); click(&mut h, "Undo clip edit"); }
    assert!(!h.state().can_undo()); assert_eq!(h.state().source().clips[0].frames[0].duration_ms, 108);
}
#[cfg(target_os = "linux")]
#[test]
fn widget_save_as_preserves_pixels_license_and_existing_destination() {
    let root = tempfile::tempdir().unwrap(); let output = root.path().canonicalize().unwrap().join("new-package");
    let mut h = harness(); let (_, original_files) = admitted();
    text(&mut h, "Duration ms", "125"); click(&mut h, "Apply frame draft");
    text(&mut h, "New package directory", output.to_str().unwrap()); click(&mut h, "Save As new package");
    assert!(matches!(h.state().save_outcome(), SaveOutcome::Published { destination } if destination == &output));
    for name in ["atlas.png", "LICENSE.txt"] { assert_eq!(std::fs::read(output.join(name)).unwrap(), original_files[name]); }
    let saved = std::fs::read(output.join("sprite.json")).unwrap();
    let document: orr_sprite::SpriteSource = serde_json::from_slice(&saved).unwrap();
    assert_eq!(document.clips[0].frames[0].duration_ms, 125);
    assert_eq!(document.clips[1], h.state().source().clips[1]);
    click(&mut h, "Save As new package");
    assert!(matches!(h.state().save_outcome(), SaveOutcome::Failed { published: false, leftover_stage: None, .. }));
    assert_eq!(std::fs::read(output.join("sprite.json")).unwrap(), saved);
}
#[test]
fn widget_save_as_rejects_old_version_before_filesystem_write() {
    let root = tempfile::tempdir().unwrap(); let output = root.path().canonicalize().unwrap().join("must-not-exist");
    let mut h = harness();
    text(&mut h, "New package version", "1.0.0");
    text(&mut h, "New package directory", output.to_str().unwrap()); click(&mut h, "Save As new package");
    assert!(matches!(h.state().save_outcome(), SaveOutcome::Failed { published: false, leftover_stage: None, .. }));
    assert!(h.state().message().contains("strictly newer")); assert!(!output.exists());
}

#[test]
fn installed_package_admission_decodes_real_png_and_rejects_bad_atlas() {
    let temp = tempfile::tempdir().unwrap();
    let package_dir = temp.path().join("package");
    let project_dir = temp.path().join("project");
    std::fs::create_dir(&package_dir).unwrap();
    std::fs::create_dir(&project_dir).unwrap();
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[255, 0, 0, 255, 0, 255, 0, 255]).unwrap();
        writer.finish().unwrap();
    }
    let mut manifest = Manifest { schema: 1, name: "fixture.sprite".into(), version: "1.0.0".into(),
        engine: "*".into(), capabilities: ["sprite".into()].into(), dependencies: BTreeMap::new(),
        files: ["sprite.json".into(), "atlas.png".into(), "LICENSE.txt".into()].into() };
    std::fs::write(package_dir.join("orr.package.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
    std::fs::write(package_dir.join("sprite.json"), source()).unwrap();
    std::fs::write(package_dir.join("atlas.png"), &bytes).unwrap();
    std::fs::write(package_dir.join("LICENSE.txt"), b"fixture license\n").unwrap();
    let mut runtime = orr_package::Runtime::content_only();
    runtime.capabilities.insert("sprite".into());
    let project = orr_package::Project::open(&project_dir, runtime.clone()).unwrap();
    // Package admission rejects periods; retain this exact negative before the valid fixture.
    assert_eq!(project.install(std::slice::from_ref(&package_dir)).unwrap_err().to_string(),
        "invalid package/capability name");
    assert!(!project_dir.join("orr.packages.lock.json").exists());
    manifest.name = "fixture-sprite".into();
    std::fs::write(package_dir.join("orr.package.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
    project.install(std::slice::from_ref(&package_dir)).unwrap();
    let before = std::fs::read(project_dir.join("orr.packages.lock.json")).unwrap();
    let panel = ClipPanel::open_installed(&project_dir, "fixture-sprite", "sprite.json", "walk", "1.0.1").unwrap();
    assert_eq!(panel.source().atlas.width, 2);
    assert_eq!(std::fs::read(project_dir.join("orr.packages.lock.json")).unwrap(), before);
    assert!(!panel.has_pending_draft());
    let bad_project_dir = temp.path().join("bad-project");
    std::fs::create_dir(&bad_project_dir).unwrap();
    std::fs::write(package_dir.join("atlas.png"), b"not a PNG").unwrap();
    let bad = orr_package::Project::open(&bad_project_dir, runtime).unwrap();
    bad.install(std::slice::from_ref(&package_dir)).unwrap();
    assert!(ClipPanel::open_installed(&bad_project_dir, "fixture-sprite", "sprite.json", "walk", "1.0.1").is_err());
}
