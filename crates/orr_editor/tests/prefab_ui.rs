//! Generic prefabs in the main editor: create from a selection, instantiate,
//! override marks and revert, apply to the prefab, update when the prefab file
//! changes, undo/redo and save/reopen. Real host, real widgets (headless egui).
#![cfg(feature = "scene-prefabs")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::{Duration, Instant};

use common::fixed;
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::game::EditorGame;
use orr_editor::{Editor, EditorApp, Target};
use orr_reflect::{Guid, Value};
use serde_json::{json, Value as J};

const JOINTS: &str = include_str!("../../../scenes/joints_2d_demo.scene.yaml");
const YARD: &str = include_str!("../../../scenes/yard3d_authoring.scene.yaml");
const BODY: &str = "orr_physics::Body";
const COLLIDER: &str = "orr_physics::Collider";
const JOINT: &str = "orr_physics::Joint";

type App = Harness<'static, EditorApp>;

fn open(root: &std::path::Path, game: EditorGame) -> Editor {
    let mut editor = Editor::open_game(&root.join("level.scene.yaml"), game).expect("open the scene");
    editor.sync();
    editor
}

fn project(text: &str, game: EditorGame) -> (tempfile::TempDir, Editor) {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    std::fs::write(root.path().join("level.scene.yaml"), text).unwrap();
    let editor = open(root.path(), game);
    (root, editor)
}

fn app(editor: Editor) -> App {
    let mut h = Harness::builder().with_size([1500.0, 1400.0]).build_eframe(move |_| EditorApp::new(editor, None));
    settle(&mut h);
    h
}

fn settle(h: &mut App) {
    for _ in 0..3 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
}

fn click(h: &mut App, label: &str) {
    h.get_all_by_label(label).next().unwrap_or_else(|| panic!("no widget labeled {label}")).click();
    h.run_steps(3);
    settle(h);
}

fn wait(h: &mut App, what: &str, mut ready: impl FnMut(&mut App) -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        settle(h);
        if ready(h) {
            return;
        }
        assert!(Instant::now() < end, "timed out waiting for {what}");
        sleep(Duration::from_millis(30));
    }
}

/// Number of override marks (Revert buttons) the inspector shows.
fn marks(h: &App) -> usize {
    h.query_all_by_label("Revert").count()
}

/// Types the file name of the next prefab into the panel's text field.
fn name_prefab(h: &mut App, name: &str) {
    use egui::accesskit::Role;
    h.get_by(|n| n.role() == Role::TextInput && n.value().is_some_and(|v| v.ends_with(".prefab.yaml"))).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by(|n| n.role() == Role::TextInput && n.value().is_some_and(|v| v.ends_with(".prefab.yaml"))).type_text(name);
    h.run_steps(2);
    settle(h);
}

fn target(guid: &str) -> Target {
    Target::Guid(Guid::parse(guid).unwrap())
}

fn select(h: &mut App, guid: &str) {
    h.state_mut().editor.select(Some(target(guid)));
    settle(h);
}

fn list(h: &mut App) -> Vec<J> {
    h.state_mut().editor.host_call("prefab_instance.list", json!({})).unwrap()["instances"].as_array().unwrap().clone()
}

/// Prefab id (`e_00000001`) to live entity of an instance.
fn members(instance: &J) -> BTreeMap<String, String> {
    instance["members"].as_array().unwrap().iter().map(|m| (m["source_guid"].as_str().unwrap().to_string(), m["entity"].as_str().unwrap().to_string())).collect()
}

fn read(h: &mut App, guid: &str, component: &str, path: &str) -> Value {
    h.state_mut().editor.field_of(&target(guid), component, path).unwrap_or_else(|e| panic!("{guid} {component}.{path}: {e}"))
}

fn set(h: &mut App, guid: &str, component: &str, path: &str, value: Value) {
    assert!(h.state_mut().editor.set_entity_field(&target(guid), component, path, value), "{:?}", h.state().editor.status());
    settle(h);
}

fn override_count(h: &mut App, guid: &str) -> u64 {
    let entity = h.state_mut().editor.host_call("prefab_instance.entity", json!({"entity": guid})).unwrap();
    entity["instance_override_count"].as_u64().unwrap_or(0)
}

fn scene_text(h: &mut App) -> String {
    h.state_mut().editor.host_call("scene.save", json!({})).unwrap()["text"].as_str().unwrap().to_string()
}

/// Creates `motor.prefab.yaml` from the hub, propeller and pin, without linking them.
fn create_motor(h: &mut App, root: &std::path::Path) {
    assert!(h.state_mut().editor.select_named("motor_hub"));
    for name in ["motor_propeller", "motor_pin"] {
        let guid = h.state().editor.rows().iter().find(|r| r.name.as_deref() == Some(name)).and_then(|r| r.guid.clone()).unwrap();
        assert!(h.state_mut().editor.toggle_selection(Target::Guid(guid)));
    }
    settle(h);
    assert_eq!(h.state().editor.selected_guids().len(), 3);
    name_prefab(h, "motor.prefab.yaml");
    click(h, "Link the selection to the new prefab"); // off: the originals stay plain entities
    click(h, "Create prefab from selection");
    assert!(root.join("motor.prefab.yaml").is_file());
}

/// Two instances of the motor; returns their members (prefab id -> live id).
fn two_motors(root: &std::path::Path) -> (App, BTreeMap<String, String>, BTreeMap<String, String>) {
    let mut h = app(open(root, EditorGame::PhysGame));
    create_motor(&mut h, root);
    click(&mut h, "Instantiate prefab");
    click(&mut h, "Instantiate prefab");
    let instances = list(&mut h);
    assert_eq!(instances.len(), 2);
    let (a, b) = (members(&instances[0]), members(&instances[1]));
    (h, a, b)
}

fn physics_project() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    std::fs::write(root.path().join("level.scene.yaml"), JOINTS).unwrap();
    let path = root.path().to_path_buf();
    (root, path)
}

#[test]
fn create_refuses_outside_references_and_instances_get_fresh_remapped_ids() {
    let (_root, path) = physics_project();
    let mut h = app(open(&path, EditorGame::PhysGame));
    // The joint alone refers to two bodies outside the selection: refused, no file.
    assert!(h.state_mut().editor.select_named("motor_pin"));
    settle(&mut h);
    name_prefab(&mut h, "pin.prefab.yaml");
    click(&mut h, "Create prefab from selection");
    assert_eq!(h.query_all_by_label_contains("cannot create a prefab").count(), 1, "the refusal is shown in the panel");
    assert_eq!(h.query_all_by_label_contains("is outside the fragment selection").count(), 1);
    assert!(!path.join("pin.prefab.yaml").exists());

    create_motor(&mut h, &path);
    let text = std::fs::read_to_string(path.join("motor.prefab.yaml")).unwrap();
    assert!(text.contains("e_00000001") && text.contains("e_00000003") && !text.contains("e_00000009"), "fragment-local ids:\n{text}");
    let entities_before = h.state().editor.rows().len();
    let history_before = h.state().editor.history().entries.len();
    click(&mut h, "Instantiate prefab");
    assert_eq!(h.state().editor.rows().len(), entities_before + 3);
    assert_eq!(h.state().editor.history().entries.len(), history_before + 1, "one undo step");
    let instances = list(&mut h);
    let m = members(&instances[0]);
    assert_eq!(read(&mut h, &m["e_00000003"], JOINT, "a"), Value::EntityGuid(Some(m["e_00000001"].clone())));
    assert_eq!(read(&mut h, &m["e_00000003"], JOINT, "b"), Value::EntityGuid(Some(m["e_00000002"].clone())));
    assert!(m.values().all(|g| !["e_00000007", "e_00000008", "e_00000009"].contains(&g.as_str())));
    // The new instance is selected, and undo removes it in one step.
    assert!(h.state().editor.is_selected(&target(instances[0]["instance"].as_str().unwrap())));
    assert!(h.state_mut().editor.undo());
    settle(&mut h);
    assert_eq!(h.state().editor.rows().len(), entities_before);
    assert!(list(&mut h).is_empty());
    assert!(h.state_mut().editor.redo());
    settle(&mut h);
    assert_eq!(list(&mut h).len(), 1);
}

#[test]
fn overrides_are_marked_revert_and_apply_updates_followers() {
    let (_root, path) = physics_project();
    let (mut h, a, b) = two_motors(&path);
    let (prop_a, prop_b) = (a["e_00000002"].clone(), b["e_00000002"].clone());
    let original = read(&mut h, &prop_a, COLLIDER, "friction");
    select(&mut h, &prop_a);
    assert_eq!(marks(&h), 0, "no override, no mark");

    set(&mut h, &prop_a, COLLIDER, "friction", fixed(3));
    wait(&mut h, "the override mark", |h| marks(h) == 1);
    assert_eq!(override_count(&mut h, &prop_a), 1);

    // Revert field: one undo step, the prefab value is back and the mark is gone.
    let history = h.state().editor.history().entries.len();
    click(&mut h, "Revert");
    wait(&mut h, "the mark to clear", |h| marks(h) == 0);
    assert_eq!(read(&mut h, &prop_a, COLLIDER, "friction"), original);
    assert_eq!(h.state().editor.history().entries.len(), history + 1);
    assert!(h.state_mut().editor.undo());
    settle(&mut h);
    assert_eq!(read(&mut h, &prop_a, COLLIDER, "friction"), fixed(3));
    assert!(h.state_mut().editor.redo());
    settle(&mut h);

    // Revert all restores two overrides in one step.
    set(&mut h, &prop_a, COLLIDER, "friction", fixed(3));
    set(&mut h, &prop_a, BODY, "angle", fixed(1));
    wait(&mut h, "two overrides", |h| override_count(h, &prop_a) == 2);
    let history = h.state().editor.history().entries.len();
    click(&mut h, "Revert all overrides");
    assert_eq!(override_count(&mut h, &prop_a), 0);
    assert_eq!(h.state().editor.history().entries.len(), history + 1);
    assert!(h.state_mut().editor.undo());
    settle(&mut h);
    assert_eq!(override_count(&mut h, &prop_a), 2);

    // Apply to prefab: the file takes A's values; B follows, except its own override.
    set(&mut h, &prop_b, BODY, "angle", fixed(2));
    click(&mut h, "Apply to prefab");
    let file = std::fs::read_to_string(path.join("motor.prefab.yaml")).unwrap();
    assert!(file.contains("friction: 3"), "the file now holds A's value:\n{file}");
    assert_eq!(read(&mut h, &prop_b, COLLIDER, "friction"), fixed(3), "B follows");
    assert_eq!(read(&mut h, &prop_b, BODY, "angle"), fixed(2), "B keeps its own override");
    assert_eq!(override_count(&mut h, &prop_a), 0, "A matches the prefab again");
    assert_eq!(override_count(&mut h, &prop_b), 1);
}

#[test]
fn a_changed_prefab_file_updates_instances_and_reports_conflicts_without_a_dialog() {
    let (_root, path) = physics_project();
    let (mut h, a, b) = two_motors(&path);
    let (prop_a, prop_b) = (a["e_00000002"].clone(), b["e_00000002"].clone());
    let hub_b = b["e_00000001"].clone();
    set(&mut h, &prop_b, COLLIDER, "friction", fixed(9)); // B overrides what the file will change
    set(&mut h, &hub_b, BODY, "angle", fixed(1)); // B overrides something the file leaves alone
    select(&mut h, &prop_b);
    let history = h.state().editor.history().entries.len();

    // Edit the prefab file on disk: both colliders get another friction and one
    // new entity is added.
    let file = path.join("motor.prefab.yaml");
    let text = std::fs::read_to_string(&file).unwrap();
    let mut changed = text.replacen("friction: 0.4", "friction: 7.5", 3);
    assert_ne!(changed, text);
    changed = changed.replace("e_00000003:", "e_00000004:\n    name: extra\n    orr_physics::Body:\n      pos: [1, 1]\n      angle: 0\n      vel: [0, 0]\n      omega: 0\n      inv_mass: 1\n      inv_inertia: 1\n      linear_damping: 0\n      angular_damping: 0\n      kind: dynamic\n  e_00000003:");
    std::fs::write(&file, &changed).unwrap();
    wait(&mut h, "the instances to update", |h| h.query_all_by_label_contains("both changed it").count() == 1);
    assert_eq!(h.state().editor.history().entries.len(), history + 1, "one undo step for all instances");
    // A (no overrides) follows; B keeps its two overrides.
    assert_eq!(read(&mut h, &prop_a, COLLIDER, "friction"), Value::Fixed("7.5".parse().unwrap()));
    assert_eq!(read(&mut h, &prop_b, COLLIDER, "friction"), fixed(9));
    assert_eq!(read(&mut h, &hub_b, BODY, "angle"), fixed(1));
    assert_eq!(h.query_all_by_label_contains("friction: the prefab and the instance both changed it").count(), 1, "one conflict, in the panel");
    assert!(h.query_by_label_contains("changed on disk").is_some(), "the notice names the cause");
    let instances = list(&mut h);
    assert!(instances.iter().all(|i| i["members"].as_array().unwrap().len() == 4), "the new entity joined both instances");
    assert!(h.state().editor.rows().iter().filter(|r| r.name.as_deref() == Some("extra")).count() == 2);

    // Undo and redo of the whole update.
    assert!(h.state_mut().editor.undo());
    settle(&mut h);
    assert_eq!(h.state().editor.rows().iter().filter(|r| r.name.as_deref() == Some("extra")).count(), 0);
    assert_eq!(read(&mut h, &prop_a, COLLIDER, "friction"), Value::Fixed("0.4".parse().unwrap()));
    assert!(h.state_mut().editor.redo());
    settle(&mut h);
    assert_eq!(h.state().editor.rows().iter().filter(|r| r.name.as_deref() == Some("extra")).count(), 2);
}

#[test]
fn nesting_is_refused_and_a_member_offers_no_create_button() {
    let (_root, path) = physics_project();
    let (mut h, a, _b) = two_motors(&path);
    select(&mut h, &a["e_00000001"]);
    assert_eq!(h.query_all_by_label("Create prefab from selection").count(), 0, "a member shows the instance actions");
    let error = h.state_mut().editor.host_call("prefab_instance.create", json!({"selected": [a["e_00000001"]]})).unwrap_err();
    assert!(error.contains("nested prefabs are not supported"), "{error}");
    // The instance actions are there instead.
    assert_eq!(h.get_all_by_label("Detach instance").count(), 1);
}

#[test]
fn links_and_overrides_survive_save_and_reopen_and_detach_undoes() {
    let (_root, path) = physics_project();
    let (mut h, a, b) = two_motors(&path);
    let prop_b = b["e_00000002"].clone();
    set(&mut h, &prop_b, COLLIDER, "friction", fixed(5));
    select(&mut h, &a["e_00000001"]);
    click(&mut h, "Detach instance");
    assert_eq!(list(&mut h).len(), 1);
    assert!(h.state_mut().editor.undo());
    settle(&mut h);
    assert_eq!(list(&mut h).len(), 2);
    assert!(h.state_mut().editor.save());
    drop(h);

    let mut h = app(open(&path, EditorGame::PhysGame));
    assert!(scene_text(&mut h).contains("profile: generic-v1"));
    let instances = list(&mut h);
    assert_eq!(instances.len(), 2);
    assert_eq!(members(&instances[0]), a);
    assert_eq!(members(&instances[1]), b);
    assert_eq!(override_count(&mut h, &prop_b), 1);
    assert_eq!(read(&mut h, &prop_b, COLLIDER, "friction"), fixed(5));
    select(&mut h, &prop_b);
    wait(&mut h, "the reopened override mark", |h| marks(h) == 1);
}

#[test]
fn yard3d_prefab_workflow_with_overrides_apply_update_and_reopen() {
    let (root, editor) = project(YARD, EditorGame::Yard3D);
    let mut h = app(editor);
    // 3D selection is one entity; a multi-entity prefab is created through the host method.
    assert!(h.state_mut().editor.select_named("box_left"));
    settle(&mut h);
    name_prefab(&mut h, "box.prefab.yaml");
    click(&mut h, "Link the selection to the new prefab");
    click(&mut h, "Create prefab from selection");
    assert!(root.path().join("box.prefab.yaml").is_file());
    let pair = h.state_mut().editor.host_call("prefab_instance.create", json!({"selected": ["e_00000002", "e_00000003"]})).unwrap();
    assert_eq!(pair["guids"].as_object().unwrap().len(), 2);
    click(&mut h, "Instantiate prefab");
    click(&mut h, "Instantiate prefab");
    let instances = list(&mut h);
    assert_eq!(instances.len(), 2);
    let (a, b) = (members(&instances[0])["e_00000001"].clone(), members(&instances[1])["e_00000001"].clone());
    assert_eq!(h.state().editor.rows().len(), 5);

    select(&mut h, &a);
    let friction = |h: &mut App, g: &str| read(h, g, "orr_physics3d::Collider", "friction");
    let original = friction(&mut h, &a);
    set(&mut h, &a, "orr_physics3d::Collider", "friction", fixed(2));
    wait(&mut h, "the 3D override mark", |h| marks(h) == 1);
    click(&mut h, "Revert");
    assert_eq!(friction(&mut h, &a), original);
    set(&mut h, &a, "orr_physics3d::Collider", "friction", fixed(2));
    click(&mut h, "Apply to prefab");
    assert!(std::fs::read_to_string(root.path().join("box.prefab.yaml")).unwrap().contains("friction: 2"));
    assert_eq!(friction(&mut h, &b), fixed(2));
    assert_eq!(override_count(&mut h, &a), 0);

    set(&mut h, &b, "orr_physics3d::Collider", "friction", fixed(4));
    assert!(h.state_mut().editor.save());
    drop(h);
    let mut h = app(open(root.path(), EditorGame::Yard3D));
    assert_eq!(list(&mut h).len(), 2);
    assert_eq!(override_count(&mut h, &b), 1);
    assert_eq!(friction(&mut h, &b), fixed(4));
}

/// Renders the real editor on a software GPU and saves the inspector with its
/// override marks. Needs `ORR_PREFAB_SHOTS` (an output folder).
#[test]
#[ignore = "needs a GPU adapter and ORR_PREFAB_SHOTS=<folder>"]
fn screenshot_of_the_inspector_with_override_marks() {
    let dest = PathBuf::from(std::env::var_os("ORR_PREFAB_SHOTS").expect("ORR_PREFAB_SHOTS"));
    std::fs::create_dir_all(&dest).unwrap();
    let (_root, path) = physics_project();
    let editor = open(&path, EditorGame::PhysGame);
    let mut h: App = Harness::builder()
        .with_size([1500.0, 1100.0])
        .wgpu()
        .build_eframe(move |cc| EditorApp::new(editor, cc.wgpu_render_state.clone()));
    settle(&mut h);
    assert!(h.state().has_gpu(), "a GPU adapter is required");
    create_motor(&mut h, &path);
    click(&mut h, "Instantiate prefab");
    click(&mut h, "Instantiate prefab");
    let instances = list(&mut h);
    let (a, b) = (members(&instances[0]), members(&instances[1]));
    set(&mut h, &a["e_00000002"], BODY, "omega", fixed(2));
    set(&mut h, &a["e_00000002"], BODY, "angle", fixed(1));
    set(&mut h, &b["e_00000002"], COLLIDER, "friction", fixed(8));
    select(&mut h, &a["e_00000002"]);
    wait(&mut h, "marks", |h| marks(h) == 2);
    h.run_steps(4);
    let image = h.render().expect("GPU readback");
    image.save(dest.join("prefab_inspector_override_marks.png")).unwrap();
    // After "Apply to prefab" the marks are gone and the other instance followed.
    click(&mut h, "Apply to prefab");
    wait(&mut h, "marks to clear", |h| marks(h) == 0);
    h.run_steps(4);
    h.render().expect("GPU readback").save(dest.join("prefab_inspector_after_apply.png")).unwrap();
}
