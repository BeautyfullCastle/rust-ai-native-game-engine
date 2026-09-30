#![allow(clippy::disallowed_types)]
//! `Editor` state machine tests (no window, no GPU) against a real host
//! thread: edits, undo, play, rewind, and the M4 loop. The editor talks to
//! the host only through ERP and the bridge; these tests do too.

mod common;

use common::*;
use orr_bridge::ControlOp;
use orr_editor::editor::{fp_of_f64, Mode, Owner};
use orr_editor::{script, Target};
use orr_reflect::Value;
use serde_json::json;

fn body() -> Owner {
    Owner::Component(BODY.to_string())
}

#[test]
fn edits_are_undoable_and_restore_exact_yaml() {
    let mut ed = demo_editor();
    let original = scene_text(&mut ed);
    let checksum = ed.checksum();
    assert!(ed.select_named("body_05"));
    assert!(ed.set_field(&body(), "pos.x", fixed(3)));
    assert!(ed.is_dirty());
    assert!(ed.title().starts_with('*'));
    let edited = scene_text(&mut ed);
    assert_ne!(edited, original);
    assert!(ed.undo());
    assert_eq!(scene_text(&mut ed), original, "undo restores the exact text");
    ed.sync();
    assert_eq!(ed.checksum(), checksum, "and the frame the viewport got");
    assert!(!ed.is_dirty());
    assert!(ed.redo());
    assert_eq!(scene_text(&mut ed), edited);
    ed.sync();
    let h = &ed.history().entries;
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].origin, "user", "the person's edits carry the user origin");
}

#[test]
fn a_gesture_is_one_undo_step() {
    let mut ed = demo_editor();
    ed.select_named("body_05");
    ed.begin_edit("drag pos.x");
    for x in 1..=20 {
        ed.set_field(&body(), "pos.x", fixed(x));
    }
    ed.end_edit();
    ed.sync();
    assert_eq!(ed.history().entries.len(), 1);
    assert_eq!(ed.history().entries[0].label, "drag pos.x");
    ed.undo();
    ed.sync();
    assert!(ed.history().entries.iter().all(|h| h.undone));
    assert!(!ed.is_dirty());
}

#[test]
fn refused_edits_report_and_change_nothing() {
    let mut ed = demo_editor();
    ed.select_named("body_05");
    let before = scene_text(&mut ed);
    // Outside the range of Body.pos (-30000..=30000).
    assert!(!ed.set_field(&body(), "pos.x", fixed(99_999)));
    assert!(ed.status().is_some_and(|m| m.error), "{:?}", ed.status());
    assert_eq!(scene_text(&mut ed), before);
    ed.sync();
    assert!(!ed.is_dirty());
    // Undo with nothing to undo is a message, not a panic.
    assert!(!ed.undo());
}

#[test]
fn spawn_add_remove_delete() {
    let mut ed = demo_editor();
    let count = ed.rows().len();
    assert!(ed.spawn_body([1.0, 2.0]));
    ed.sync();
    assert_eq!(ed.rows().len(), count + 1);
    let sel = ed.selection().cloned().expect("spawned entity is selected");
    let pos = field(&mut ed, &sel, BODY, "pos");
    assert_eq!(pos, Value::Vec2(orr_fp::FPVec2::new(fp_of_f64(1.0).unwrap(), fp_of_f64(2.0).unwrap())));
    assert!(ed.remove_component(COLLIDER));
    assert!(ed.add_component(COLLIDER));
    assert!(ed.delete_selected());
    ed.sync();
    assert_eq!(ed.rows().len(), count);
    assert!(ed.selection().is_none());
    // One undo brings the deleted entity back.
    assert!(ed.undo());
    ed.sync();
    assert_eq!(ed.rows().len(), count + 1);
}

#[test]
fn delete_without_selection_is_an_error_message() {
    let mut ed = demo_editor();
    assert!(!ed.delete_selected());
    assert!(ed.status().is_some_and(|m| m.error));
}

#[test]
fn shape_kind_switch_uses_the_variant_default() {
    let mut ed = demo_editor();
    ed.select_named("body_05");
    assert!(ed.set_field(&Owner::Component(COLLIDER.into()), "shape.kind", Value::Enum("circle".into())));
    let t = ed.selection().cloned().unwrap();
    let shape = field(&mut ed, &t, COLLIDER, "shape");
    assert!(matches!(&shape, Value::Variant(k, _) if k == "circle"), "{shape:?}");
}

#[test]
fn save_and_open_round_trip() {
    let mut ed = demo_editor();
    ed.select_named("body_05");
    ed.set_field(&body(), "pos", vec2(2, 3));
    let path = temp_path("roundtrip.scene.yaml");
    assert!(ed.save_as(&path));
    ed.sync();
    assert!(!ed.is_dirty());
    assert!(ed.title().contains("roundtrip.scene.yaml"), "{}", ed.title());
    let checksum = ed.checksum();
    let mut other = demo_editor();
    assert!(other.open_path(&path));
    other.sync();
    assert_eq!(other.checksum(), checksum);
    assert_eq!(scene_text(&mut other), std::fs::read_to_string(&path).unwrap());
    // A missing file is an error message, and the document stays.
    assert!(!other.open_path(&temp_path("missing.scene.yaml")));
    other.sync();
    assert_eq!(other.checksum(), checksum);
    assert!(other.status().is_some_and(|m| m.error));
    let _ = std::fs::remove_file(path);
}

#[test]
fn save_writes_the_hosts_scene_file_and_clears_dirty() {
    let path = temp_path("save_in_place.scene.yaml");
    std::fs::copy(orr_editor::editor::default_scene_path(), &path).unwrap();
    let mut ed = Editor::open(&path).unwrap();
    ed.sync();
    ed.select_named("body_05");
    ed.set_field(&body(), "pos.x", fixed(4));
    assert!(ed.is_dirty());
    assert!(ed.save());
    ed.sync();
    assert!(!ed.is_dirty());
    assert_eq!(scene_text(&mut ed), std::fs::read_to_string(&path).unwrap());
    let _ = std::fs::remove_file(path);
}

use orr_editor::editor::Editor;

#[test]
fn stop_returns_to_the_unchanged_document() {
    let mut ed = demo_editor();
    ed.select_named("body_05");
    ed.set_field(&body(), "pos", vec2(1, 9));
    ed.sync();
    let yaml = scene_text(&mut ed);
    let checksum = ed.checksum();
    let history = ed.history().clone();
    ed.step(30);
    ed.sync();
    assert_eq!(ed.mode(), Mode::Play);
    assert_ne!(ed.checksum(), checksum, "the world moved");
    ed.set_field(&body(), "vel", vec2(5, 5));
    assert_eq!(scene_text(&mut ed), yaml, "play edits never touch the document");
    assert!(ed.stop().is_some());
    ed.sync();
    assert_eq!(ed.mode(), Mode::Edit);
    assert_eq!(scene_text(&mut ed), yaml);
    assert_eq!(ed.checksum(), checksum);
    assert_eq!(*ed.history(), history);
    assert!(ed.selection().is_some(), "selection survives play");
}

#[test]
fn undo_is_refused_in_play_mode() {
    let mut ed = demo_editor();
    ed.select_named("body_05");
    ed.set_field(&body(), "pos.x", fixed(2));
    ed.step(1);
    assert!(!ed.undo());
    assert!(ed.status().is_some_and(|m| m.error));
    ed.stop();
    assert!(ed.undo());
}

#[test]
fn step_pause_seek_and_replay_forward_give_identical_checksums() {
    let mut ed = demo_editor();
    ed.play();
    ed.pause();
    ed.step(60);
    ed.sync();
    let tl = ed.timeline().unwrap();
    assert_eq!((tl.tick, tl.last_tick, tl.playing), (60, 60, false));
    let first = checksums(&mut ed, 0, 60);
    assert_eq!(*first.last().unwrap(), tl.checksum);
    ed.seek(0);
    ed.sync();
    assert_eq!(ed.timeline().unwrap().tick, 0);
    assert_eq!(ed.checksum(), first[0]);
    ed.step(60);
    ed.sync();
    assert_eq!(checksums(&mut ed, 0, 60), first, "replaying forward reproduces every checksum");
}

#[test]
fn a_play_mode_edit_is_recorded_and_replays_to_the_same_checksum() {
    let mut ed = demo_editor();
    ed.select_named("body_05");
    ed.step(20);
    ed.sync();
    assert!(ed.set_field(&body(), "vel", vec2(7, 3)));
    ed.sync();
    assert!(ed.timeline().unwrap().pending_edits >= 1);
    ed.step(40);
    ed.sync();
    let head = ed.timeline().unwrap().tick;
    let live = ed.checksum();
    let stopped = ed.stop().cloned().expect("stopped");
    assert_eq!((stopped.tick, stopped.checksum), (head, live));
    let (last, replayed) = replay_end(&stopped.replay);
    assert_eq!(last, head);
    assert_eq!(replayed, live);
}

#[test]
fn play_mode_spawn_and_delete_are_recorded_too() {
    let mut ed = demo_editor();
    ed.step(10);
    ed.sync();
    assert!(ed.spawn_body([0.0, 30.0]));
    let spawned = ed.selection().cloned().unwrap();
    assert!(matches!(spawned, Target::Entity(_)), "an entity made in play has no GUID");
    ed.step(10);
    ed.sync();
    ed.select_named("body_07");
    assert!(ed.delete_selected());
    ed.step(10);
    ed.sync();
    let live = ed.checksum();
    let stopped = ed.stop().cloned().unwrap();
    assert_eq!(replay_end(&stopped.replay).1, live);
}

/// The M4 success criterion: edit, play 120 ticks, rewind to 30, play on to
/// the same checksum, with the simulation on the host's thread and the editor
/// seeing it only through ERP and frames.
#[test]
fn the_m4_loop_edit_play_rewind_play_again() {
    // Load the demo scene and edit a body's position.
    let mut ed = demo_editor();
    ed.select_named("body_05");
    assert!(ed.set_field(&body(), "pos", vec2(-3, 25)));
    // Play 120 ticks.
    ed.play();
    ed.pause();
    ed.step(120);
    ed.sync();
    assert_eq!(ed.timeline().unwrap().tick, 120);
    let first_run = checksums(&mut ed, 0, 120);
    // Rewind to tick 30.
    ed.seek(30);
    ed.sync();
    assert_eq!(ed.timeline().unwrap().tick, 30);
    assert_eq!(ed.checksum(), first_run[30]);
    // Play again by the clock (the host paces it, at 4x to keep the test short).
    ed.set_speed(4.0);
    ed.control(ControlOp::Play);
    let mut guard = 0;
    while ed.timeline().is_none_or(|t| t.tick < 120) {
        ed.pump();
        std::thread::sleep(std::time::Duration::from_millis(5));
        guard += 1;
        assert!(guard < 2000, "play by the clock never reached tick 120: {:?}", ed.timeline());
    }
    ed.pause();
    ed.sync();
    let tl = ed.timeline().unwrap();
    assert!(tl.tick >= 120);
    assert_eq!(tl.branches, 1, "playing from a rewound tick branches once");
    // Whatever tick the clock stopped at, every checksum agrees with the first run up to 120.
    let n = tl.tick.min(first_run.len() as u64 - 1);
    assert_eq!(checksums(&mut ed, 30, n), first_run[30..=n as usize].to_vec());
    if tl.tick == 120 {
        assert_eq!(tl.checksum, first_run[120], "checksum at tick 120 equals the first run");
    }
    // The edit is in the document, the play did not touch it.
    assert!(ed.is_dirty());
}

#[test]
fn speed_scales_the_ticks_per_second() {
    let mut ed = demo_editor();
    ed.play();
    ed.set_speed(4.0);
    ed.control(ControlOp::Play);
    let t0 = std::time::Instant::now();
    let start = ed.host_call("sim.state", json!({})).unwrap()["head_tick"].as_u64().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let end = ed.host_call("sim.state", json!({})).unwrap()["head_tick"].as_u64().unwrap();
    let secs = t0.elapsed().as_secs_f64();
    let rate = (end - start) as f64 / secs;
    // 60 ticks/s at 4x is 240 ticks/s (the pacer catches up at most 8 ticks per frame of the host).
    assert!((120.0..=260.0).contains(&rate), "4x speed ran {rate:.0} ticks/s");
    // Paused: no ticks.
    ed.pause();
    let a = ed.host_call("sim.state", json!({})).unwrap()["head_tick"].as_u64().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
    let b = ed.host_call("sim.state", json!({})).unwrap()["head_tick"].as_u64().unwrap();
    assert_eq!(a, b);
}

#[test]
fn scripts_drive_the_same_paths() {
    let mut ed = demo_editor();
    let text = "
        # a small session
        select body_05
        set orr_physics::Body pos 3 4.5
        set Scene max_entities 5000
        play
        pause
        step 30
        seek 10
        stop
        undo
    ";
    script::run_script(&mut ed, text).expect("script runs");
    assert_eq!(ed.mode(), Mode::Edit);
    assert_eq!(ed.history().entries.iter().filter(|h| !h.undone).count(), 1, "two edits, one undone");
    assert!(script::run_script(&mut ed, "select nobody").is_err());
    assert!(script::run_script(&mut ed, "set orr_physics::Body pos 99999 0").is_err());
    assert!(script::run_script(&mut ed, "frobnicate").is_err());
}

#[test]
fn fixed_from_view_numbers_goes_through_decimal_text() {
    assert_eq!(fp_of_f64(1.5).unwrap().raw(), 3 << 15);
    assert_eq!(fp_of_f64(-0.000001).unwrap().raw(), 0);
    assert!(fp_of_f64(f64::NAN).is_none());
    assert!(fp_of_f64(f64::INFINITY).is_none());
    // The same text gives the same value everywhere.
    assert_eq!(fp_of_f64(0.1).unwrap(), orr_reflect::decimal::parse_fp("0.10000").unwrap());
}

#[test]
fn picking_finds_the_body_under_a_world_point() {
    let mut ed = demo_editor();
    let mut pos = |name: &str| {
        let t = target_named(&ed, name);
        xy(&field(&mut ed, &t, BODY, "pos"))
    };
    let (paddle, floor) = (pos("paddle_0"), pos("floor"));
    assert_eq!(ed.pick(paddle), Some(target_named(&ed, "paddle_0")));
    assert_eq!(ed.pick(floor), Some(target_named(&ed, "floor")));
    assert_eq!(ed.pick([500.0, 500.0]), None, "nothing far outside the scene");
}

#[test]
fn the_frame_on_screen_is_the_hosts_frame() {
    // What the viewport draws is a snapshot the host published: same checksum as the host's live frame.
    let mut ed = demo_editor();
    assert_eq!(ed.checksum(), doc_checksum(&mut ed));
    ed.select_named("body_05");
    ed.set_field(&body(), "pos.x", fixed(2));
    ed.sync();
    assert_eq!(ed.checksum(), doc_checksum(&mut ed), "an edit shows up in the next frame");
    ed.step(15);
    ed.sync();
    let live = orr_remote::wire::parse_checksum(&ed.host_call("sim.state", json!({})).unwrap()["checksum"]).unwrap();
    assert_eq!(ed.checksum(), live);
    assert_eq!(ed.snapshot().unwrap().tick(), 15);
    assert!(!ed.bodies().is_empty());
}
