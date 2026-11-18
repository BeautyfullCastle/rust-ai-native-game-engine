use super::*;
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{registered_project::PreparedRegisteredProject, Editor, EditorApp, Mode};
use orr_games::coop_defense_game::{DefenseRun, EnemyHealth};

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
        .with_size([1800.0, 1500.0])
        .build_eframe(move |_| EditorApp::new(editor, None).with_custom_inspectors(registry));
    settle(&mut h);
    h.get_by_label(ENEMY_HEALTH_NAME).click();
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
fn scale(editor: &Editor) -> u32 {
    editor
        .snapshot()
        .unwrap()
        .predicted()
        .singleton::<EnemyHealth>()
        .multiplier
}
fn only_health() -> Registry {
    let mut registry = Registry::default();
    register(&mut registry);
    registry
}

#[test]
fn actual_app_health_edit_undo_redo_generic_save_reopen_and_play() {
    let (dir, mut h) = setup(super::super::cadence_inspector::registry());
    assert_eq!(scale(&h.state().editor), 2);
    assert!(h.query_by_label("Enemy health scale x2 of x4").is_some());
    h.get_by_label("Set enemy health to maximum (x4)").click();
    settle(&mut h);
    assert_eq!(scale(&h.state().editor), 4);
    assert_eq!(h.state().editor.history().entries.len(), 1);
    assert!(h.state().editor.sim().dirty);
    // At the maximum the button is disabled, so a repeated click adds no history.
    h.get_by_label("Set enemy health to maximum (x4)").click();
    settle(&mut h);
    assert_eq!(h.state().editor.history().entries.len(), 1);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("stormglass.scene.yaml")).unwrap(),
        SCENE,
        "an edit alone never writes the scene file"
    );
    assert!(h.state_mut().editor.undo());
    settle(&mut h);
    assert_eq!(scale(&h.state().editor), 2);
    assert!(h.state_mut().editor.redo());
    settle(&mut h);
    assert_eq!(scale(&h.state().editor), 4);
    h.get_by_label("Reset enemy health to default (x1)").click();
    settle(&mut h);
    assert_eq!(scale(&h.state().editor), 1);
    // The same component remains editable through the generic widgets.
    h.get_by_label("Use generic fields").click();
    settle(&mut h);
    assert!(h
        .query_by_label("Set enemy health to maximum (x4)")
        .is_none());
    h.get_by(|node| {
        node.role() == egui::accesskit::Role::TextInput && node.value().as_deref() == Some("1")
    })
    .focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by(|node| {
        node.role() == egui::accesskit::Role::TextInput && node.value().as_deref() == Some("1")
    })
    .type_text("3");
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    settle(&mut h);
    assert_eq!(scale(&h.state().editor), 3);
    h.get_by_label("Use generic fields").click();
    settle(&mut h);
    assert!(h.state_mut().editor.save());
    settle(&mut h);
    let mut reopened = open(dir.path());
    reopened.sync();
    assert_eq!(scale(&reopened), 3);
    // Play mode rejects the same edit the Inspector would produce.
    reopened
        .host_call("sim.start", serde_json::json!({}))
        .unwrap();
    assert!(reopened
        .host_call(
            "world.singleton.patch",
            serde_json::json!({"name": ENEMY_HEALTH_NAME, "path": "multiplier", "value": 4})
        )
        .is_err());
    reopened
        .host_call("sim.step", serde_json::json!({"n": 250}))
        .unwrap();
    reopened.sync();
    assert!(
        reopened
            .snapshot()
            .unwrap()
            .predicted()
            .singleton::<DefenseRun>()
            .spawned
            > 0
    );
    assert_eq!(scale(&reopened), 3);
    h.state_mut().editor.start_play();
    settle(&mut h);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    assert!(h
        .query_by_label("Set enemy health to maximum (x4)")
        .is_none());
    assert!(h.state_mut().editor.stop().is_some());
    settle(&mut h);
    assert_eq!(scale(&h.state().editor), 3);
    assert!(h.query_by_label("Enemy health scale x3 of x4").is_some());
}

fn invalid_draw(ui: &mut egui::Ui, _: egui::Id, _: &Snapshot) -> Option<FieldEdit> {
    for (label, field, value) in [
        ("Below range", "multiplier", Value::Int(0)),
        ("Above range", "multiplier", Value::Int(5)),
        ("Bad type", "multiplier", Value::Bool(true)),
        ("Unknown field", "hp", Value::Int(1)),
        ("Nested path", "multiplier.x", Value::Int(1)),
        ("Same value", "multiplier", Value::Int(2)),
    ] {
        if ui.button(label).clicked() {
            return Some(FieldEdit {
                field: field.into(),
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
            id: "health-test",
            schema,
            priority: 0,
            api_version: 1,
            draw: invalid_draw,
        })
        .unwrap();
    registry
}

#[test]
fn actual_app_invalid_health_requests_and_noop_preserve_state_history_and_file() {
    let (dir, mut h) = setup(test_registry(
        SchemaKey::for_type(&scenario_types(), ENEMY_HEALTH_NAME).unwrap(),
    ));
    let checksum = h.state().editor.checksum();
    for label in [
        "Below range",
        "Above range",
        "Bad type",
        "Unknown field",
        "Nested path",
        "Same value",
    ] {
        h.get_by_label(label).click();
        settle(&mut h);
        assert_eq!(h.state().editor.checksum(), checksum, "{label}");
        assert_eq!(h.state().editor.history().entries.len(), 0, "{label}");
        assert!(!h.state().editor.sim().dirty, "{label}");
        assert_eq!(scale(&h.state().editor), 2, "{label}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("stormglass.scene.yaml")).unwrap(),
            SCENE,
            "{label}"
        );
    }
}

#[test]
fn actual_app_play_mode_refuses_custom_edits_without_changing_document() {
    let (_dir, mut h) = setup(test_registry(
        SchemaKey::for_type(&scenario_types(), ENEMY_HEALTH_NAME).unwrap(),
    ));
    let checksum = h.state().editor.checksum();
    h.state_mut().editor.start_play();
    settle(&mut h);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    // The custom panel is not offered while playing, so no intent can be produced.
    assert!(h.query_by_label("Same value").is_none());
    assert!(h.state_mut().editor.stop().is_some());
    settle(&mut h);
    assert_eq!(scale(&h.state().editor), 2);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), 0);
    assert!(h.query_by_label("Same value").is_some());
}

#[test]
fn mismatched_schema_or_type_uses_generic_fallback_and_duplicates_are_refused() {
    for wrong_type in [false, true] {
        let mut schema = SchemaKey::for_type(&scenario_types(), ENEMY_HEALTH_NAME).unwrap();
        if wrong_type {
            schema.type_name = "Other::EnemyHealth".into();
        } else {
            schema.schema_json.push(' ');
        }
        let (_dir, h) = setup(test_registry(schema));
        assert!(h.query_by_label("Below range").is_none());
        assert!(h.query_by_label("multiplier").is_some());
        assert_eq!(h.state().editor.history().entries.len(), 0);
    }
    assert!(SchemaKey::for_type(&scenario_types(), "CoopDefense::Missing").is_err());
    let mut registry = only_health();
    let before = registry.resolve(
        &SchemaKey::for_type(&scenario_types(), ENEMY_HEALTH_NAME).unwrap(),
    )
    .map(|e| e.id);
    assert_eq!(before, Some("stormglass-enemy-health"));
    // Same id or equal priority for the same schema is refused; the registry is unchanged.
    register_duplicate(&mut registry);
}

fn register_duplicate(registry: &mut Registry) {
    let extension = || Extension {
        id: "stormglass-enemy-health",
        schema: SchemaKey::for_type(&scenario_types(), ENEMY_HEALTH_NAME).unwrap(),
        priority: 0,
        api_version: 1,
        draw: invalid_draw,
    };
    assert!(registry.register(extension()).is_err());
    let mut other = extension();
    other.id = "other-id";
    assert!(registry.register(other).is_err(), "equal priority");
    let mut unsupported = extension();
    unsupported.id = "future";
    unsupported.api_version = 2;
    assert!(registry.register(unsupported).is_err());
    let key = SchemaKey::for_type(&scenario_types(), ENEMY_HEALTH_NAME).unwrap();
    assert_eq!(
        registry.resolve(&key).map(|e| e.id),
        Some("stormglass-enemy-health")
    );
}

// Writes the screenshot only when ORR_CUSTOM_INSPECTOR_SHOT names a target file.
#[test]
fn health_panel_renders_for_screenshot() {
    let (_dir, mut h) = setup(only_health());
    let Some(target) = std::env::var_os("ORR_CUSTOM_INSPECTOR_SHOT") else {
        return;
    };
    h.run_steps(3);
    let image = h.render().expect("WGPU readback must succeed");
    let file = std::fs::File::create(target).unwrap();
    let mut encoder = png::Encoder::new(file, image.width(), image.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(image.as_raw())
        .unwrap();
}
