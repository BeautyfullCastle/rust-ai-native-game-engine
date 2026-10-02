//! egui_kittest harness tests of the app: real widgets, real input events,
//! a real host thread behind the editor, no GPU (the viewport shows its
//! notice; everything else works).
#![allow(clippy::disallowed_types)]

mod common;

use std::time::{Duration, Instant};

use common::*;
use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use orr_editor::app::{BottomTab, LBL_PAUSE, LBL_PLAY, LBL_STEP, LBL_STOP};
use orr_editor::editor::Mode;
use orr_editor::{EditorApp, Target};
use orr_reflect::{decimal, Value};

fn harness() -> Harness<'static, EditorApp> {
    Harness::builder().with_size([1500.0, 900.0]).build_eframe(|_cc| EditorApp::new(demo_editor(), None))
}

/// Brings the editor up to date with its host (what the window does over a few frames) and draws.
fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..3 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
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
    settle(h);
}

fn pos_x(h: &mut Harness<'_, EditorApp>, name: &str) -> (Target, Value) {
    let ed = &mut h.state_mut().editor;
    let t = target_named(ed, name);
    let v = field(ed, &t, BODY, "pos.x");
    (t, v)
}

fn pos_x_text(h: &mut Harness<'_, EditorApp>, name: &str) -> (Target, String) {
    let (t, v) = pos_x(h, name);
    let Value::Fixed(x) = v else { panic!("pos.x is fixed") };
    (t, decimal::fp_to_decimal(x))
}

fn yaml(h: &mut Harness<'_, EditorApp>) -> String {
    scene_text(&mut h.state_mut().editor)
}

/// Runs the window until `pred` holds (the host runs on its own clock).
fn wait_for(h: &mut Harness<'_, EditorApp>, what: &str, pred: impl Fn(&EditorApp) -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while !pred(h.state()) {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn hierarchy_click_selects_an_entity() {
    let mut h = harness();
    h.run_steps(3);
    assert!(h.state().editor.selection().is_none());
    h.get_by_label("body_05").click();
    settle(&mut h);
    let want = target_named(&h.state().editor, "body_05");
    assert_eq!(h.state().editor.selection(), Some(&want));
    // The inspector shows the components, generated from reflection.
    assert!(h.query_by_label("orr_physics::Body").is_some());
    assert!(h.query_by_label("orr_physics::Collider").is_some());
    // Clicking another entity changes the selection.
    h.get_by_label("floor").click();
    settle(&mut h);
    let want = target_named(&h.state().editor, "floor");
    assert_eq!(h.state().editor.selection(), Some(&want));
}

#[test]
fn fixed_field_edit_in_the_inspector_undo_redo_restore_exact_text() {
    let mut h = harness();
    h.run_steps(3);
    h.get_by_label("body_05").click();
    settle(&mut h);
    let original = yaml(&mut h);
    let checksum = h.state().editor.checksum();
    let (target, shown) = pos_x_text(&mut h, "body_05");

    type_into(&mut h, &shown, "3.25");
    assert_eq!(pos_x(&mut h, "body_05").1, Value::Fixed(decimal::parse_fp("3.25").unwrap()));
    let edited = yaml(&mut h);
    assert_ne!(edited, original);
    assert!(edited.contains("3.25"), "the scene text has the exact decimal");
    assert_eq!(h.state().editor.history().entries.len(), 1, "one typed value is one history entry");
    assert!(h.state().editor.is_dirty());
    let _ = target;

    // Ctrl+Z restores the exact text and checksum.
    press(&h, Modifiers::COMMAND, Key::Z);
    settle(&mut h);
    assert_eq!(yaml(&mut h), original);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert!(!h.state().editor.is_dirty());

    // Ctrl+Y and Ctrl+Shift+Z both redo.
    press(&h, Modifiers::COMMAND, Key::Y);
    settle(&mut h);
    assert_eq!(yaml(&mut h), edited);
    press(&h, Modifiers::COMMAND, Key::Z);
    settle(&mut h);
    press(&h, Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    settle(&mut h);
    assert_eq!(yaml(&mut h), edited);
}

#[test]
fn text_that_is_not_a_plain_decimal_is_refused_with_a_message() {
    let mut h = harness();
    h.run_steps(3);
    h.get_by_label("body_05").click();
    settle(&mut h);
    let original = yaml(&mut h);
    let (_, shown) = pos_x_text(&mut h, "body_05");
    type_into(&mut h, &shown, "abc");
    assert_eq!(yaml(&mut h), original);
    assert!(h.state().editor.status().is_some_and(|m| m.error), "{:?}", h.state().editor.status());
    // Out of range: the document refuses.
    let (_, shown) = pos_x_text(&mut h, "body_05");
    type_into(&mut h, &shown, "99999");
    assert_eq!(yaml(&mut h), original);
    assert!(h.state().editor.status().is_some_and(|m| m.error));
}

#[test]
fn a_drag_on_the_scrub_handle_is_one_transaction() {
    let mut h = harness();
    h.run_steps(3);
    h.get_by_label("body_05").click();
    settle(&mut h);
    let (_, before) = pos_x_text(&mut h, "body_05");
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
    settle(&mut h);
    let (_, after) = pos_x_text(&mut h, "body_05");
    assert_ne!(before, after, "dragging changed the value");
    let ed = &h.state().editor;
    assert_eq!(ed.history().entries.len(), 1, "press to release is one history entry: {:?}", ed.history());
    assert!(!ed.history().in_tx);
    // The dragged value is a plain decimal, the same text a person could type.
    assert!(decimal::parse_fp(&after).is_ok());
    press(&h, Modifiers::COMMAND, Key::Z);
    settle(&mut h);
    assert_eq!(pos_x_text(&mut h, "body_05").1, before);
}

#[test]
fn play_pause_rewind_with_the_buttons() {
    let mut h = harness();
    h.run_steps(3);
    h.get_by_label(LBL_STEP).click();
    settle(&mut h);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    assert_eq!(h.state().editor.timeline().unwrap().tick, 1, "Step from edit mode starts play and runs one tick");
    h.get_by_label(LBL_PLAY).click();
    wait_for(&mut h, "the clock to run ticks", |a| a.editor.timeline().is_some_and(|t| t.playing && t.tick > 3));
    h.get_by_label(LBL_PAUSE).click();
    settle(&mut h);
    let tl = h.state().editor.timeline().unwrap();
    assert!(!tl.playing);
    let paused_at = tl.tick;
    std::thread::sleep(Duration::from_millis(100));
    settle(&mut h);
    assert_eq!(h.state().editor.timeline().unwrap().tick, paused_at, "paused means no more ticks");
    // Rewind button.
    h.get_by_label("\u{23EE}").click();
    settle(&mut h);
    assert_eq!(h.state().editor.timeline().unwrap().tick, 0);
    // Stop returns to edit mode.
    h.get_by_label(LBL_STOP).click();
    settle(&mut h);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
}

#[test]
fn stop_leaves_the_document_unchanged() {
    let mut h = harness();
    h.run_steps(3);
    let original = yaml(&mut h);
    let checksum = h.state().editor.checksum();
    h.state_mut().editor.step(45);
    settle(&mut h);
    assert_ne!(h.state().editor.checksum(), checksum);
    h.get_by_label(LBL_STOP).click();
    settle(&mut h);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    assert_eq!(yaml(&mut h), original);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert!(!h.state().editor.is_dirty());
}

#[test]
fn play_mode_inspector_edit_is_recorded_and_replays() {
    let mut h = harness();
    h.run_steps(3);
    h.get_by_label("body_05").click();
    settle(&mut h);
    h.state_mut().editor.step(20);
    settle(&mut h);
    let (_, shown) = pos_x_text(&mut h, "body_05");
    type_into(&mut h, &shown, "4.5");
    assert_eq!(pos_x(&mut h, "body_05").1, Value::Fixed(decimal::parse_fp("4.5").unwrap()));
    let ed = &h.state().editor;
    assert!(ed.timeline().unwrap().pending_edits >= 1, "recorded as a debug command");
    assert!(!ed.is_dirty(), "a play edit is not a document edit");
    h.state_mut().editor.step(40);
    settle(&mut h);
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
    h.run_steps(3);
    h.state_mut().editor.step(60);
    settle(&mut h);
    let first = checksums(&mut h.state_mut().editor, 0, 60);
    // Two sliders: speed, then the tick scrubber.
    h.get_all_by_role(Role::Slider).last().expect("the tick slider").focus();
    h.run_steps(2);
    for _ in 0..10 {
        press(&h, Modifiers::NONE, Key::ArrowLeft);
        h.run_steps(2);
    }
    settle(&mut h);
    let tl = h.state().editor.timeline().unwrap();
    assert!(tl.tick < 60, "the slider moved the head back: {tl:?}");
    assert_eq!(h.state().editor.checksum(), first[tl.tick as usize]);
}

#[test]
fn history_tab_lists_edits_with_origin() {
    let mut h = harness();
    h.run_steps(3);
    h.state_mut().editor.select_named("body_05");
    h.state_mut().editor.set_field(&orr_editor::Owner::Component(BODY.into()), "pos.x", fixed(2));
    h.state_mut().ui.bottom_tab = BottomTab::History;
    settle(&mut h);
    assert!(h.query_by_label_contains("[user]").is_some());
    assert!(h.query_by_label_contains("set orr_physics::Body.pos.x").is_some());
}

#[test]
fn menu_open_save_as_and_dirty_title() {
    let mut h = harness();
    h.run_steps(3);
    h.state_mut().editor.select_named("body_05");
    h.state_mut().editor.set_field(&orr_editor::Owner::Component(BODY.into()), "pos.x", fixed(2));
    settle(&mut h);
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
    settle(&mut h);
    assert!(path.exists());
    assert!(!h.state().editor.is_dirty());
    assert!(h.state().editor.title().contains("ui_save.scene.yaml"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn hierarchy_spawn_and_delete_buttons() {
    let mut h = harness();
    h.run_steps(3);
    let n = h.state().editor.rows().len();
    h.get_by_label("+ Body").click();
    settle(&mut h);
    assert_eq!(h.state().editor.rows().len(), n + 1);
    assert!(h.state().editor.selection().is_some());
    h.get_by_label("Delete").click();
    settle(&mut h);
    assert_eq!(h.state().editor.rows().len(), n);
}

#[test]
fn viewport_click_selects_the_body_under_the_pointer_and_drag_moves_it() {
    let mut h = harness();
    h.run_steps(3);
    settle(&mut h);
    let rect = h.state().ui.viewport_rect.expect("the viewport was laid out");
    let vp = h.state().ui.viewport_px;
    let t = target_named(&h.state().editor, "body_05");
    let p = xy(&field(&mut h.state_mut().editor, &t, BODY, "pos"));
    let s = h.state().editor.camera.world_to_screen(p, vp);
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
    let before = yaml(&mut h);
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    for i in 1..=6 {
        h.event(Event::PointerMoved(at + egui::vec2(i as f32 * 10.0, 5.0 * i as f32)));
        h.step();
    }
    h.event(Event::PointerButton { pos: at + egui::vec2(60.0, 30.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    settle(&mut h);
    let q = xy(&field(&mut h.state_mut().editor, &t, BODY, "pos"));
    assert!(q[0] > p[0] && q[1] < p[1], "moved right and down: {p:?} -> {q:?}");
    let ed = &h.state().editor;
    assert_eq!(ed.history().entries.len(), 1, "one drag is one transaction");
    assert!(ed.is_dirty());
    press(&h, Modifiers::COMMAND, Key::Z);
    settle(&mut h);
    assert_eq!(yaml(&mut h), before);
}

#[test]
fn viewport_drag_in_play_mode_records_a_debug_command() {
    let mut h = harness();
    h.run_steps(3);
    h.state_mut().editor.step(10);
    settle(&mut h);
    let rect = h.state().ui.viewport_rect.unwrap();
    let vp = h.state().ui.viewport_px;
    let t = target_named(&h.state().editor, "body_05");
    let p = xy(&field(&mut h.state_mut().editor, &t, BODY, "pos"));
    let s = h.state().editor.camera.world_to_screen(p, vp);
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
    settle(&mut h);
    let ed = &h.state().editor;
    assert!(ed.timeline().unwrap().pending_edits >= 1);
    assert!(!ed.is_dirty());
    assert!(ed.history().entries.is_empty());
}

#[test]
fn camera_zoom_and_pan() {
    let mut h = harness();
    h.run_steps(3);
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
    h.run_steps(3);
    h.get_by_label("body_05").click();
    settle(&mut h);
    let t = target_named(&h.state().editor, "body_05");
    // Flags: one checkbox per bit.
    h.get_by_label("sensor").click();
    settle(&mut h);
    let flags = field(&mut h.state_mut().editor, &t, COLLIDER, "flags");
    assert_eq!(flags, Value::Flags(vec!["sensor".to_string()]));
    // Enum: a combo box.
    combo(&h, "dynamic").click();
    settle(&mut h);
    h.get_by_label("kinematic").click();
    settle(&mut h);
    assert_eq!(field(&mut h.state_mut().editor, &t, BODY, "kind"), Value::Enum("kinematic".to_string()));
    // Tagged value: switch the shape kind.
    combo(&h, "polygon").click();
    settle(&mut h);
    h.get_by_label("circle").click();
    settle(&mut h);
    let shape = field(&mut h.state_mut().editor, &t, COLLIDER, "shape");
    assert!(matches!(&shape, Value::Variant(k, _) if k == "circle"), "{shape:?}");
    // Remove and add a component.
    let removes: Vec<_> = h.get_all_by_label("Remove component").collect();
    removes.last().expect("a remove button").click();
    settle(&mut h);
    assert!(h.state_mut().editor.field_of(&t, COLLIDER, "flags").is_err(), "the collider is gone");
    combo(&h, "+ Add component").click();
    settle(&mut h);
    h.get_by_label(COLLIDER).click();
    settle(&mut h);
    assert!(h.state_mut().editor.field_of(&t, COLLIDER, "flags").is_ok(), "the collider is back");
    // Every step was an undoable edit by the user.
    let hist = h.state().editor.history().entries.clone();
    assert!(hist.len() >= 5 && hist.iter().all(|e| e.origin == "user"), "{hist:?}");
}

#[test]
fn escape_during_scrub_rolls_back_and_next_drag_is_a_fresh_undo_step() {
    let mut h = harness();
    h.run_steps(3);
    h.get_by_label("body_05").click();
    settle(&mut h);
    let original = yaml(&mut h);
    for cancel in [true, false] {
        let handle = h.get_all_by_label("\u{2194}").next().unwrap().rect().center();
        h.event(Event::PointerMoved(handle));
        h.step();
        h.event(Event::PointerButton { pos: handle, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.step();
        for i in 1..=3 {
            h.event(Event::PointerMoved(handle + egui::vec2(i as f32 * 6.0, 0.0)));
            h.step();
        }
        if cancel {
            press(&h, Modifiers::NONE, Key::Escape);
            h.step();
            // Continuing the pointer after Escape must not issue an ordinary
            // standalone field edit when the cancelled transaction drains.
            h.event(Event::PointerMoved(handle + egui::vec2(30.0, 0.0)));
            h.run_steps(3);
        }
        h.event(Event::PointerButton { pos: handle + egui::vec2(30.0, 0.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(2);
        settle(&mut h);
        if cancel {
            assert_eq!(yaml(&mut h), original);
            assert!(h.state().editor.history().entries.is_empty());
        } else {
            assert_ne!(yaml(&mut h), original);
            assert_eq!(h.state().editor.history().entries.len(), 1);
        }
        assert!(!h.state().editor.in_gesture());
    }
}

#[test]
fn focus_loss_without_pointer_release_cancels_and_rearms_the_next_drag() {
    let mut h = harness();
    h.run_steps(3);
    h.get_by_label("body_05").click();
    settle(&mut h);
    let original = yaml(&mut h);
    let handle = h.get_all_by_label("\u{2194}").next().unwrap().rect().center();
    h.event(Event::PointerMoved(handle));
    h.step();
    h.event(Event::PointerButton { pos: handle, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerMoved(handle + egui::vec2(18.0, 0.0)));
    h.step();
    h.input_mut().focused = false;
    h.event(Event::WindowFocused(false));
    h.step();
    // The release happened outside the app; deliberately never send it.
    h.input_mut().focused = true;
    h.event(Event::WindowFocused(true));
    h.event(Event::PointerMoved(handle + egui::vec2(24.0, 0.0)));
    h.step();
    settle(&mut h);
    assert_eq!(yaml(&mut h), original);
    assert!(!h.state().editor.in_gesture());
    assert!(h.state().editor.history().entries.is_empty());

    let handle = h.get_all_by_label("\u{2194}").next().unwrap().rect().center();
    h.event(Event::PointerMoved(handle));
    h.event(Event::PointerButton { pos: handle, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerMoved(handle + egui::vec2(18.0, 0.0)));
    h.step();
    h.event(Event::PointerButton { pos: handle + egui::vec2(18.0, 0.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.step();
    settle(&mut h);
    assert_ne!(yaml(&mut h), original);
    assert_eq!(h.state().editor.history().entries.len(), 1);
    assert!(!h.state().editor.in_gesture());
}
