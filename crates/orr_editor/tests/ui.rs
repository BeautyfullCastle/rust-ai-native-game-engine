//! egui_kittest harness tests of the app: real widgets, real input events,
//! no GPU (the viewport shows its notice; everything else works).

mod common;

use common::*;
use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use orr_edit::Target;
use orr_editor::app::{BottomTab, LBL_PAUSE, LBL_PLAY, LBL_STEP, LBL_STOP};
use orr_editor::editor::Mode;
use orr_editor::EditorApp;
use orr_reflect::{decimal, Value};

fn harness() -> Harness<'static, EditorApp> {
    Harness::builder().with_size([1500.0, 900.0]).build_eframe(|_cc| EditorApp::new(demo_editor(), None))
}

fn press(h: &Harness<'_, EditorApp>, modifiers: Modifiers, key: Key) {
    h.key_press_modifiers(modifiers, key);
}

/// The text field that shows exactly `value`.
fn text_field<'a>(h: &'a Harness<'_, EditorApp>, value: &'a str) -> egui_kittest::Node<'a> {
    h.get_by(move |n| n.role() == Role::TextInput && n.value().as_deref() == Some(value))
}

/// The combo box that shows `value`.
fn combo<'a>(h: &'a Harness<'_, EditorApp>, value: &'a str) -> egui_kittest::Node<'a> {
    h.get_by(move |n| n.role() == Role::ComboBox && n.value().as_deref() == Some(value))
}

/// Types `text` into the text field showing `current`, then commits with Enter.
fn type_into(h: &mut Harness<'_, EditorApp>, current: &str, text: &str) {
    text_field(h, current).focus();
    h.run_steps(2);
    press(h, Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    text_field(h, current).type_text(text);
    h.run_steps(2);
    press(h, Modifiers::NONE, Key::Enter);
    h.run_steps(3);
}

fn pos_x_text(h: &Harness<'_, EditorApp>, name: &str) -> (Target, String) {
    let ed = &h.state().editor;
    let t = target_named(ed, name);
    let Value::Fixed(x) = ed.view().field(&t, BODY, "pos.x").unwrap() else { panic!("pos.x is fixed") };
    (t, decimal::fp_to_decimal(x))
}

#[test]
fn hierarchy_click_selects_an_entity() {
    let mut h = harness();
    h.run();
    assert!(h.state().editor.selection().is_none());
    h.get_by_label("body_05").click();
    h.run();
    let want = target_named(&h.state().editor, "body_05");
    assert_eq!(h.state().editor.selection(), Some(&want));
    // The inspector shows the components, generated from reflection.
    assert!(h.query_by_label("orr_physics::Body").is_some());
    assert!(h.query_by_label("orr_physics::Collider").is_some());
    // Clicking another entity changes the selection.
    h.get_by_label("floor").click();
    h.run();
    let want = target_named(&h.state().editor, "floor");
    assert_eq!(h.state().editor.selection(), Some(&want));
}

#[test]
fn fixed_field_edit_in_the_inspector_undo_redo_restore_exact_text() {
    let mut h = harness();
    h.run();
    h.get_by_label("body_05").click();
    h.run();
    let original = h.state().editor.doc().to_yaml();
    let checksum = h.state().editor.checksum();
    let (target, shown) = pos_x_text(&h, "body_05");

    type_into(&mut h, &shown, "3.25");
    let ed = &h.state().editor;
    assert_eq!(ed.view().field(&target, BODY, "pos.x").unwrap(), Value::Fixed(decimal::parse_fp("3.25").unwrap()));
    let edited = ed.doc().to_yaml();
    assert_ne!(edited, original);
    assert!(edited.contains("3.25"), "the scene text has the exact decimal");
    assert_eq!(ed.doc().history().len(), 1, "one typed value is one history entry");
    assert!(ed.is_dirty());

    // Ctrl+Z restores the exact text and checksum.
    press(&h, Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(h.state().editor.doc().to_yaml(), original);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert!(!h.state().editor.is_dirty());

    // Ctrl+Y and Ctrl+Shift+Z both redo.
    press(&h, Modifiers::COMMAND, Key::Y);
    h.run();
    assert_eq!(h.state().editor.doc().to_yaml(), edited);
    press(&h, Modifiers::COMMAND, Key::Z);
    h.run();
    press(&h, Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    h.run();
    assert_eq!(h.state().editor.doc().to_yaml(), edited);
}

#[test]
fn text_that_is_not_a_plain_decimal_is_refused_with_a_message() {
    let mut h = harness();
    h.run();
    h.get_by_label("body_05").click();
    h.run();
    let original = h.state().editor.doc().to_yaml();
    let (_, shown) = pos_x_text(&h, "body_05");
    type_into(&mut h, &shown, "abc");
    assert_eq!(h.state().editor.doc().to_yaml(), original);
    assert!(h.state().editor.status().is_some_and(|m| m.error), "{:?}", h.state().editor.status());
    // Out of range: the document refuses.
    let (_, shown) = pos_x_text(&h, "body_05");
    type_into(&mut h, &shown, "99999");
    assert_eq!(h.state().editor.doc().to_yaml(), original);
    assert!(h.state().editor.status().is_some_and(|m| m.error));
}

#[test]
fn a_drag_on_the_scrub_handle_is_one_transaction() {
    let mut h = harness();
    h.run();
    h.get_by_label("body_05").click();
    h.run();
    let (target, before) = pos_x_text(&h, "body_05");
    let handle = h.get_all_by_label("\u{2194}").next().expect("a scrub handle").rect().center();
    h.event(Event::PointerMoved(handle));
    h.step();
    h.event(Event::PointerButton { pos: handle, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    for i in 1..=8 {
        h.event(Event::PointerMoved(handle + egui::vec2(i as f32 * 6.0, 0.0)));
        h.step();
    }
    h.event(Event::PointerButton { pos: handle + egui::vec2(48.0, 0.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    let (_, after) = pos_x_text(&h, "body_05");
    assert_ne!(before, after, "dragging changed the value");
    let ed = &h.state().editor;
    assert_eq!(ed.doc().history().len(), 1, "press to release is one history entry: {:?}", ed.doc().history());
    assert!(!ed.doc().in_tx());
    // The dragged value is a plain decimal, the same text a person could type.
    assert!(decimal::parse_fp(&after).is_ok());
    let _ = target;
    press(&h, Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(pos_x_text(&h, "body_05").1, before);
}

#[test]
fn play_pause_rewind_with_the_buttons() {
    let mut h = harness();
    h.run();
    h.get_by_label(LBL_STEP).click();
    h.run_steps(3);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    assert_eq!(h.state().editor.timeline().unwrap().tick, 1, "Step from edit mode starts play and runs one tick");
    h.get_by_label(LBL_PLAY).click();
    h.run_steps(4);
    let tl = h.state().editor.timeline().unwrap();
    assert!(tl.playing && tl.tick > 1, "the clock ran ticks: {tl:?}");
    h.get_by_label(LBL_PAUSE).click();
    h.run_steps(2);
    let tl = h.state().editor.timeline().unwrap();
    assert!(!tl.playing);
    let paused_at = tl.tick;
    h.run_steps(3);
    assert_eq!(h.state().editor.timeline().unwrap().tick, paused_at, "paused means no more ticks");
    // Rewind button.
    h.get_by_label("\u{23EE}").click();
    h.run_steps(2);
    assert_eq!(h.state().editor.timeline().unwrap().tick, 0);
    // Stop returns to edit mode.
    h.get_by_label(LBL_STOP).click();
    h.run_steps(2);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
}

#[test]
fn stop_leaves_the_document_unchanged() {
    let mut h = harness();
    h.run();
    let yaml = h.state().editor.doc().to_yaml();
    let checksum = h.state().editor.checksum();
    h.state_mut().editor.step(45);
    h.run_steps(2);
    assert_ne!(h.state().editor.checksum(), checksum);
    h.get_by_label(LBL_STOP).click();
    h.run_steps(2);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    assert_eq!(h.state().editor.doc().to_yaml(), yaml);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert!(!h.state().editor.is_dirty());
}

#[test]
fn play_mode_inspector_edit_is_recorded_and_replays() {
    let mut h = harness();
    h.run();
    h.get_by_label("body_05").click();
    h.run();
    h.state_mut().editor.step(20);
    h.run_steps(2);
    let (target, shown) = pos_x_text(&h, "body_05");
    type_into(&mut h, &shown, "4.5");
    let ed = &h.state().editor;
    assert_eq!(ed.view().field(&target, BODY, "pos.x").unwrap(), Value::Fixed(decimal::parse_fp("4.5").unwrap()));
    assert!(ed.timeline().unwrap().pending_edits >= 1, "recorded as a debug command");
    assert!(!ed.is_dirty(), "a play edit is not a document edit");
    h.state_mut().editor.step(40);
    let head = h.state().editor.timeline().unwrap().tick;
    let live = h.state().editor.checksum();
    let stopped = h.state_mut().editor.stop().cloned().unwrap();
    assert_eq!((stopped.tick, stopped.checksum), (head, live));
    let (last, replayed) = replay_end(&stopped.replay);
    assert_eq!((last, replayed), (head, live), "open the recording as a viewer: same final checksum");
}

#[test]
fn scrubbing_the_timeline_slider_seeks() {
    let mut h = harness();
    h.run();
    h.state_mut().editor.step(60);
    h.run_steps(3);
    let first = checksums(&h.state().editor, 0, 60);
    // Two sliders: speed, then the tick scrubber.
    h.get_all_by_role(Role::Slider).last().expect("the tick slider").focus();
    h.run_steps(2);
    for _ in 0..10 {
        press(&h, Modifiers::NONE, Key::ArrowLeft);
        h.run_steps(2);
    }
    let tl = h.state().editor.timeline().unwrap();
    assert!(tl.tick < 60, "the slider moved the head back: {tl:?}");
    assert_eq!(h.state().editor.checksum(), first[tl.tick as usize]);
}

#[test]
fn history_tab_lists_edits_with_origin() {
    let mut h = harness();
    h.run();
    h.state_mut().editor.select_named("body_05");
    h.state_mut().editor.set_field(&orr_editor::Owner::Component(BODY.into()), "pos.x", fixed(2));
    h.state_mut().ui.bottom_tab = BottomTab::History;
    h.run_steps(2);
    assert!(h.query_by_label_contains("[user]").is_some());
    assert!(h.query_by_label_contains("set orr_physics::Body.pos.x").is_some());
}

#[test]
fn menu_open_save_as_and_dirty_title() {
    let mut h = harness();
    h.run();
    h.state_mut().editor.select_named("body_05");
    h.state_mut().editor.set_field(&orr_editor::Owner::Component(BODY.into()), "pos.x", fixed(2));
    h.run_steps(2);
    assert!(h.state().editor.title().starts_with('*'));
    let path = temp_path("ui_save.scene.yaml");
    h.get_by_label("File").click();
    h.run_steps(2);
    h.get_by_label("Save As\u{2026}").click();
    h.run_steps(3);
    // The path prompt is open: type the path and press OK.
    assert!(h.state().ui.dialog.is_some());
    h.state_mut().ui.dialog.as_mut().unwrap().text = path.display().to_string();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(path.exists());
    assert!(!h.state().editor.is_dirty());
    assert!(h.state().editor.title().contains("ui_save.scene.yaml"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn hierarchy_spawn_and_delete_buttons() {
    let mut h = harness();
    h.run();
    let n = h.state().editor.view().entities().len();
    h.get_by_label("+ Body").click();
    h.run_steps(3);
    assert_eq!(h.state().editor.view().entities().len(), n + 1);
    assert!(h.state().editor.selection().is_some());
    h.get_by_label("Delete").click();
    h.run_steps(3);
    assert_eq!(h.state().editor.view().entities().len(), n);
}

#[test]
fn viewport_click_selects_the_body_under_the_pointer_and_drag_moves_it() {
    let mut h = harness();
    h.run();
    let rect = h.state().ui.viewport_rect.expect("the viewport was laid out");
    let vp = h.state().ui.viewport_px;
    let ed = &h.state().editor;
    let t = target_named(ed, "body_05");
    let Value::Vec2(p) = ed.view().field(&t, BODY, "pos").unwrap() else { panic!() };
    let world = [orr_view::fp_to_f32(p.x), orr_view::fp_to_f32(p.y)];
    let s = ed.camera.world_to_screen(world, vp);
    let at = Pos2::new(rect.min.x + s[0], rect.min.y + s[1]);
    // Click selects.
    h.event(Event::PointerMoved(at));
    h.step();
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert_eq!(h.state().editor.selection(), Some(&t));
    // Drag moves it (one transaction) and undo brings it back.
    let before = h.state().editor.doc().to_yaml();
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    for i in 1..=6 {
        h.event(Event::PointerMoved(at + egui::vec2(i as f32 * 10.0, 5.0 * i as f32)));
        h.step();
    }
    h.event(Event::PointerButton { pos: at + egui::vec2(60.0, 30.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    let ed = &h.state().editor;
    let Value::Vec2(q) = ed.view().field(&t, BODY, "pos").unwrap() else { panic!() };
    assert!(q.x > p.x && q.y < p.y, "moved right and down: {p:?} -> {q:?}");
    assert_eq!(ed.doc().history().len(), 1, "one drag is one transaction");
    assert!(ed.is_dirty());
    press(&h, Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(h.state().editor.doc().to_yaml(), before);
}

#[test]
fn viewport_drag_in_play_mode_records_a_debug_command() {
    let mut h = harness();
    h.run();
    h.state_mut().editor.step(10);
    h.run_steps(2);
    let rect = h.state().ui.viewport_rect.unwrap();
    let vp = h.state().ui.viewport_px;
    let t = target_named(&h.state().editor, "body_05");
    let Value::Vec2(p) = h.state().editor.view().field(&t, BODY, "pos").unwrap() else { panic!() };
    let s = h.state().editor.camera.world_to_screen([orr_view::fp_to_f32(p.x), orr_view::fp_to_f32(p.y)], vp);
    let at = Pos2::new(rect.min.x + s[0], rect.min.y + s[1]);
    h.event(Event::PointerMoved(at));
    h.step();
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    for i in 1..=4 {
        h.event(Event::PointerMoved(at + egui::vec2(i as f32 * 8.0, 0.0)));
        h.step();
    }
    h.event(Event::PointerButton { pos: at + egui::vec2(32.0, 0.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    let ed = &h.state().editor;
    assert!(ed.timeline().unwrap().pending_edits >= 1);
    assert!(!ed.is_dirty());
    assert!(ed.doc().history().is_empty());
}

#[test]
fn camera_zoom_and_pan() {
    let mut h = harness();
    h.run();
    let rect = h.state().ui.viewport_rect.unwrap();
    let before = h.state().editor.camera;
    let at = rect.center();
    h.event(Event::PointerMoved(at));
    h.step();
    h.event(Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, 120.0), modifiers: Modifiers::NONE, phase: egui::TouchPhase::Move });
    h.run_steps(3);
    assert!(h.state().editor.camera.half_extent < before.half_extent, "wheel up zooms in");
    let zoomed = h.state().editor.camera;
    h.event(Event::PointerButton { pos: at, button: PointerButton::Middle, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    for i in 1..=5 {
        h.event(Event::PointerMoved(at + egui::vec2(i as f32 * 10.0, 0.0)));
        h.step();
    }
    h.event(Event::PointerButton { pos: at + egui::vec2(50.0, 0.0), button: PointerButton::Middle, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(h.state().editor.camera.center[0] < zoomed.center[0], "dragging right moves the camera left");
}

#[test]
fn inspector_flags_enum_and_component_buttons() {
    let mut h = harness();
    h.run();
    h.get_by_label("body_05").click();
    h.run();
    let t = target_named(&h.state().editor, "body_05");
    let collider = "orr_physics::Collider";
    // Flags: one checkbox per bit.
    h.get_by_label("sensor").click();
    h.run_steps(3);
    let flags = h.state().editor.view().field(&t, collider, "flags").unwrap();
    assert_eq!(flags, Value::Flags(vec!["sensor".to_string()]));
    // Enum: a combo box.
    combo(&h, "dynamic").click();
    h.run_steps(3);
    h.get_by_label("kinematic").click();
    h.run_steps(3);
    assert_eq!(h.state().editor.view().field(&t, BODY, "kind").unwrap(), Value::Enum("kinematic".to_string()));
    // Tagged value: switch the shape kind.
    combo(&h, "polygon").click();
    h.run_steps(3);
    h.get_by_label("circle").click();
    h.run_steps(3);
    let shape = h.state().editor.view().field(&t, collider, "shape").unwrap();
    assert!(matches!(&shape, Value::Variant(k, _) if k == "circle"), "{shape:?}");
    // Remove and add a component.
    let removes: Vec<_> = h.get_all_by_label("Remove component").collect();
    removes.last().expect("a remove button").click();
    h.run_steps(3);
    assert!(h.state().editor.view().field(&t, collider, "flags").is_err(), "the collider is gone");
    combo(&h, "+ Add component").click();
    h.run_steps(3);
    h.get_by_label(collider).click();
    h.run_steps(3);
    assert!(h.state().editor.view().field(&t, collider, "flags").is_ok(), "the collider is back");
    // Every step was an undoable edit by the user.
    let hist = h.state().editor.doc().history();
    assert!(hist.len() >= 5 && hist.iter().all(|e| e.origin == orr_edit::Origin::User), "{hist:?}");
}
