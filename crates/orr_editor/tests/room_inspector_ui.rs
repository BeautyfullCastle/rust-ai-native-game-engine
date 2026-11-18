//! A Room project shows readable entity names and only the panels that apply to it.
#![cfg(feature = "room-project")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{game::EditorGame, Editor, EditorApp, HostSpec};
use orr_reflect::Scene;
use orr_sample::{
    room_game::{RoomConfig, RoomEscapeV1},
    room_project,
};
use orr_sim::Simulation;

fn room_app(temp: &tempfile::TempDir) -> Harness<'static, EditorApp> {
    let simulation = Simulation::<RoomEscapeV1>::new(RoomConfig, 60, room_project::SEED);
    let text = Scene::unbake(&room_project::types(), simulation.frame(), None)
        .unwrap()
        .to_yaml();
    let scene = temp.path().join("room.scene.yaml");
    std::fs::write(&scene, &text).unwrap();
    let mut editor = Editor::start(&HostSpec::PreparedRoom {
        scene,
        text,
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    editor.sync();
    assert_eq!(editor.game(), EditorGame::RoomEscape);
    let mut harness = Harness::builder()
        .with_size([1400.0, 1000.0])
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |_| EditorApp::new(editor, None));
    for _ in 0..3 {
        harness.state_mut().editor.sync();
        harness.run_steps(2);
    }
    harness
}

#[test]
fn room_hierarchy_rows_show_actor_roles_and_not_guids() {
    let temp = tempfile::tempdir().unwrap();
    let harness = room_app(&temp);
    let rows = harness.state().editor.rows();
    assert!(!rows.is_empty(), "the Room scene has entities");
    for row in rows {
        let guid = row.guid.as_ref().expect("Room entities carry GUIDs").to_string();
        let label = row.label();
        assert_ne!(label, guid, "a Room row is labelled with its GUID");
        assert!(
            ["Player", "Key", "Exit", "Wall", "Floor"].contains(&label.as_str()),
            "unexpected Room row label {label}"
        );
    }
    // The hierarchy lists only the rows in view, so check the first row that is drawn.
    assert!(
        ["Player", "Key", "Exit", "Wall", "Floor"]
            .iter()
            .any(|role| harness.query_all_by_label(role).next().is_some()),
        "Hierarchy draws a role label"
    );
    for row in rows {
        let guid = row.guid.as_ref().unwrap().to_string();
        assert!(harness.query_all_by_label(&guid).next().is_none());
    }
}

#[cfg(feature = "terrain-surface")]
#[test]
fn room_project_hides_terrain_panels() {
    let temp = tempfile::tempdir().unwrap();
    let harness = room_app(&temp);
    assert!(harness.query_by_label("Terrain appearance").is_none());
    assert!(harness.query_by_label("Reload appearance").is_none());
}
