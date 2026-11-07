//! Actual CPU import widgets -> new package -> verified binding -> saved Room policy.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::{
    fs,
    time::{Duration, Instant},
};
const SOURCE: &[u8] =
    include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json");
fn click<T>(h: &mut Harness<'_, T>, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(3);
}
fn text<T>(h: &mut Harness<'_, T>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    h.run_steps(3);
}
fn open(root: &Path) -> orr_sample::room_project::PreparedProject {
    orr_sample::room_project::PreparedProject::open_with_capabilities(
        root,
        orr_sample::room_project::RoomProjectCapabilities {
            character: true,
            lighting: true,
        },
    )
    .unwrap()
}
fn wait(panel: &mut Panel) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while panel.busy() && Instant::now() < deadline {
        panel.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!panel.busy(), "{}", panel.status);
}
pub(super) fn authored_room() -> crate::package_manager::presentation_tests::Fixture {
    let f = crate::package_manager::presentation_tests::fixture(true, true);
    let initial = open(&f.root);
    let before = initial.scene().frame().to_bytes();
    let before_checksum = initial.scene().frame().checksum();
    drop(initial);
    let source = f.temp.path().join("trim-source.animated.json");
    fs::write(&source, SOURCE).unwrap();
    let license = f.temp.path().join("trim-LICENSE.txt");
    fs::write(&license, b"Fixture rights explicitly reviewed").unwrap();
    let destination = f.temp.path().join("trimmed model package");
    let mut h = Harness::builder()
        .with_size([1600.0, 2600.0])
        .build_ui_state(
            |ui,
             s: &mut (
                crate::model_package_import_panel::Panel,
                PathBuf,
                Option<crate::model_package_import::Report>,
            )| {
                if let Some(report) = s.0.poll(Some(&s.1), 4, 7) {
                    s.2 = Some(report);
                }
                s.0.show(ui, true, Some(&s.1), 4, 7);
            },
            (
                crate::model_package_import_panel::Panel::default(),
                f.root.clone(),
                None,
            ),
        );
    h.run_steps(3);
    click(&mut h, "Import a model package");
    click(&mut h, "Cooked animated model");
    for (label, value) in [
        (
            "Cooked animated model file (.animated.json)",
            source.to_str().unwrap(),
        ),
        ("License file", license.to_str().unwrap()),
        (
            "Package destination directory",
            destination.to_str().unwrap(),
        ),
        ("Package name", "trimmed-courier"),
        ("Package version", "1.0.0"),
    ] {
        text(&mut h, label, value);
    }
    click(&mut h, "Inspect animated source");
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.query_by_label("Clip 2: escaped (1.000s)").is_none() {
        h.run_steps(3);
        assert!(Instant::now() < deadline, "{}", h.state().0.status());
        std::thread::sleep(Duration::from_millis(5));
    }
    click(&mut h, "Create trimmed animation clip");
    click(&mut h, "Trim source clip");
    click(&mut h, "Trim source clip 2: escaped (1.000s)");
    text(&mut h, "Trim start seconds", "0.25");
    text(&mut h, "Trim end seconds", "0.75");
    text(&mut h, "New animation clip name", "trimmed-search");
    assert!(
        h.get_by_label("Import cooked animated package")
            .accesskit_node()
            .is_disabled()
    );
    click(&mut h, "Preview animated variant");
    let deadline = Instant::now() + Duration::from_secs(10);
    let preview = loop {
        h.run_steps(3);
        if let Some(preview) = h.state_mut().0.preview_frame() {
            break preview;
        }
        assert!(Instant::now() < deadline, "{}", h.state().0.status());
        std::thread::sleep(Duration::from_millis(5));
    };
    let original = orr_model::animation::AnimatedModel::from_bytes(SOURCE).unwrap();
    assert_eq!(preview.prepared.model.source().clips.len(), 4);
    assert_eq!(
        &preview.prepared.model.source().clips[..3],
        original.source().clips.as_slice()
    );
    assert_eq!(
        preview.prepared.model.source().clips[3].name,
        "trimmed-search"
    );
    assert_eq!(preview.prepared.model.source().clips[3].duration(), 0.5);
    assert!(!destination.exists());
    assert!(h.state().2.is_none());
    assert_eq!(open(&f.root).scene().frame().to_bytes(), before);
    assert_eq!(fs::read(&source).unwrap(), SOURCE);
    click(&mut h, "I have reviewed rights to distribute this content");
    click(&mut h, "Import cooked animated package");
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().2.is_none() {
        h.run_steps(3);
        assert!(Instant::now() < deadline, "{}", h.state().0.status());
        std::thread::sleep(Duration::from_millis(5));
    }
    let report = h.state_mut().2.take().unwrap();
    assert_eq!(report.source_sha256, preview.prepared.asset_sha256);
    assert_eq!(
        fs::read(report.root.join(&report.asset)).unwrap(),
        preview.prepared.model.to_bytes().unwrap()
    );
    assert_eq!(report.package, "trimmed-courier");
    drop(h);
    let prepared = open(&f.root);
    let character = prepared.character().unwrap().document.clone();
    let guid = orr_reflect::Guid::parse(&character.player).unwrap();
    let old = prepared.models().document.bindings[&character.player].clone();
    drop(prepared);
    let mut packages = Panel {
        root: Some(f.root.clone()),
        source: report.root.display().to_string(),
        source_identity: report.root.display().to_string(),
        ..Default::default()
    };
    packages.start_preview(report.root.clone());
    wait(&mut packages);
    assert!(packages.preview.is_some(), "{}", packages.status);
    packages.start_add();
    wait(&mut packages);
    packages.start_verify();
    wait(&mut packages);
    assert!(
        packages
            .verified
            .as_ref()
            .unwrap()
            .direct
            .contains_key(&report.package)
    );
    let asset = crate::model_bindings::load_asset_for_kind(
        &f.root,
        &report.package,
        &report.asset,
        ModelKind::Animated,
    )
    .unwrap();
    let binding = crate::model_bindings::Binding::from_animated_asset(
        report.package.clone(),
        report.asset.clone(),
        &asset,
        old.animation.unwrap(),
        old.transform,
    )
    .unwrap();
    let mut bindings =
        crate::model_bindings::Bindings::open(f.root.join("room.models.json")).unwrap();
    bindings
        .assign_validated(std::slice::from_ref(&guid), &binding, &asset)
        .unwrap();
    bindings.save().unwrap();
    let mut prepared = open(&f.root);
    let character = prepared.take_character().unwrap();
    let (_, path, scene, models) = prepared.into_parts();
    let mut editor = crate::Editor::start(&crate::HostSpec::PreparedRoom {
        scene: path,
        text: scene.text().into(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    editor
        .install_room_character(character.document.clone())
        .unwrap();
    let mut model_panel = crate::model_panel::ModelPanel::default();
    model_panel.install_room(models).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        editor.sync();
        if editor.yard_rows_coherent() && model_panel.scene_matches(&editor) {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let panel = crate::room_character_panel::Panel::new_for_editor(character, &editor).unwrap();
    let mut h = Harness::builder()
        .with_size([1600.0, 2600.0])
        .build_ui_state(
            |ui,
             s: &mut (
                crate::Editor,
                crate::model_panel::ModelPanel,
                crate::room_character_panel::Panel,
            )| {
                s.0.sync();
                s.2.show(ui, &mut s.0, &mut s.1);
            },
            (editor, model_panel, panel),
        );
    h.run_steps(3);
    click(&mut h, "Room character");
    click(&mut h, "Searching clip");
    click(&mut h, "Searching clip 3: trimmed-search (0.50s)");
    // Enable the existing pickup policy as well. Searching idle inherits clip3;
    // neutral native playback therefore uses it and retains the existing pose diagnostic.
    click(&mut h, "Play key pickup clip once");
    click(&mut h, "Apply character settings");
    assert_eq!(h.state().2.document().searching, 3);
    assert_eq!(h.state().2.document().schema, 6);
    let authored = h.state().2.document().clone();
    assert_eq!(authored.motion.unwrap().searching_idle, 3);
    click(&mut h, "Undo character edit");
    assert_eq!(h.state().2.document().searching, 0);
    click(&mut h, "Redo character edit");
    assert_eq!(h.state().2.document(), &authored);
    click(&mut h, "Save character and model bindings");
    assert!(!h.state().2.dirty());
    assert_eq!(
        h.state().0.snapshot().unwrap().predicted().checksum(),
        before_checksum
    );
    drop(h);
    let reopened = open(&f.root);
    assert_eq!(reopened.character().unwrap().document, authored);
    let saved = &reopened.models().document.bindings[&authored.player];
    assert_eq!(saved.package, "trimmed-courier");
    assert_eq!(saved.animation.unwrap().clip_index, 3);
    assert_eq!(saved.transform, old.transform);
    assert_eq!(reopened.scene().frame().to_bytes(), before);
    assert_eq!(fs::read(&source).unwrap(), SOURCE);
    f
}
#[test]
fn actual_trim_widgets_preview_import_install_assign_room_save_and_reopen() {
    let _fixture = authored_room();
}
