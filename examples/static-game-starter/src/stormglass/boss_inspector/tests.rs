use super::*;
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{registered_project::PreparedRegisteredProject, Editor, EditorApp, Mode};
use orr_games::coop_defense_game::WaveComposition;

const SCENE: &str = include_str!("../../../scenes/stormglass-composition.scene.yaml");

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
        .with_size([1800.0, 1800.0])
        .build_eframe(move |_| EditorApp::new(editor, None).with_custom_inspectors(registry));
    settle(&mut h);
    h.get_by_label(WAVE_COMPOSITION_NAME).click();
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
fn composition(editor: &Editor) -> WaveComposition {
    *editor
        .snapshot()
        .unwrap()
        .predicted()
        .singleton::<WaveComposition>()
}

#[test]
fn actual_app_boss_toggle_undo_redo_save_reopen_and_play() {
    let (dir, mut h) = setup(super::super::cadence_inspector::registry());
    let initial = composition(&h.state().editor);
    assert_eq!(initial.opening_boss, [1, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
    h.get_by_label("Wave 1: remove opening boss").click();
    settle(&mut h);
    h.get_by_label("Wave 2: add opening boss").click();
    settle(&mut h);
    let mut expected = initial;
    expected.opening_boss[0] = 0;
    expected.opening_boss[1] = 1;
    let after = composition(&h.state().editor);
    assert_eq!(after, expected, "kind patterns stay untouched");
    assert_eq!(h.state().editor.history().entries.len(), 2);
    assert!(h.state_mut().editor.undo());
    settle(&mut h);
    assert_eq!(composition(&h.state().editor).opening_boss[1], 0);
    assert!(h.state_mut().editor.redo());
    settle(&mut h);
    assert_eq!(composition(&h.state().editor), expected);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("stormglass.scene.yaml")).unwrap(),
        SCENE
    );
    // Same component through the generic inspector.
    h.get_by_label("Use generic fields").click();
    settle(&mut h);
    assert!(h.query_by_label("Wave 2: remove opening boss").is_none());
    assert!(h.query_by_label("opening_boss [10]").is_some());
    h.get_by_label("Use generic fields").click();
    settle(&mut h);
    assert!(h.state_mut().editor.save());
    settle(&mut h);
    let mut reopened = open(dir.path());
    reopened.sync();
    assert_eq!(composition(&reopened), expected);
    reopened
        .host_call("sim.start", serde_json::json!({}))
        .unwrap();
    assert!(reopened
        .host_call(
            "world.singleton.patch",
            serde_json::json!({
                "name": WAVE_COMPOSITION_NAME, "path": "opening_boss[0]", "value": 1
            })
        )
        .is_err());
    assert_eq!(composition(&reopened), expected);
    let checksum = h.state().editor.checksum();
    h.state_mut().editor.start_play();
    settle(&mut h);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    assert!(h.query_by_label("Wave 1: add opening boss").is_none());
    assert!(h.state_mut().editor.stop().is_some());
    settle(&mut h);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert!(h.query_by_label("Wave 1: add opening boss").is_some());
}

fn invalid_draw(ui: &mut egui::Ui, _: egui::Id, _: &Snapshot) -> Option<FieldEdit> {
    for (label, field, index, value) in [
        ("Flag above range", "opening_boss", 0, Value::Int(2)),
        ("Index out of bounds", "opening_boss", 10, Value::Int(1)),
        ("Bad type", "opening_boss", 0, Value::Bool(true)),
        ("Nested array", "regular_kinds", 0, Value::Int(1)),
        ("Same value", "opening_boss", 0, Value::Int(1)),
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

#[test]
fn actual_app_invalid_boss_requests_preserve_state_history_and_file() {
    let mut registry = Registry::default();
    registry
        .register(Extension {
            id: "boss-test",
            schema: SchemaKey::for_type(&scenario_types(), WAVE_COMPOSITION_NAME).unwrap(),
            priority: 0,
            api_version: 1,
            draw: invalid_draw,
        })
        .unwrap();
    let (dir, mut h) = setup(registry);
    let checksum = h.state().editor.checksum();
    for label in [
        "Flag above range",
        "Index out of bounds",
        "Bad type",
        "Nested array",
        "Same value",
    ] {
        h.get_by_label(label).click();
        settle(&mut h);
        assert_eq!(h.state().editor.checksum(), checksum, "{label}");
        assert_eq!(h.state().editor.history().entries.len(), 0, "{label}");
        assert!(!h.state().editor.sim().dirty, "{label}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("stormglass.scene.yaml")).unwrap(),
            SCENE,
            "{label}"
        );
    }
}
