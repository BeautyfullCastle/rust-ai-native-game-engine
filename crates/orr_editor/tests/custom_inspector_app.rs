#![cfg(feature = "custom-inspector")]
#![allow(clippy::disallowed_types)]

#[path = "../examples/arena_custom_inspector/panels.rs"]
mod panels;

use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{custom_inspector::*, game::EditorGame, Editor, EditorApp, Mode};
use orr_reflect::Value;

const SCENE: &str = "schema: orr.scene/1\nsingletons:\n  Score: { kills: [0,0,0,0,0,0,0,0] }\nentities:\n  e_00000001:\n    name: hero\n    Position: { pos: [-300,0] }\n    PlayerTag: { slot: 0 }\n  e_00000002:\n    name: projectile\n    Position: { pos: [0,200] }\n    Bullet: { velocity: [1,0], owner_slot: 0, ttl: 60 }\n";

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
