#![cfg(feature = "custom-inspector")]
#![allow(clippy::disallowed_types)]

#[path = "../examples/arena_custom_inspector/panels.rs"]
mod panels;

use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{custom_inspector::*, game::EditorGame, Editor, EditorApp, Mode};
use orr_reflect::Value;

const SCENE: &str = include_str!("../examples/arena_custom_inspector/projectile.scene.yaml");

fn app(path: &std::path::Path, name: &str, registry: Registry) -> Harness<'static, EditorApp> {
    let mut editor = Editor::open_game(path, EditorGame::Arena).unwrap();
    editor.sync();
    assert!(editor.select_named(name));
    editor.sync();
    Harness::builder()
        .with_size([1400.0, 950.0])
        .build_eframe(move |_| EditorApp::new(editor, None).with_custom_inspectors(registry))
}
fn value(editor: &Editor, component: &str, field: &str) -> Value {
    editor
        .inspect()
        .unwrap()
        .components
        .iter()
        .find(|(name, _)| name == component)
        .unwrap()
        .1
        .field(field)
        .unwrap()
        .clone()
}
fn settle(h: &mut Harness<'_, EditorApp>) {
    h.state_mut().editor.sync();
    h.run_steps(2);
    h.state_mut().editor.sync();
}
fn scene() -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("arena.scene.yaml");
    std::fs::write(&path, SCENE).unwrap();
    (tmp, path)
}

#[test]
fn actual_app_custom_edit_undo_redo_save_reopen_and_play_preserve_document() {
    let (tmp, path) = scene();
    let mut h = app(&path, "hero", panels::panels().unwrap());
    settle(&mut h);
    let initial = h.state().editor.checksum();
    assert!(!h.state().editor.sim().dirty);
    h.get_by_label("Assign slot 1").click();
    settle(&mut h);
    assert_eq!(value(&h.state().editor, "PlayerTag", "slot"), Value::Int(1));
    assert_eq!(h.state().editor.history().entries.len(), 1);
    assert!(h.state().editor.sim().dirty);
    assert!(h.state_mut().editor.undo());
    settle(&mut h);
    assert_eq!(h.state().editor.checksum(), initial);
    assert!(h.state_mut().editor.redo());
    settle(&mut h);
    let edited = h.state().editor.checksum();
    let saved = tmp.path().join("saved.scene.yaml");
    assert!(h.state_mut().editor.save_as(&saved));
    settle(&mut h);
    assert!(!h.state().editor.sim().dirty);
    let mut reopened = Editor::open_game(&saved, EditorGame::Arena).unwrap();
    reopened.sync();
    assert!(reopened.select_named("hero"));
    reopened.sync();
    assert_eq!(value(&reopened, "PlayerTag", "slot"), Value::Int(1));
    assert_eq!(reopened.checksum(), edited);
    h.state_mut().editor.start_play();
    settle(&mut h);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    assert!(
        h.query_by_label("Assign slot 1").is_none(),
        "custom callback must not draw during Play"
    );
    assert!(h.state_mut().editor.stop().is_some());
    settle(&mut h);
    assert_eq!(h.state().editor.checksum(), edited);
    assert_eq!(std::fs::read_to_string(path).unwrap(), SCENE);
}

#[test]
fn second_consumer_uses_same_dispatch_and_generic_toggle() {
    let (_tmp, path) = scene();
    let mut h = app(&path, "projectile", panels::panels().unwrap());
    settle(&mut h);
    h.get_by_label("Set lifetime to 180 ticks").click();
    settle(&mut h);
    assert_eq!(value(&h.state().editor, "Bullet", "ttl"), Value::Int(180));
    let edited = h.state().editor.checksum();
    h.get_by_label("Use generic fields").click();
    settle(&mut h);
    assert!(h.query_by_label("Set lifetime to 180 ticks").is_none());
    assert_eq!(h.state().editor.checksum(), edited);
    // Escape cancels a generic-field draft without a document/Undo mutation.
    h.get_by(|node| {
        node.role() == egui::accesskit::Role::TextInput && node.value().as_deref() == Some("180")
    })
    .focus();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.get_by(|node| {
        node.role() == egui::accesskit::Role::TextInput && node.value().as_deref() == Some("180")
    })
    .type_text("90");
    h.run_steps(1);
    h.key_press(egui::Key::Escape);
    settle(&mut h);
    assert_eq!(h.state().editor.checksum(), edited);
    assert_eq!(h.state().editor.history().entries.len(), 1);
    // The editor also clears selection on Escape; reselect before continuing.
    assert!(h.state_mut().editor.select_named("projectile"));
    settle(&mut h);
    // The existing generic TextEdit commits through the same event/Undo path.
    h.get_by(|node| {
        node.role() == egui::accesskit::Role::TextInput && node.value().as_deref() == Some("180")
    })
    .focus();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.get_by(|node| {
        node.role() == egui::accesskit::Role::TextInput && node.value().as_deref() == Some("180")
    })
    .type_text("120");
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    settle(&mut h);
    assert_eq!(value(&h.state().editor, "Bullet", "ttl"), Value::Int(120));
    assert!(h.state_mut().editor.undo());
    settle(&mut h);
    assert_eq!(value(&h.state().editor, "Bullet", "ttl"), Value::Int(180));
    h.get_by_label("Use generic fields").click();
    settle(&mut h);
    assert!(h.query_by_label("Set lifetime to 180 ticks").is_some());
    assert!(h.state_mut().editor.undo());
    settle(&mut h);
    assert_eq!(value(&h.state().editor, "Bullet", "ttl"), Value::Int(60));
}

#[test]
fn invalid_custom_intent_leaves_history_checksum_and_saved_bytes_unchanged() {
    let (_tmp, path) = scene();
    let mut types = orr_reflect::TypeRegistry::new();
    orr_sample::arena_game::register_reflect(&mut types);
    let mut registry = Registry::default();
    registry
        .register(Extension {
            id: "invalid-slot-test",
            schema: SchemaKey::for_type(&types, "PlayerTag").unwrap(),
            priority: 0,
            api_version: 1,
            draw: |ui, _, _| {
                ui.button("Request invalid slot")
                    .clicked()
                    .then(|| FieldEdit {
                        field: "slot".into(),
                        value: Value::Int(8),
                    })
            },
        })
        .unwrap();
    let mut h = app(&path, "hero", registry);
    settle(&mut h);
    let before = h.state().editor.checksum();
    h.get_by_label("Request invalid slot").click();
    settle(&mut h);
    assert_eq!(h.state().editor.checksum(), before);
    assert!(h.state().editor.history().entries.is_empty());
    assert_eq!(value(&h.state().editor, "PlayerTag", "slot"), Value::Int(0));
    assert_eq!(std::fs::read_to_string(path).unwrap(), SCENE);
}

#[test]
fn selection_preview_and_schema_mismatch_do_not_invoke_wrong_callbacks() {
    let (_tmp, path) = scene();
    let mut h = app(&path, "hero", panels::panels().unwrap());
    settle(&mut h);
    assert!(h.query_by_label("Assign slot 1").is_some());
    assert!(h.state_mut().editor.select_named("projectile"));
    settle(&mut h);
    assert!(h.query_by_label("Assign slot 1").is_none());
    assert!(h.query_by_label("Set lifetime to 180 ticks").is_some());
    let before = h.state().editor.checksum();
    let proposal = h
        .state_mut()
        .editor
        .host_call(
            "proposal.begin",
            serde_json::json!({"label":"inspect preview"}),
        )
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    h.state_mut().editor.host_call("proposal.apply", serde_json::json!({"id":proposal,"ops":[{"op":"patch","entity":"e_00000002","component":"Bullet","path":"ttl","value":120}]})).unwrap();
    assert!(h.state_mut().editor.set_preview(Some(proposal)));
    settle(&mut h);
    assert!(h.query_by_label("Set lifetime to 180 ticks").is_none());
    assert_eq!(h.state().editor.checksum(), before);
    assert!(h.state().editor.history().entries.is_empty());
    assert!(h.state_mut().editor.set_preview(None));
    settle(&mut h);
    assert!(h.query_by_label("Set lifetime to 180 ticks").is_some());

    let mut types = orr_reflect::TypeRegistry::new();
    orr_sample::arena_game::register_reflect(&mut types);
    let mut key = SchemaKey::for_type(&types, "PlayerTag").unwrap();
    key.schema_json.push(' ');
    let mut mismatch = Registry::default();
    mismatch
        .register(Extension {
            id: "wrong-schema",
            schema: key,
            priority: 0,
            api_version: 1,
            draw: |_, _, _| panic!("mismatched schema callback invoked"),
        })
        .unwrap();
    let mut fallback = app(&path, "hero", mismatch);
    settle(&mut fallback);
    assert!(fallback.query_by_label("Use generic fields").is_none());
    assert!(fallback.state().editor.history().entries.is_empty());
}

#[test]
fn actual_app_generic_focus_commit_undo_redo_save_and_reopen() {
    let (tmp, path) = scene();
    let mut h = app(&path, "projectile", panels::panels().unwrap());
    settle(&mut h);
    h.get_by_label("Set lifetime to 180 ticks").click();
    settle(&mut h);
    h.get_by_label("Use generic fields").click();
    settle(&mut h);
    h.get_by(|n| {
        n.role() == egui::accesskit::Role::TextInput && n.value().as_deref() == Some("180")
    })
    .focus();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.get_by(|n| {
        n.role() == egui::accesskit::Role::TextInput && n.value().as_deref() == Some("180")
    })
    .type_text("120");
    h.run_steps(1);
    h.get_by(|n| {
        n.role() == egui::accesskit::Role::TextInput && n.value().as_deref() == Some("200")
    })
    .focus();
    settle(&mut h);
    assert_eq!(value(&h.state().editor, "Bullet", "ttl"), Value::Int(120));
    assert_eq!(h.state().editor.history().entries.len(), 2);
    assert!(h.state().editor.sim().dirty);
    settle(&mut h);
    assert_eq!(
        h.state().editor.history().entries.len(),
        2,
        "blur commits once"
    );
    assert!(h.state_mut().editor.undo());
    settle(&mut h);
    assert_eq!(value(&h.state().editor, "Bullet", "ttl"), Value::Int(180));
    assert!(h.state_mut().editor.redo());
    settle(&mut h);
    assert_eq!(value(&h.state().editor, "Bullet", "ttl"), Value::Int(120));
    let checksum = h.state().editor.checksum();
    let saved = tmp.path().join("focus-saved.scene.yaml");
    assert!(h.state_mut().editor.save_as(&saved));
    settle(&mut h);
    assert!(!h.state().editor.sim().dirty);
    let mut reopened = Editor::open_game(&saved, EditorGame::Arena).unwrap();
    reopened.sync();
    assert!(reopened.select_named("projectile"));
    reopened.sync();
    assert_eq!(value(&reopened, "Bullet", "ttl"), Value::Int(120));
    assert_eq!(reopened.checksum(), checksum);
    assert_eq!(std::fs::read_to_string(path).unwrap(), SCENE);
}

#[test]
fn projectile_velocity_preset_survives_save_and_changes_real_play_motion() {
    use orr_fp::{FPVec2, FP};
    let (tmp, path) = scene();
    let mut h = app(&path, "projectile", panels::panels().unwrap());
    settle(&mut h);
    let initial = h.state().editor.checksum();
    h.get_by_label("Velocity up").click();
    settle(&mut h);
    let up = Value::Vec2(FPVec2::new(FP::ZERO, FP::from_int(2)));
    assert_eq!(value(&h.state().editor, "Bullet", "velocity"), up);
    assert_eq!(value(&h.state().editor, "Bullet", "ttl"), Value::Int(60));
    assert_eq!(h.state().editor.history().entries.len(), 1);
    assert!(h.state().editor.sim().dirty);
    h.get_by_label("Velocity up").click();
    settle(&mut h);
    assert_eq!(h.state().editor.history().entries.len(), 1);
    assert!(h.state_mut().editor.undo());
    settle(&mut h);
    assert_eq!(h.state().editor.checksum(), initial);
    assert!(h.state_mut().editor.redo());
    settle(&mut h);
    let edited = h.state().editor.checksum();
    let saved = tmp.path().join("projectile.scene.yaml");
    assert!(h.state_mut().editor.save_as(&saved));
    settle(&mut h);
    assert!(!h.state().editor.sim().dirty);
    let saved_bytes = std::fs::read(&saved).unwrap();
    let mut reopened = app(&saved, "projectile", panels::panels().unwrap());
    settle(&mut reopened);
    assert_eq!(value(&reopened.state().editor, "Bullet", "velocity"), up);
    assert_eq!(reopened.state().editor.checksum(), edited);
    assert!(reopened.state_mut().editor.start_play());
    settle(&mut reopened);
    assert!(reopened.query_by_label("Velocity up").is_none());
    reopened.state_mut().editor.step(3);
    settle(&mut reopened);
    assert_eq!(
        value(&reopened.state().editor, "Position", "pos"),
        Value::Vec2(FPVec2::new(FP::ZERO, FP::from_int(206)))
    );
    assert!(reopened.state_mut().editor.stop().is_some());
    settle(&mut reopened);
    assert_eq!(reopened.state().editor.checksum(), edited);
    assert_eq!(std::fs::read(saved).unwrap(), saved_bytes);
    assert_eq!(std::fs::read_to_string(path).unwrap(), SCENE);
}

#[test]
fn projectile_direction_presets_and_generic_axis_share_history() {
    use orr_fp::{FPVec2, FP};
    let (_tmp, path) = scene();
    let mut h = app(&path, "projectile", panels::panels().unwrap());
    settle(&mut h);
    for (index, (label, x, y)) in [
        ("Velocity left", -2, 0),
        ("Velocity right", 2, 0),
        ("Velocity down", 0, -2),
        ("Velocity stopped", 0, 0),
        ("Velocity up", 0, 2),
    ]
    .into_iter()
    .enumerate()
    {
        h.get_by_label(label).click();
        settle(&mut h);
        assert_eq!(
            value(&h.state().editor, "Bullet", "velocity"),
            Value::Vec2(FPVec2::new(FP::from_int(x), FP::from_int(y)))
        );
        assert_eq!(h.state().editor.history().entries.len(), index + 1);
    }
    h.get_by_label("Use generic fields").click();
    settle(&mut h);
    // Unique current value 2 identifies velocity.y; changing it preserves x.
    h.get_by(|node| {
        node.role() == egui::accesskit::Role::TextInput && node.value().as_deref() == Some("2")
    })
    .focus();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.get_by(|node| {
        node.role() == egui::accesskit::Role::TextInput && node.value().as_deref() == Some("2")
    })
    .type_text("1.5");
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    settle(&mut h);
    assert_eq!(
        value(&h.state().editor, "Bullet", "velocity"),
        Value::Vec2(FPVec2::new(FP::ZERO, FP::from_raw(98304)))
    );
    assert_eq!(h.state().editor.history().entries.len(), 6);
    assert!(h.state_mut().editor.undo());
    settle(&mut h);
    assert_eq!(
        value(&h.state().editor, "Bullet", "velocity"),
        Value::Vec2(FPVec2::new(FP::ZERO, FP::from_int(2)))
    );
}
