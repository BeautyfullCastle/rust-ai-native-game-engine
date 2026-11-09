//! Headless tests of the production Room widgets; no native window/GPU claim.
#![cfg(feature = "room-input-map-editor")]
#[path = "../src/room_input_map_authoring.rs"]
mod room_input_map_authoring;
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_input::{ActionMap, Key};
use orr_sample::room_controls::RoomControls;
use room_input_map_authoring::App;
use std::path::Path;
fn harness() -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size([720.0, 650.0])
        .with_max_steps(120)
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(|_| App::default());
    settle(&mut h, "initial layout");
    h
}

fn settle(h: &mut Harness<'_, App>, context: &str) {
    h.try_run()
        .unwrap_or_else(|error| panic!("layout did not settle after {context}: {error}"));
}
fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).scroll_to_me();
    settle(h, label);
    assert!(
        !h.get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .is_disabled(),
        "disabled {label}"
    );
    h.get_by_role_and_label(Role::Button, label).click();
    settle(h, label);
}
fn combo(h: &mut Harness<'_, App>, label: &str, choice: &str) {
    h.get_by_role_and_label(Role::ComboBox, label)
        .scroll_to_me();
    settle(h, label);
    h.get_by_role_and_label(Role::ComboBox, label).click();
    settle(h, label);
    h.get_by_role_and_label(Role::Button, choice).scroll_to_me();
    settle(h, choice);
    h.get_by_role_and_label(Role::Button, choice).click();
    settle(h, choice);
    assert_eq!(
        h.get_by_role_and_label(Role::ComboBox, label)
            .accesskit_node()
            .value()
            .as_deref(),
        Some(choice)
    );
}
fn path(h: &mut Harness<'_, App>, value: &Path) {
    h.get_by_role(Role::TextInput).scroll_to_me();
    settle(h, "path scroll");
    h.get_by_role(Role::TextInput).focus();
    settle(h, "path focus");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    settle(h, "select path");
    h.get_by_role(Role::TextInput)
        .type_text(value.to_str().unwrap());
    settle(h, "path input");
}

#[test]
fn production_apply_history_save_reopen_and_pending_cancellation() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("room.json");
    let mut h = harness();
    combo(&mut h, "Binding 1", "Key J");
    assert!(h.state().panel().has_pending_draft());
    for label in [
        "Undo map edit",
        "Redo map edit",
        "Use Room defaults",
        "Open map (undoable)",
    ] {
        assert!(h
            .get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .is_disabled());
    }
    click(&mut h, "Apply action");
    let accepted = h.state().panel().document().clone();
    click(&mut h, "Apply action");
    click(&mut h, "Undo map edit");
    assert_eq!(h.state().panel().document(), &RoomControls::default_map());
    assert!(!h.state().panel().can_undo());
    click(&mut h, "Redo map edit");
    click(&mut h, "Clear action draft");
    click(&mut h, "Apply action");
    assert!(h.state().panel().has_pending_draft());
    assert_eq!(h.state().panel().document(), &accepted);
    path(&mut h, &file);
    click(&mut h, "Save As new file");
    assert_eq!(ActionMap::load_file(&file).unwrap(), accepted);
    click(&mut h, "Save As new file");
    assert!(!h.state().panel().message().starts_with("Saved"));
    click(&mut h, "Discard draft");
    click(&mut h, "Use Room defaults");
    click(&mut h, "Open map (undoable)");
    assert_eq!(h.state().panel().document(), &accepted);
    click(&mut h, "Open map (undoable)");
    click(&mut h, "Undo map edit");
    assert_eq!(h.state().panel().document(), &RoomControls::default_map());
    click(&mut h, "Redo map edit");
    let controls = RoomControls::from_map(&ActionMap::load_file(&file).unwrap()).unwrap();
    assert_eq!(controls.input_from_keys([Key::J]).move_x, -1);
}
#[test]
fn production_duplicate_rejection_action_navigation_and_reserved_choices_absent() {
    let mut h = harness();
    combo(&mut h, "Binding 1", "Key D");
    click(&mut h, "Apply action");
    assert_eq!(h.state().panel().document(), &RoomControls::default_map());
    assert!(h
        .get_by_role_and_label(Role::ComboBox, "Action")
        .accesskit_node()
        .is_disabled());
    click(&mut h, "Discard draft");
    combo(&mut h, "Action", "interact");
    click(&mut h, "Add binding");
    click(&mut h, "Add binding");
    click(&mut h, "Apply action");
    assert!(h.state().panel().has_pending_draft());
    click(&mut h, "Remove binding 3");
    click(&mut h, "Apply action");
    assert!(!h.state().panel().has_pending_draft());
    h.get_by_role_and_label(Role::ComboBox, "Binding 1").click();
    settle(&mut h, "open dropdown");
    assert!(h.query_by_role_and_label(Role::Button, "Key R").is_none());
    assert!(h
        .query_by_role_and_label(Role::Button, "Key Escape")
        .is_none());
    h.key_press(egui::Key::Escape);
    settle(&mut h, "cancel dropdown");
    assert!(!h.state().panel().has_pending_draft());
}

#[test]
fn all_five_actions_persist_and_invalid_open_preserves_history() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("all.json");
    let invalid = dir.path().join("reserved.json");
    let mut h = harness();
    for (action, choice) in [
        ("move_left", "Key J"),
        ("move_right", "Key L"),
        ("move_forward", "Key I"),
        ("move_backward", "Key K"),
        ("interact", "Key O"),
    ] {
        combo(&mut h, "Action", action);
        click(&mut h, "Clear action draft");
        click(&mut h, "Add binding");
        combo(&mut h, "Binding 1", choice);
        click(&mut h, "Apply action");
        assert!(!h.state().panel().has_pending_draft());
    }
    path(&mut h, &file);
    click(&mut h, "Save As new file");
    let accepted = ActionMap::load_file(&file).unwrap();
    let controls = RoomControls::from_map(&accepted).unwrap();
    let input = controls.input_from_keys([Key::L, Key::I, Key::O]);
    assert_eq!((input.move_x, input.move_z), (1, -1));
    assert_ne!(input.buttons, 0);
    assert_eq!(
        controls
            .input_from_keys([Key::J, Key::L, Key::I, Key::K])
            .move_x,
        0
    );
    let mut bad = accepted.clone();
    bad.actions[0].bindings = vec![orr_input::Button::Keyboard { key: Key::R }];
    std::fs::write(&invalid, serde_json::to_vec(&bad).unwrap()).unwrap();
    path(&mut h, &invalid);
    click(&mut h, "Open map (undoable)");
    assert!(h.state().panel().message().contains("reserved"));
    assert_eq!(h.state().panel().document(), &accepted);
    click(&mut h, "Clear action draft");
    assert!(h
        .get_by_role_and_label(Role::Button, "Undo map edit")
        .accesskit_node()
        .is_disabled());
    click(&mut h, "Discard draft");
    click(&mut h, "Undo map edit");
    click(&mut h, "Redo map edit");
    assert_eq!(h.state().panel().document(), &accepted);
}
