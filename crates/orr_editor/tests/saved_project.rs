//! Real saved-project admission and EditorApp lifecycle; no runtime installation.
#![cfg(feature = "sprites")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

#[path = "common/saved_project.rs"]
mod fixture;

use egui_kittest::kittest::Queryable;
use fixture::*;
use orr_editor::{
    app::{LBL_PLAY, LBL_STOP},
    editor::input::Phase,
    sprite_bindings::Source,
    Mode,
};
use serde_json::json;

#[test]
fn checked_in_project_contains_faithful_verified_installed_package() {
    let fixture = SavedFixture::new();
    let project = orr_editor::sprite_bindings::open_project(fixture.root.path()).unwrap();
    let lock = project.verify().unwrap();
    assert_eq!(lock.direct.len(), 1);
    assert_eq!(lock.direct["sample-sprites"], "1.0.0");
    let installed = &lock.packages["sample-sprites"];
    let original = repository().join("assets/sprite_demo");
    let manifest: orr_package::Manifest =
        serde_json::from_slice(&std::fs::read(original.join("orr.package.json")).unwrap()).unwrap();
    assert_eq!(installed.manifest, manifest);
    assert_eq!(
        installed.digest,
        "952838c26cb40890a4c9b1bb9a3c25c5e2430bca3b33b4064b9d79efd7432f86"
    );
    for file in &manifest.files {
        assert_eq!(
            project.read_asset("sample-sprites", file).unwrap(),
            std::fs::read(original.join(file)).unwrap(),
            "installed {file} must faithfully copy the original package"
        );
    }
}

#[test]
fn saved_project_edit_save_play_stop_close_reopen_and_relocate() {
    let fixture = SavedFixture::new();
    let mut h = fixture.headless();
    settle(&mut h);
    assert_loaded(&h, HERO);
    let initial_checksum = h.state().editor.checksum();
    let initial_sidecar = std::fs::read(fixture.sidecar()).unwrap();
    edit_hero_x(&mut h);
    assert_ne!(h.state().editor.checksum(), initial_checksum);
    assert_eq!(h.state().editor.history().entries.len(), 1);
    h.get_by_label("File").click();
    h.run_steps(2);
    h.get_by(|node| {
        node.role() == egui::accesskit::Role::Button
            && node
                .label()
                .is_some_and(|label| label.starts_with("Save ") && !label.starts_with("Save As"))
    })
    .click();
    settle(&mut h);
    assert!(!h.state().editor.is_dirty());
    let saved_scene = std::fs::read(fixture.scene()).unwrap();
    assert_eq!(
        std::fs::read(fixture.sidecar()).unwrap(),
        initial_sidecar,
        "scene Save does not silently save the presentation sidecar"
    );

    open_inspector(&mut h, "target");
    click(&mut h, "Follow selected entity");
    assert!(h.state().sprites.bindings.as_ref().unwrap().dirty());
    click(&mut h, "Save bindings");
    assert_loaded(&h, TARGET);
    let authored = document(&h).clone();
    assert_eq!(
        authored.bindings[HERO].source,
        Source::Locomotion {
            idle: "idle".into(),
            walk: "walk".into()
        }
    );
    assert_eq!(
        authored.bindings[TARGET].source,
        Source::Locomotion {
            idle: "walk".into(),
            walk: "idle".into()
        }
    );
    let saved_sidecar = std::fs::read(fixture.sidecar()).unwrap();
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().entries.len();
    assert_eq!(history, 1, "sidecar edits are absent from host history");
    assert_eq!(std::fs::read(fixture.scene()).unwrap(), saved_scene);
    let camera = h.state().editor.camera;

    click(&mut h, LBL_PLAY);
    wait_for(&mut h, "saved-project live Play", |app| {
        app.editor.can_take_control()
    });
    click(&mut h, "Take control");
    wait_for(&mut h, "saved-project managed input", |app| {
        app.editor.input_phase() == Phase::Active
    });
    let start = body_position(h.state(), HERO);
    key(&h, egui::Key::ArrowRight, true);
    wait_for(&mut h, "saved-project independent walking actor", |app| {
        moving(app, HERO) && body_position(app, HERO)[0] > start[0]
    });
    assert!(!moving(h.state(), TARGET));
    assert!(matches!(region(h.state(), HERO), Some(20 | 21)));
    assert!(matches!(region(h.state(), TARGET), Some(20 | 21)));
    assert_eq!(
        h.state().editor.camera.center,
        body_position(h.state(), TARGET),
        "saved follow GUID selects actor two, not keyboard slot zero"
    );
    key(&h, egui::Key::ArrowRight, false);
    wait_for(&mut h, "saved-project idle after release", |app| {
        !moving(app, HERO)
    });
    click(&mut h, LBL_STOP);
    wait_stopped(&mut h, camera);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    assert_eq!(h.state().editor.camera, camera);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), history);
    assert_eq!(document(&h), &authored);
    assert_eq!(std::fs::read(fixture.scene()).unwrap(), saved_scene);
    assert_eq!(std::fs::read(fixture.sidecar()).unwrap(), saved_sidecar);
    let replay = h
        .state_mut()
        .editor
        .host_call(
            "verify.self",
            json!({"inputs":{"kind":"last_play"},"checks":["recording_matches"]}),
        )
        .unwrap();
    assert_eq!(
        replay["passed"], true,
        "view-only project bindings preserve recorded simulation checksums"
    );

    click(&mut h, "Close bindings");
    assert!(h.state().sprites.bindings.is_none());
    click(&mut h, "Open bindings");
    assert_loaded(&h, TARGET);
    assert_eq!(document(&h), &authored);
    drop(h);
    let mut reopened = fixture.headless();
    settle(&mut reopened);
    assert_loaded(&reopened, TARGET);
    assert_eq!(document(&reopened), &authored);
    assert_eq!(body_position(reopened.state(), HERO), [-48.0, 0.0]);
    assert_eq!(reopened.state().editor.checksum(), checksum);
    drop(reopened);
    let moved = fixture.relocate();
    relocated_child(
        moved.root.path(),
        "relocated_saved_project_cpu_child",
        checksum,
    );
}

#[test]
fn relocated_saved_project_cpu_child() {
    let Some(root) = child_root() else { return };
    let mut h = open_headless(&root);
    settle(&mut h);
    assert_loaded(&h, TARGET);
    assert_eq!(body_position(h.state(), HERO), [-48.0, 0.0]);
    assert_eq!(h.state().editor.checksum(), child_checksum());
    let camera = h.state().editor.camera;
    click(&mut h, LBL_PLAY);
    wait_for(&mut h, "relocated project follow identity", |app| {
        app.editor.mode() == Mode::Play && app.editor.camera.center == body_position(app, TARGET)
    });
    click(&mut h, LBL_STOP);
    wait_stopped(&mut h, camera);
    assert_eq!(h.state().editor.camera, camera);
    assert_eq!(h.state().editor.checksum(), child_checksum());
    assert_loaded(&h, TARGET);
}

#[test]
fn native_cli_rejects_ambiguous_or_missing_projects_before_window_startup() {
    let fixture = SavedFixture::new();
    let empty = tempdir();
    for (flag, value) in [
        ("--game", "arena"),
        ("--scene", "other.scene.yaml"),
        ("--connect", "ws://127.0.0.1:1"),
        ("--script", "other.script"),
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_orr_editor"))
            .arg("--project")
            .arg(fixture.root.path())
            .args([flag, value])
            .current_dir(empty.path())
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("--project") && error.contains(flag),
            "{error}"
        );
        assert!(
            !error.contains("eframe:"),
            "argument error must precede native window creation: {error}"
        );
    }
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_orr_editor"))
        .arg("--project")
        .arg(empty.path())
        .current_dir(empty.path())
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("orr.project.json"), "{error}");
    assert!(
        !error.contains("eframe:"),
        "project admission error must precede native window creation: {error}"
    );
}
