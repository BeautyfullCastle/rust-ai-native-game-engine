use super::*;
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{
    editor::Owner, inspector::InspEvent, registered_project::PreparedRegisteredProject, Editor,
};
use orr_games::coop_defense_game::{DefenseEnemy, WavePacing};
use orr_reflect::Reflect;

fn pacing_snapshot(ticks: u32) -> Snapshot {
    Snapshot::new(
        SchemaKey::for_type(&scenario_types(), WAVE_PACING_NAME).unwrap(),
        WavePacing::describe(),
        WavePacing::describe().read(bytemuck::bytes_of(&WavePacing {
            spawn_interval_ticks: ticks,
        })),
    )
    .unwrap()
}

#[test]
fn registered_cadence_choices_preserve_custom_values_and_disabled_state() {
    let registry = registry();
    let snapshot = pacing_snapshot(181);
    assert!(registry.resolve(snapshot.key()).is_some());
    for editable in [true, false] {
        let mut h = Harness::builder().with_size([800.0, 400.0]).build_ui_state(
            |ui, events: &mut Vec<InspEvent>| {
                registry.show(ui, egui::Id::new("cadence"), &snapshot, editable, events)
            },
            Vec::new(),
        );
        h.run_steps(3);
        assert!(h.state().is_empty());
        if editable {
            assert!(h
                .query_by_label("Current pair interval: 181 ticks")
                .is_some());
            h.get_by_label("Every 140 ticks").click();
            h.run_steps(3);
            assert_eq!(
                h.state().as_slice(),
                [InspEvent::Set {
                    path: "spawn_interval_ticks".into(),
                    value: Value::Int(140)
                }]
            );
            h.state_mut().clear();
            h.get_by_label("Use generic fields").click();
            h.run_steps(3);
            assert!(h.query_by_label("Every 140 ticks").is_none());
            h.get_by_role(egui::accesskit::Role::TextInput).focus();
            h.run_steps(2);
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
            h.run_steps(2);
            h.get_by_role(egui::accesskit::Role::TextInput)
                .type_text("182");
            h.run_steps(2);
            h.key_press(egui::Key::Enter);
            h.run_steps(2);
            assert_eq!(
                h.state().as_slice(),
                [InspEvent::Set {
                    path: "spawn_interval_ticks".into(),
                    value: Value::Int(182)
                }]
            );
            h.state_mut().clear();
            assert!(h.state().is_empty());
        } else {
            assert!(h.query_by_label("Every 140 ticks").is_none());
            assert!(h.state().is_empty());
        }
    }
    for invalid in [34, 351] {
        assert!(snapshot
            .checked_edit(FieldEdit {
                field: "spawn_interval_ticks".into(),
                value: Value::Int(invalid)
            })
            .is_err());
    }
    let current = pacing_snapshot(70);
    let mut h = Harness::builder().with_size([800.0, 400.0]).build_ui_state(
        |ui, events: &mut Vec<InspEvent>| {
            registry.show(ui, egui::Id::new("cadence"), &current, true, events)
        },
        Vec::new(),
    );
    h.run_steps(3);
    h.get_by_label("Every 70 ticks").click();
    h.run_steps(3);
    assert!(h.state().is_empty(), "same choice must not add history");
}

#[test]
fn cadence_widget_edit_undo_redo_save_reopen_and_actual_play() {
    let dir = tempfile::tempdir().unwrap();
    let scene = dir.path().join("stormglass.scene.yaml");
    let project = dir.path().join("project.json");
    std::fs::write(&scene, orr_remote::coop_defense::INITIAL_SCENE).unwrap();
    std::fs::write(
        &project,
        include_bytes!("../../../scenes/stormglass.project.json"),
    )
    .unwrap();
    let open = || {
        Editor::open_registered_project(
            PreparedRegisteredProject::open(&project, crate::stormglass::registry()).unwrap(),
        )
        .unwrap()
    };
    let mut editor = open();
    editor.sync();
    let registry = registry();
    let snapshot = pacing_snapshot(35);
    let mut h = Harness::builder().with_size([800.0, 400.0]).build_ui_state(
        |ui, events: &mut Vec<InspEvent>| {
            registry.show(ui, egui::Id::new("cadence"), &snapshot, true, events)
        },
        Vec::new(),
    );
    h.run_steps(3);
    h.get_by_label("Every 350 ticks").click();
    h.run_steps(3);
    let [InspEvent::Set { path, value }] = h.state().as_slice() else {
        panic!("one real widget edit required")
    };
    assert!(editor.set_field(
        &Owner::Singleton(WAVE_PACING_NAME.into()),
        path,
        value.clone()
    ));
    editor.sync();
    let ticks = |e: &Editor| {
        e.snapshot()
            .unwrap()
            .predicted()
            .singleton::<WavePacing>()
            .spawn_interval_ticks
    };
    assert_eq!(ticks(&editor), 350);
    assert_eq!(
        std::fs::read_to_string(&scene).unwrap(),
        orr_remote::coop_defense::INITIAL_SCENE
    );
    assert!(editor.undo());
    editor.sync();
    assert_eq!(ticks(&editor), 35);
    assert!(editor.redo());
    editor.sync();
    assert_eq!(ticks(&editor), 350);
    assert!(editor.save());
    drop(editor);
    let mut editor = open();
    editor.sync();
    assert_eq!(ticks(&editor), 350);
    editor
        .host_call("sim.start", serde_json::json!({}))
        .unwrap();
    assert!(editor
        .host_call(
            "world.singleton.patch",
            serde_json::json!({"name":WAVE_PACING_NAME,"path":"spawn_interval_ticks","value":70})
        )
        .is_err());
    editor
        .host_call("sim.step", serde_json::json!({"n":349}))
        .unwrap();
    editor.sync();
    assert!(!editor
        .snapshot()
        .unwrap()
        .predicted()
        .dense::<DefenseEnemy>()
        .1
        .iter()
        .any(|e| e.active != 0));
    editor
        .host_call("sim.step", serde_json::json!({"n":1}))
        .unwrap();
    editor.sync();
    assert_eq!(
        editor
            .snapshot()
            .unwrap()
            .predicted()
            .dense::<DefenseEnemy>()
            .1
            .iter()
            .filter(|e| e.active != 0)
            .count(),
        2
    );
    assert_eq!(ticks(&editor), 350);
}
