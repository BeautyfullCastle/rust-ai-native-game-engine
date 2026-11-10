use super::*;
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{registered_project::PreparedRegisteredProject, Editor, EditorApp, Mode};
use orr_games::coop_defense_game::{DefenseRun, WaveCounts};

const SCENE: &str = include_str!("../../../scenes/stormglass-counts.scene.yaml");
fn setup(registry: Registry) -> (tempfile::TempDir, Harness<'static, EditorApp>) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("stormglass.scene.yaml"), SCENE).unwrap();
    std::fs::write(
        dir.path().join("project.json"),
        include_bytes!("../../../scenes/stormglass.project.json"),
    )
    .unwrap();
    let mut editor = open(dir.path());
    editor.sync();
    let mut h = Harness::builder()
        .with_size([1800.0, 1500.0])
        .build_eframe(move |_| EditorApp::new(editor, None).with_custom_inspectors(registry));
    settle(&mut h);
    h.get_by_label(WAVE_COUNTS_NAME).click();
    settle(&mut h);
    (dir, h)
}
fn open(dir: &std::path::Path) -> Editor {
    Editor::open_registered_project(
        PreparedRegisteredProject::open(dir.join("project.json"), crate::stormglass::registry())
            .unwrap(),
    )
    .unwrap()
}
fn settle(h: &mut Harness<'_, EditorApp>) {
    h.state_mut().editor.sync();
    h.run_steps(3);
    h.state_mut().editor.sync();
}
fn counts(editor: &Editor) -> [u32; 10] {
    editor
        .snapshot()
        .unwrap()
        .predicted()
        .singleton::<WaveCounts>()
        .pair_counts
}

#[test]
fn actual_app_counts_bounds_undo_redo_save_reopen_and_play_consumption() {
    let (dir, mut h) = setup(super::super::cadence_inspector::registry());
    let initial = counts(&h.state().editor);
    assert_eq!(initial, [1, 2, 3, 16, 18, 20, 22, 24, 26, 28]);
    // Both inclusive bounds are visible and disabled clicks do not create history.
    for label in ["Wave 1: fewer pairs", "Wave 10: more pairs"] {
        h.get_by_label(label).click();
        settle(&mut h);
    }
    assert_eq!(h.state().editor.history().entries.len(), 0);
    h.get_by_label("Wave 1: more pairs").click();
    settle(&mut h);
    assert_eq!(counts(&h.state().editor)[0], 2);
    assert_eq!(h.state().editor.history().entries.len(), 1);
    h.get_by_label("Wave 10: fewer pairs").click();
    settle(&mut h);
    let mut expected = initial;
    expected[0] = 2;
    expected[9] = 27;
    assert_eq!(
        counts(&h.state().editor),
        expected,
        "inactive waves stay independently authored"
    );
    assert_eq!(h.state().editor.history().entries.len(), 2);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("stormglass.scene.yaml")).unwrap(),
        SCENE
    );
    assert!(h.state_mut().editor.undo());
    settle(&mut h);
    assert_eq!(counts(&h.state().editor)[9], 28);
    assert!(h.state_mut().editor.redo());
    settle(&mut h);
    assert_eq!(counts(&h.state().editor), expected);
    // The same actual app offers the generic inspector and exact integer entry.
    h.get_by_label("Use generic fields").click();
    settle(&mut h);
    assert!(h.query_by_label("Wave 1: more pairs").is_none());
    h.get_by(|node| {
        node.role() == egui::accesskit::Role::TextInput && node.value().as_deref() == Some("27")
    })
    .focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by(|node| {
        node.role() == egui::accesskit::Role::TextInput && node.value().as_deref() == Some("27")
    })
    .type_text("25");
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    settle(&mut h);
    expected[9] = 25;
    assert_eq!(counts(&h.state().editor), expected);
    h.get_by_label("Use generic fields").click();
    settle(&mut h);
    assert!(h.query_by_label("Wave 10: 25 pairs").is_some());
    assert!(h.state_mut().editor.save());
    settle(&mut h);
    let mut reopened = open(dir.path());
    reopened.sync();
    assert_eq!(counts(&reopened), expected);
    reopened
        .host_call("sim.start", serde_json::json!({}))
        .unwrap();
    assert!(reopened
        .host_call(
            "world.singleton.patch",
            serde_json::json!({
                "name": WAVE_COUNTS_NAME, "path": "pair_counts[0]", "value": 28
            })
        )
        .is_err());
    reopened
        .host_call("sim.step", serde_json::json!({"n": 350}))
        .unwrap();
    reopened.sync();
    assert_eq!(
        reopened
            .snapshot()
            .unwrap()
            .predicted()
            .singleton::<DefenseRun>()
            .spawned,
        4,
        "two authored pairs must cap actual first-wave spawning at four enemies"
    );
    assert_eq!(counts(&reopened), expected);
    h.state_mut().editor.start_play();
    settle(&mut h);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    assert!(h.query_by_label("Wave 1: more pairs").is_none());
    assert!(h.state_mut().editor.stop().is_some());
    settle(&mut h);
    assert_eq!(counts(&h.state().editor), expected);
    assert!(h.query_by_label("Wave 1: more pairs").is_some());
}

fn invalid_draw(ui: &mut egui::Ui, _: egui::Id, _: &Snapshot) -> Option<FieldEdit> {
    for (label, field, index, value) in [
        ("Below bound", "pair_counts", 0, Value::Int(0)),
        ("Above bound", "pair_counts", 9, Value::Int(29)),
        ("Bad index", "pair_counts", 10, Value::Int(1)),
        ("Bad type", "pair_counts", 0, Value::Bool(true)),
        ("Nested path", "pair_counts[0]", 0, Value::Int(1)),
        ("Same value", "pair_counts", 0, Value::Int(1)),
    ] {
        if ui.button(label).clicked() {
            return Some(FieldEdit {
                field: FieldTarget::ArrayElement {
                    field: field.into(),
                    index,
                },
                value,
            });
        }
    }
    None
}
fn test_registry(schema: SchemaKey) -> Registry {
    let mut registry = Registry::default();
    registry
        .register(Extension {
            id: "count-test",
            schema,
            priority: 0,
            api_version: 1,
            draw: invalid_draw,
        })
        .unwrap();
    registry
}

#[test]
fn actual_app_invalid_array_requests_and_noop_preserve_state_history_and_file() {
    let (dir, mut h) = setup(test_registry(
        SchemaKey::for_type(&scenario_types(), WAVE_COUNTS_NAME).unwrap(),
    ));
    let checksum = h.state().editor.checksum();
    for label in [
        "Below bound",
        "Above bound",
        "Bad index",
        "Bad type",
        "Nested path",
        "Same value",
    ] {
        h.get_by_label(label).click();
        settle(&mut h);
        assert_eq!(h.state().editor.checksum(), checksum, "{label}");
        assert_eq!(h.state().editor.history().entries.len(), 0, "{label}");
        assert!(!h.state().editor.sim().dirty, "{label}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("stormglass.scene.yaml")).unwrap(),
            SCENE
        );
    }
}

#[test]
fn actual_app_mismatched_array_schema_or_type_uses_generic_fallback() {
    for wrong_type in [false, true] {
        let mut schema = SchemaKey::for_type(&scenario_types(), WAVE_COUNTS_NAME).unwrap();
        if wrong_type {
            schema.type_name = "Other::WaveCounts".into();
        } else {
            schema.schema_json.push(' ');
        }
        let (_dir, h) = setup(test_registry(schema));
        assert!(h.query_by_label("Bad index").is_none());
        assert!(h.query_by_label("pair_counts [10]").is_some());
        assert_eq!(h.state().editor.history().entries.len(), 0);
    }
}
