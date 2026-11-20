//! The settings of the scene window, applied to a Room project.
//!
//! A Room draws its lighting from the lights and singletons of its scene, so the window edits the
//! open Room like any other scene: a change sets component fields, is one undo step, and reaches
//! the lighting the player reads from the same frame.
//! cargo test -p orr_editor --features scenery,room-project,project-create --test scenery_scene_room
#![cfg(all(feature = "scenery", feature = "room-project", feature = "project-create"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

mod common;
#[path = "../../orr_sample/tests/support/room_lit_scene.rs"]
mod room_lit_scene;

use common::*;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use orr_bridge::FrameView;
use orr_editor::game::EditorGame;
use orr_editor::scenery_scene;
use orr_editor::scenery_window::Target as WindowTarget;
use orr_editor::{Editor, EditorApp, HostSpec, Owner};
use orr_reflect::Value;
use orr_sample::project_create::{create, CreateOptions};
use orr_sample::room_project::PreparedProject;
use orr_sample::room_view;
use orr_scenery::Settings;
use std::path::Path;

/// A new Room project whose scene text is `edit` of the template's, opened as the editor opens it.
fn room_editor(root: &Path, edit: impl FnOnce(&str) -> String) -> Editor {
    let project = root.join("room");
    create(&CreateOptions { output: project.clone(), template: "room-escape-3d-v1".into(), seed: "scene-window".into() }).unwrap();
    let scene = project.join("room.scene.yaml");
    std::fs::write(&scene, edit(&std::fs::read_to_string(&scene).unwrap())).unwrap();
    let (_, path, scene, _) = PreparedProject::open(&project).unwrap().into_parts();
    let mut editor = Editor::start(&HostSpec::PreparedRoom { scene: path, text: scene.text().into(), listen: None, debug_hooks: false }).unwrap();
    editor.sync();
    assert_eq!(editor.game(), EditorGame::RoomEscape);
    editor
}

fn number(value: &Value) -> f32 {
    let Value::Fixed(f) = value else { panic!("a fixed-point number, got {value:?}") };
    f.to_f32()
}

fn singleton_number(editor: &mut Editor, name: &str, path: &str) -> f32 {
    number(&editor.read_field(&Owner::Singleton(name.into()), path).unwrap())
}

/// Applies the control to the scene as the window does, and lets the editor take the answer in.
fn apply(editor: &mut Editor, group: &str, name: &str, settings: &Settings) -> usize {
    let changes = scenery_scene::changes(editor, group, name, settings);
    let accepted = scenery_scene::apply(editor, &changes);
    assert!(editor.status().is_none_or(|m| m.text.is_empty() || !m.error), "{group} {name}: the scene refused an edit");
    editor.sync();
    accepted
}

/// The lighting the player draws from the frame the editor shows.
fn drawn_lighting(editor: &Editor) -> orr_render::Lighting {
    room_view::lighting_for_frame(editor.snapshot().expect("a frame").predicted())
}

#[test]
fn a_room_with_lights_has_the_components_the_window_edits() {
    let root = tempfile::tempdir().unwrap();
    let editor = room_editor(root.path(), room_lit_scene::with_lights);
    assert!(scenery_scene::available(&editor));
    let settings = scenery_scene::read(&editor, Settings::default());
    assert!((settings.sun.intensity - 2.4).abs() < 1.0e-3, "sun intensity: {}", settings.sun.intensity);
    assert!((settings.sun.ambient - 0.4).abs() < 1.0e-3, "ambient: {}", settings.sun.ambient);
    // The room's actors are drawn as before, and the lights are markers of the scene's look.
    assert!(!editor.yard_frame().items.is_empty());
    let look = editor.yard_frame().look.as_ref().expect("the scene has lights");
    assert_eq!(look.lights.len(), 2);
    assert!(look.lights.iter().any(|light| light.sun));
    // What the player draws is what the scene says.
    let lighting = drawn_lighting(&editor);
    assert!((lighting.intensity - 2.4).abs() < 1.0e-3 && !lighting.shadows);
    assert_eq!(room_view::point_light_settings(editor.snapshot().unwrap().predicted()).point_light.map(|light| light.range), Some(12.0));
}

#[test]
fn a_room_without_lights_opens_with_default_components_and_no_sun_to_turn() {
    let root = tempfile::tempdir().unwrap();
    let mut editor = room_editor(root.path(), str::to_owned);
    assert!(scenery_scene::available(&editor), "the singletons come with their default values");
    let mut legacy = orr_sample::yard3d_view::yard_lighting();
    legacy.shadows = false;
    assert_eq!(drawn_lighting(&editor), legacy, "no light: the room's own lighting, as before");
    // Without a sun light the window has nothing to turn.
    assert!(scenery_scene::changes(&editor, "Sun", "Hour", &Settings::default()).is_empty());
    // The exposure is a field of the post process, so it still can be edited, and the player follows.
    let mut settings = scenery_scene::read(&editor, Settings::default());
    settings.look.exposure = 2.0;
    assert_eq!(apply(&mut editor, "Look", "Exposure", &settings), 1);
    assert!((singleton_number(&mut editor, "orr_gfx::PostProcess", "exposure") - 2.0).abs() < 1.0e-3);
    assert!((drawn_lighting(&editor).exposure - 2.0).abs() < 1.0e-3);
}

#[test]
fn a_changed_setting_sets_component_fields_reaches_the_player_and_undo_reverses_it() {
    let root = tempfile::tempdir().unwrap();
    let mut editor = room_editor(root.path(), room_lit_scene::with_lights);
    let sun = target_named(&editor, "sun");
    let mut settings = scenery_scene::read(&editor, Settings::default());
    settings.sun.intensity = 3.0;
    settings.sun.ambient = 0.7;
    settings.look.exposure = 1.7;
    settings.sun.hour = 8.0;
    let checksum = editor.checksum();
    for (group, name) in [("Sun", "Intensity"), ("Sun", "Ambient"), ("Look", "Exposure"), ("Sun", "Hour")] {
        assert!(apply(&mut editor, group, name, &settings) > 0, "{group} {name} changed nothing");
    }
    assert!((number(&field(&mut editor, &sun, "orr_gfx::Light", "intensity")) - 3.0).abs() < 1.0e-3);
    assert!((singleton_number(&mut editor, "orr_gfx::Environment", "ambient") - 0.7).abs() < 1.0e-3);
    assert!((singleton_number(&mut editor, "orr_gfx::PostProcess", "exposure") - 1.7).abs() < 1.0e-3);
    let back = scenery_scene::read(&editor, Settings::default());
    assert!((back.sun.hour - 8.0).abs() < 0.05, "hour: {}", back.sun.hour);
    assert_ne!(editor.checksum(), checksum);
    // The frame the player draws from carries the edit.
    let lighting = drawn_lighting(&editor);
    assert!((lighting.intensity - 3.0).abs() < 1.0e-3);
    assert!((lighting.ambient - 0.7).abs() < 1.0e-3);
    assert!((lighting.exposure - 1.7).abs() < 1.0e-3);

    // One undo step per control, in reverse order.
    for _ in 0..4 {
        assert!(editor.undo());
        editor.sync();
    }
    assert!(!editor.undo(), "nothing else was recorded");
    assert!((number(&field(&mut editor, &sun, "orr_gfx::Light", "intensity")) - 2.4).abs() < 1.0e-3);
    assert!((singleton_number(&mut editor, "orr_gfx::PostProcess", "exposure") - 1.0).abs() < 1.0e-3);
    assert_eq!(editor.checksum(), checksum, "undo restores the whole frame");
    assert!((drawn_lighting(&editor).intensity - 2.4).abs() < 1.0e-3);
}

#[test]
fn the_fields_of_a_point_light_and_its_rule_are_component_fields() {
    let root = tempfile::tempdir().unwrap();
    let mut editor = room_editor(root.path(), room_lit_scene::with_lights);
    let point = target_named(&editor, "point_light");
    let range = number(&field(&mut editor, &point, "orr_gfx::Light", "range"));
    assert!((range - 12.0).abs() < 1.0e-3);
    assert_eq!(number(&field(&mut editor, &point, "RoomEscapeV1::LightRule", "offset.y")), 2.0);
    // The scene the editor holds is admitted by the Room: it can be saved and reopened.
    let text = scene_text(&mut editor);
    orr_sample::room_project::PreparedScene::parse(&text).unwrap();
}

fn sun_components() -> serde_json::Value {
    serde_json::json!({
        "orr_gfx::Transform3": {"position": [0, 20, 0], "rotation": [46.12, -142.62, 0], "scale": [1, 1, 1]},
        "orr_gfx::Light": {
            "color": [1, 0.94, 0.82], "intensity": 2.1, "range": 10, "inner_angle": 20, "outer_angle": 30,
            "shadow_strength": 1, "shadow_range": 0, "shadow_softness": 1, "shadow_sharp_range": 0,
            "kind": "directional", "enabled": true, "cast_shadows": false,
            "layers": ["default", "layer2", "layer3", "layer4", "layer5", "layer6", "layer7", "layer8"],
            "shadow_cascades": 1
        }
    })
}

#[test]
fn a_light_is_added_to_a_room_in_one_spawn_and_nothing_else_may_be() {
    let root = tempfile::tempdir().unwrap();
    let mut editor = room_editor(root.path(), str::to_owned);
    let checksum = editor.checksum();
    // An entity without a room role that is not a light, and a light with another render component.
    let mut with_material = sun_components();
    with_material["orr_gfx::Material"] = serde_json::json!({});
    for (what, params) in [
        ("an empty entity", serde_json::json!({"name": "empty", "components": {}})),
        ("a light with a material", serde_json::json!({"name": "sun", "components": with_material})),
    ] {
        assert!(editor.host_call("world.spawn", params).is_err(), "{what} was admitted");
        editor.sync();
        assert_eq!(editor.checksum(), checksum, "{what} left a trace");
    }
    // A sun is one spawn: the window then has a light to turn.
    editor.host_call("world.spawn", serde_json::json!({"name": "sun", "components": sun_components()})).expect("a sun is a render entity");
    editor.sync();
    assert_ne!(editor.checksum(), checksum);
    assert!(!scenery_scene::changes(&editor, "Sun", "Hour", &Settings::default()).is_empty());
    assert!((drawn_lighting(&editor).intensity - 2.1).abs() < 1.0e-3);
    // A second sun is refused: a room has one sun and one point light.
    let with_sun = editor.checksum();
    assert!(editor.host_call("world.spawn", serde_json::json!({"name": "second sun", "components": sun_components()})).is_err());
    editor.sync();
    assert_eq!(editor.checksum(), with_sun);
    assert!(editor.undo());
    editor.sync();
    assert_eq!(editor.checksum(), checksum, "undo removes the sun");
}

#[test]
fn the_window_edits_the_open_room_when_asked() {
    let root = tempfile::tempdir().unwrap();
    let editor = room_editor(root.path(), room_lit_scene::with_lights);
    let mut h = Harness::builder().with_size([1500.0, 1000.0]).build_eframe(move |_cc| EditorApp::new(editor, None));
    h.run_steps(3);
    h.state_mut().scenery.open = true;
    h.run_steps(3);
    h.get_by_label("Edit the open scene").click();
    h.run_steps(4);
    assert_eq!(h.state().scenery.target, WindowTarget::OpenScene);
    assert!(h.state().scenery.edits_open_scene(&h.state().editor), "the Room is editable through the window");
    // The snapshot the player reads is the frame the window edits.
    let frame: FrameView<'_> = h.state().editor.snapshot().unwrap().predicted();
    assert_eq!(frame.count::<orr_gfx::Light>(), 2);
}
