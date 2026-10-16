//! Real EditorApp widgets, verified installed PNG assets, and a local Arena host.
//! Presentation edits never become scene edits or host history entries.
#![cfg(feature = "sprites")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

#[path = "common/arena_sprites.rs"]
mod fixture;

use egui_kittest::kittest::{NodeT, Queryable};
use fixture::*;
use orr_editor::{
    sprite_bindings::{Bindings, Source},
    sprite_panel::SpritePanel,
};
use serde_json::json;

#[test]
fn real_widgets_keep_two_bindings_and_follow_target_independent_and_saveable() {
    let fixture = Fixture::new();
    let mut h = fixture.headless();
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().entries.len();
    let scene_bytes = std::fs::read(&fixture.scene).unwrap();
    two_locomotion_bindings(&mut h, &fixture);
    assert_eq!(region(h.state(), HERO), Some(10));
    assert_eq!(region(h.state(), TARGET), Some(20));
    click(&mut h, "Follow selected entity");
    assert_eq!(document(&h).camera_follow.as_deref(), Some(TARGET));
    click(&mut h, "hero");
    click(&mut h, "Follow selected entity");
    assert_eq!(document(&h).camera_follow.as_deref(), Some(HERO));
    click(&mut h, "Undo binding");
    assert_eq!(document(&h).camera_follow.as_deref(), Some(TARGET));
    click(&mut h, "Redo binding");
    assert_eq!(document(&h).camera_follow.as_deref(), Some(HERO));
    click(&mut h, "Clear camera follow");
    assert_eq!(document(&h).camera_follow, None);
    click(&mut h, "Undo binding");
    assert_eq!(document(&h).camera_follow.as_deref(), Some(HERO));

    // Legacy one-clip and region bindings can coexist with a locomotion binding.
    assign(&mut h, "target", Source::Region(21));
    assert_eq!(region(h.state(), HERO), Some(10));
    assert_eq!(region(h.state(), TARGET), Some(21));
    assign(&mut h, "target", Source::Clip("greet".into()));
    assert_eq!(region(h.state(), TARGET), Some(10));
    click(&mut h, "Remove selected sprite bindings");
    assert_eq!(document(&h).bindings.len(), 1);
    click(&mut h, "Undo binding");
    assert_eq!(document(&h).bindings.len(), 2);
    assert_eq!(document(&h).camera_follow.as_deref(), Some(HERO));
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), history);
    assert_eq!(std::fs::read(&fixture.scene).unwrap(), scene_bytes);

    assert!(!fixture.sidecar.exists());
    assert!(h.state_mut().editor.save());
    settle(&mut h);
    assert!(!fixture.sidecar.exists(), "scene Save is not sidecar Save");
    let saved_scene_bytes = std::fs::read(&fixture.scene).unwrap();
    click(&mut h, "Save bindings");
    no_error(&h);
    assert!(!h.state().sprites.bindings.as_ref().unwrap().dirty());
    let expected = document(&h).clone();
    assert_eq!(
        Bindings::open(fixture.sidecar.clone()).unwrap().document(),
        &expected
    );
    click(&mut h, "Close bindings");
    assert!(h.state().sprites.bindings.is_none());
    click(&mut h, "Open bindings");
    no_error(&h);
    assert_eq!(document(&h), &expected);
    assert_eq!(region(h.state(), HERO), Some(10));
    assert_eq!(region(h.state(), TARGET), Some(10));
    assert_eq!(std::fs::read(&fixture.scene).unwrap(), saved_scene_bytes);

    let reopened = fixture.editor();
    let mut panel = SpritePanel::default();
    panel
        .open(&egui::Context::default(), fixture.sidecar.clone())
        .unwrap();
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &expected);
    assert_eq!(panel.sampled_region(&reopened, HERO), Some(10));
    assert_eq!(panel.sampled_region(&reopened, TARGET), Some(10));
}

#[test]
fn failed_asset_clip_save_and_scene_operations_preserve_authored_state() {
    let fixture = Fixture::new();
    let mut h = fixture.headless();
    two_locomotion_bindings(&mut h, &fixture);
    click(&mut h, "hero");
    click(&mut h, "Follow selected entity");
    let valid = document(&h).clone();
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().entries.len();
    let scene_bytes = std::fs::read(&fixture.scene).unwrap();

    for source in [
        Source::Clip("missing".into()),
        Source::Locomotion {
            idle: "idle".into(),
            walk: "missing".into(),
        },
        Source::Locomotion {
            idle: "missing".into(),
            walk: "walk".into(),
        },
        Source::Region(u32::MAX),
    ] {
        h.state_mut().sprites.source = source;
        click(&mut h, "Assign sprite to selection");
        assert!(
            h.state().sprites.error().is_some(),
            "invalid source must be diagnosed"
        );
        assert_eq!(document(&h), &valid);
        assert_eq!(region(h.state(), HERO), Some(10));
    }
    h.state_mut().sprites.source = valid.bindings[HERO].source.clone();
    for package in ["missing-package", "../sample-sprites"] {
        h.state_mut().sprites.package = package.into();
        click(&mut h, "Load sprite document");
        assert!(h.state().sprites.error().is_some());
        assert_eq!(document(&h), &valid);
        assert_eq!(region(h.state(), HERO), Some(10));
    }
    h.state_mut().sprites.package = PACKAGE.into();
    h.state_mut().sprites.document = "../sprites.json".into();
    click(&mut h, "Load sprite document");
    assert!(h.state().sprites.error().is_some());
    assert_eq!(document(&h), &valid);
    h.state_mut().sprites.document = "sprites.json".into();

    // An actual failed atomic save must retain both history and dirty content.
    let blocked = fixture.root.path().join("directory-instead-of-file");
    std::fs::create_dir(&blocked).unwrap();
    h.state_mut().sprites.bindings.as_mut().unwrap().path = blocked;
    click(&mut h, "Save bindings");
    assert!(h.state().sprites.error().is_some());
    assert!(h.state().sprites.bindings.as_ref().unwrap().dirty());
    assert_eq!(document(&h), &valid);
    h.state_mut().sprites.bindings.as_mut().unwrap().path = fixture.sidecar.clone();
    click(&mut h, "Save bindings");
    no_error(&h);
    let saved_bytes = std::fs::read(&fixture.sidecar).unwrap();
    let bad_sidecar = fixture.root.path().join("invalid.sprites.json");
    std::fs::write(&bad_sidecar, br#"{"version":999}"#).unwrap();
    assert!(h
        .state_mut()
        .sprites
        .open(&egui::Context::default(), bad_sidecar)
        .is_err());
    assert_eq!(document(&h), &valid);
    assert_eq!(region(h.state(), HERO), Some(10));

    let other_scene = fixture.root.path().join("other.scene.yaml");
    std::fs::write(&other_scene, SCENE).unwrap();
    assert!(h.state_mut().editor.open_path(&other_scene));
    settle(&mut h);
    assert_eq!(
        document(&h),
        &valid,
        "scene changes retain authored sidecar"
    );
    assert_eq!(
        region(h.state(), HERO),
        None,
        "wrong scene never paints same-looking GUIDs"
    );
    assert!(h
        .get_by_label("Assign sprite to selection")
        .accesskit_node()
        .is_disabled());
    assert!(h
        .get_by_label("Follow selected entity")
        .accesskit_node()
        .is_disabled());
    assert_eq!(std::fs::read(&fixture.sidecar).unwrap(), saved_bytes);
    assert_eq!(std::fs::read(&fixture.scene).unwrap(), scene_bytes);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), history);
}

#[test]
fn orphaned_and_recycled_entity_handles_cannot_inherit_sprite_or_follow_identity() {
    let fixture = Fixture::new();
    let mut h = fixture.headless();
    two_locomotion_bindings(&mut h, &fixture);
    click(&mut h, "target");
    click(&mut h, "Follow selected entity");
    let old_entity = h
        .state()
        .editor
        .row_of(h.state().editor.selection().unwrap())
        .unwrap()
        .entity;
    let before = document(&h).clone();
    assert!(h.state_mut().editor.delete_selected());
    assert_eq!(
        region(h.state(), TARGET),
        None,
        "stale rows must not resolve against a new snapshot"
    );
    settle(&mut h);
    assert_eq!(region(h.state(), TARGET), None);
    assert!(h
        .state()
        .sprites
        .diagnostics(&h.state().editor)
        .iter()
        .any(|message| message.contains(TARGET)));
    assert_eq!(
        document(&h),
        &before,
        "orphans remain repairable, never reassigned"
    );
    assert!(h.state_mut().editor.undo());
    settle(&mut h);
    assert_eq!(region(h.state(), TARGET), Some(20));
    assert!(h.state_mut().editor.select_named("target"));
    assert!(h.state_mut().editor.delete_selected());
    settle(&mut h);
    assert!(h.state_mut().editor.spawn_arena_player(1, [60.0, 0.0]));
    assert_eq!(
        region(h.state(), TARGET),
        None,
        "recycled snapshot cannot use stale GUID rows before settle"
    );
    settle(&mut h);
    let new_guid = h.state().editor.selected_guid().unwrap();
    let new_entity = h
        .state()
        .editor
        .row_of(h.state().editor.selection().unwrap())
        .unwrap()
        .entity;
    assert_ne!(new_guid.as_str(), TARGET);
    assert_eq!(
        new_entity.index, old_entity.index,
        "fixture exercises a recycled index"
    );
    // Edit rebakes may reuse generation zero; the stable GUID is authoritative.
    assert_eq!(region(h.state(), TARGET), None);
    assert_eq!(region(h.state(), new_guid.as_str()), None);
    assert_eq!(region(h.state(), HERO), Some(10));
    assert_eq!(document(&h).camera_follow.as_deref(), Some(TARGET));
    assert_eq!(document(&h), &before);

    // The host still contains only simulation components; the sidecar cannot
    // leak sprite or camera settings into reflected entity state.
    let world = h
        .state_mut()
        .editor
        .host_call("world.get", json!({"entity": new_guid.as_str()}))
        .unwrap();
    let text = world.to_string();
    assert!(!text.contains("Locomotion") && !text.contains("camera_follow"));
    let held_camera = h.state().editor.camera;
    click(&mut h, orr_editor::app::LBL_PLAY);
    wait_for(
        &mut h,
        "missing follow target diagnostic during live Play",
        |app| {
            app.editor
                .timeline()
                .is_some_and(|timeline| timeline.playing)
                && app.sprites.playback.diagnostic().is_some_and(|message| {
                    message.contains("Camera follow target") && message.contains(TARGET)
                })
        },
    );
    assert_eq!(
        h.state().editor.camera,
        held_camera,
        "missing target holds camera, never follows replacement handle"
    );
    assert_eq!(region(h.state(), TARGET), None);
    assert_eq!(region(h.state(), new_guid.as_str()), None);
    assert_eq!(document(&h), &before);
    pan_viewport(&mut h);
    assert!(h.state().sprites.playback.follow_suspended());
    assert_ne!(h.state().editor.camera, held_camera);
    assert_eq!(document(&h), &before);
    click(&mut h, orr_editor::app::LBL_STOP);
    assert_eq!(
        h.state().editor.camera,
        held_camera,
        "Stop restores edit camera even when the configured target never resolved"
    );
}

#[test]
fn changed_or_removed_package_cannot_replace_an_unselected_actors_valid_cached_clip() {
    use orr_package::{Project, Runtime};
    let fixture = Fixture::new();
    let mut h = fixture.headless();
    two_locomotion_bindings(&mut h, &fixture);
    assign(&mut h, "target", Source::Clip("greet".into()));
    click(&mut h, "Save bindings");
    no_error(&h);
    let before = document(&h).clone();
    let saved = std::fs::read(&fixture.sidecar).unwrap();
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().entries.len();
    let project =
        Project::open_for_install(fixture.root.path(), Runtime::content_only().engine_version)
            .unwrap();
    let original = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/sprite_demo")
        .canonicalize()
        .unwrap();
    click(&mut h, "Undo binding");
    no_error(&h);
    let undone = document(&h).clone();
    assert_eq!(region(h.state(), TARGET), Some(20));
    project.remove(PACKAGE).unwrap();
    click(&mut h, "Redo binding");
    assert!(h.state().sprites.error().is_some());
    assert_eq!(
        document(&h),
        &undone,
        "failed Redo rolls back the candidate document"
    );
    assert_eq!(
        region(h.state(), TARGET),
        Some(20),
        "failed Redo retains valid textures"
    );
    project.install(std::slice::from_ref(&original)).unwrap();
    click(&mut h, "Redo binding");
    no_error(&h);
    assert_eq!(
        document(&h),
        &before,
        "failed Redo did not consume redo history"
    );

    let source = fixture.root.path().join("replacement-package");
    std::fs::create_dir(&source).unwrap();
    for entry in std::fs::read_dir(&original).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), source.join(entry.file_name())).unwrap();
    }
    let asset_path = source.join("sprites.json");
    let original_asset = std::fs::read(&asset_path).unwrap();
    let mut asset: serde_json::Value = serde_json::from_slice(&original_asset).unwrap();
    asset["clips"]
        .as_array_mut()
        .unwrap()
        .retain(|clip| clip["id"] != "greet");
    std::fs::write(&asset_path, serde_json::to_vec(&asset).unwrap()).unwrap();
    project.remove(PACKAGE).unwrap();
    project.install(std::slice::from_ref(&source)).unwrap();

    click(&mut h, "hero");
    h.state_mut().sprites.source = Source::Region(10); // valid in the new package
    click(&mut h, "Assign sprite to selection");
    assert!(h
        .state()
        .sprites
        .error()
        .is_some_and(|message| message.contains("greet")));
    assert_eq!(document(&h), &before);
    assert!(!h.state().sprites.bindings.as_ref().unwrap().dirty());
    assert_eq!(region(h.state(), HERO), Some(10));
    assert_eq!(
        region(h.state(), TARGET),
        Some(10),
        "unselected actor retains the old greet clip/cache"
    );
    click(&mut h, "Reload assets");
    assert!(h.state().sprites.error().is_some());
    assert_eq!(document(&h), &before);
    assert_eq!(region(h.state(), TARGET), Some(10));

    project.remove(PACKAGE).unwrap();
    click(&mut h, "Reload assets");
    assert!(h.state().sprites.error().is_some());
    assert_eq!(document(&h), &before);
    assert_eq!(region(h.state(), HERO), Some(10));
    assert_eq!(region(h.state(), TARGET), Some(10));
    click(&mut h, "Undo binding");
    assert!(h.state().sprites.error().is_some());
    assert_eq!(
        document(&h),
        &before,
        "failed Undo restores current document"
    );
    assert_eq!(
        region(h.state(), TARGET),
        Some(10),
        "failed Undo retains valid cached clip"
    );
    click(&mut h, "Assign sprite to selection");
    assert!(h.state().sprites.error().is_some());
    assert_eq!(document(&h), &before);
    assert_eq!(std::fs::read(&fixture.sidecar).unwrap(), saved);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), history);

    std::fs::write(&asset_path, original_asset).unwrap();
    project.install(&[source]).unwrap();
    // The failed assignment must not add an undo item. The last successful
    // authored change remains target's greet assignment.
    click(&mut h, "Undo binding");
    no_error(&h);
    assert!(matches!(
        document(&h).bindings[TARGET].source,
        Source::Locomotion { .. }
    ));
    assert_eq!(document(&h).bindings[HERO], before.bindings[HERO]);
    click(&mut h, "Redo binding");
    no_error(&h);
    assert_eq!(document(&h), &before);
}
