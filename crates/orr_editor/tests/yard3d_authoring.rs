//! Main-editor Yard3D authoring against the real local ERP host and bridge.
//! Headless egui assertions cover widget behavior, not GPU rendering evidence.
#![allow(clippy::disallowed_types)]
#![allow(clippy::float_arithmetic)] // Camera/pointer coordinates are view-only.

mod common;

use std::time::{Duration, Instant};

use common::{checksums, scene_text, target_named, temp_path};
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::app::{LBL_PAUSE, LBL_PLAY, LBL_STEP, LBL_STOP};
use orr_editor::game::EditorGame;
use orr_editor::{Editor, EditorApp, Mode, Target};
use orr_fp::{FPVec2, FPVec3, FP};
use orr_reflect::Value;
use orr_remote::yard3d::yard3d_doc_from_yaml;
use serde_json::{json, Value as J};

const BODY: &str = "orr_physics3d::Body";
const FIXTURE: &str = include_str!("../../../scenes/yard3d_authoring.scene.yaml");

fn editor(name: &str) -> Editor {
    let path = temp_path(name);
    std::fs::write(&path, FIXTURE).unwrap();
    let mut editor = Editor::open_game(&path, EditorGame::Yard3D).expect("real Yard3D editor");
    editor.sync();
    assert_eq!(editor.game(), EditorGame::Yard3D);
    assert_eq!(editor.rows().len(), 3);
    assert_eq!(editor.yard_frame().items.len(), 3);
    assert_eq!(editor.snapshot().unwrap().predicted().alive_count(), 3);
    assert_eq!(
        editor.checksum(),
        yard3d_doc_from_yaml(FIXTURE).unwrap().checksum()
    );
    assert!(editor.bodies().is_empty(), "3D does not use 2D drawables");
    editor
}

fn pos(x: i32, y: i32, z: i32) -> FPVec3 {
    FPVec3::new(FP::from_int(x), FP::from_int(y), FP::from_int(z))
}

fn document_bytes(editor: &mut Editor) -> Vec<u8> {
    yard3d_doc_from_yaml(&scene_text(editor))
        .unwrap()
        .frame()
        .to_bytes()
}

fn field(editor: &mut Editor, target: &Target, path: &str) -> Value {
    editor.field_of(target, BODY, path).unwrap()
}

#[test]
fn real_editor_spawns_distinct_guids_and_picks_nearest_3d_body_without_vec2_nudges() {
    let mut editor = editor("yard-spawn-pick.scene.yaml");
    assert!(
        editor.spawn_yard_body(pos(0, 2, -2)),
        "{:?}",
        editor.status()
    );
    editor.sync();
    let far = editor.selection().unwrap().clone();
    assert!(matches!(far, Target::Guid(_)));
    assert!(
        editor.spawn_yard_body(pos(0, 2, 2)),
        "{:?}",
        editor.status()
    );
    editor.sync();
    let near = editor.selection().unwrap().clone();
    assert!(matches!(near, Target::Guid(_)));
    assert_ne!(near, far);
    assert_eq!(editor.rows().len(), 5);
    assert_eq!(editor.yard_frame().items.len(), 5);
    assert_eq!(editor.history().entries.len(), 2);
    assert_eq!(field(&mut editor, &far, "pos"), Value::Vec3(pos(0, 2, -2)));
    assert_eq!(field(&mut editor, &near, "pos"), Value::Vec3(pos(0, 2, 2)));

    editor.camera3d = orr_render::OrbitCamera::new([0.0, 2.0, 0.0], 0.0, 0.0, 10.0);
    // The field reads above ingest ERP notifications. A delayed history hint
    // can invalidate the GUID map after sync, even when both counts are current.
    wait_for_coherent_pick(&mut editor, &near, 5);
    assert_eq!(
        editor.pick3d([400.0, 300.0], (800, 600)),
        Some(near.clone())
    );
    editor.camera3d.yaw = std::f32::consts::PI;
    assert_eq!(editor.pick3d([400.0, 300.0], (800, 600)), Some(far));

    let before = document_bytes(&mut editor);
    let history = editor.history().clone();
    assert!(!editor.can_nudge_selection());
    assert!(!editor.nudge_selected(FPVec2::new(FP::ONE, FP::ZERO)));
    editor.sync();
    assert_eq!(document_bytes(&mut editor), before);
    assert_eq!(editor.history(), &history);
    assert_eq!(editor.selection(), Some(&near));
}

fn wait_for_coherent_pick(editor: &mut Editor, expected: &Target, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if editor.yard_rows_coherent() {
            assert_eq!(editor.yard_frame().items.len(), count);
            assert_eq!(
                editor.pick3d([400.0, 300.0], (800, 600)),
                Some(expected.clone())
            );
            break;
        }
        assert_eq!(
            editor.pick3d([400.0, 300.0], (800, 600)),
            None,
            "an in-flight GUID refresh must not select from stale entity handles"
        );
        assert!(
            Instant::now() < deadline,
            "Yard3D rows never caught up with the snapshot: expected={expected:?}, \
             expected_count={count}, rows={}, items={}, checksum={:#018x}, status={:?}",
            editor.rows().len(),
            editor.yard_frame().items.len(),
            editor.checksum(),
            editor.status()
        );
        editor.pump();
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn unsynced_delete_spawn_and_undo_fence_guid_picking_across_rebaked_entity_handles() {
    let mut editor = editor("yard-row-fence.scene.yaml");
    assert!(editor.yard_rows_coherent());
    let left = target_named(&editor, "box_left");
    let right = target_named(&editor, "box_right");
    let old_left_handle = editor.entity_of(&left).unwrap();
    let old_right_handle = editor.entity_of(&right).unwrap();
    assert_ne!(old_left_handle, old_right_handle);
    editor.camera3d = orr_render::OrbitCamera::new([2.0, 3.0, 0.0], 0.0, 0.0, 10.0);
    assert_eq!(
        editor.pick3d([400.0, 300.0], (800, 600)),
        Some(right.clone())
    );

    editor.select(Some(left.clone()));
    assert!(editor.delete_selected());
    assert!(!editor.yard_rows_coherent());
    // Pump the actual asynchronous ERP replies and frame stream independently;
    // do not hide the intermediate stale-row window behind Editor::sync.
    wait_for_coherent_pick(&mut editor, &right, 2);
    assert!(editor.row_of(&left).is_none());
    assert_eq!(
        editor.entity_of(&right),
        Some(old_left_handle),
        "rebaking reused the complete old entity handle for a different GUID"
    );

    assert!(editor.spawn_yard_body(pos(2, 3, 4)));
    let near = editor.selection().unwrap().clone();
    assert_ne!(near, left);
    assert_ne!(near, right);
    assert!(!editor.yard_rows_coherent());
    wait_for_coherent_pick(&mut editor, &near, 3);

    assert!(editor.undo());
    assert!(!editor.yard_rows_coherent());
    wait_for_coherent_pick(&mut editor, &right, 2);
    assert!(editor.row_of(&near).is_none());
    assert!(editor.undo());
    assert!(!editor.yard_rows_coherent());
    wait_for_coherent_pick(&mut editor, &right, 3);
    assert!(editor.row_of(&left).is_some());
    assert_eq!(editor.entity_of(&right), Some(old_right_handle));
    editor.sync();
    assert!(editor.yard_rows_coherent());
}

#[test]
fn exact_xyz_quaternion_is_one_undo_and_invalid_rotation_rolls_back_the_whole_transaction() {
    let mut editor = editor("yard-transform.scene.yaml");
    assert!(editor.select_named("box_left"));
    editor.sync();
    let target = editor.selection().unwrap().clone();
    let original = document_bytes(&mut editor);
    let original_text = scene_text(&mut editor);
    let position = FPVec3::new(
        "1.125".parse().unwrap(),
        "2.25".parse().unwrap(),
        "-3.5".parse().unwrap(),
    );
    let rotation = [
        FP::ZERO,
        "0.6".parse().unwrap(),
        FP::ZERO,
        "0.8".parse().unwrap(),
    ];
    assert!(
        editor.set_yard_transform(position, rotation),
        "{:?}",
        editor.status()
    );
    editor.sync();
    assert_eq!(field(&mut editor, &target, "pos"), Value::Vec3(position));
    for (axis, expected) in ["x", "y", "z", "w"].into_iter().zip(rotation) {
        assert_eq!(
            field(&mut editor, &target, &format!("rot.{axis}")),
            Value::Fixed(expected)
        );
    }
    assert_eq!(editor.history().entries.len(), 1);
    assert_eq!(editor.history().entries[0].op_count, 2);
    assert_eq!(editor.history().entries[0].origin, "user");
    let edited = document_bytes(&mut editor);
    let edited_text = scene_text(&mut editor);
    let edited_sum = editor.checksum();
    let history = editor.history().clone();

    assert!(!editor.set_yard_transform(pos(9, 9, 9), [FP::ZERO; 4]));
    editor.sync();
    assert_eq!(editor.checksum(), edited_sum);
    assert_eq!(document_bytes(&mut editor), edited);
    assert_eq!(scene_text(&mut editor), edited_text);
    assert_eq!(editor.history(), &history);
    assert!(!editor.sim().in_tx);
    assert_eq!(field(&mut editor, &target, "pos"), Value::Vec3(position));

    assert!(editor.undo());
    editor.sync();
    assert_eq!(document_bytes(&mut editor), original);
    assert_eq!(scene_text(&mut editor), original_text);
    assert!(editor.redo());
    editor.sync();
    assert_eq!(document_bytes(&mut editor), edited);
    assert_eq!(editor.history().entries.len(), 1);
}

#[test]
fn save_reopen_preserves_guids_and_failed_load_keeps_the_last_document_and_path() {
    let mut editor = editor("yard-save-source.scene.yaml");
    assert!(editor.spawn_yard_body(pos(4, 5, 6)));
    editor.sync();
    let spawned = editor.selection().unwrap().clone();
    let bytes = document_bytes(&mut editor);
    let saved = temp_path("yard-saved.scene.yaml");
    assert!(editor.save_as(&saved));
    editor.sync();
    assert_eq!(editor.path(), Some(saved.clone()));
    assert!(!editor.is_dirty());
    let mut reopened = Editor::open_game(&saved, EditorGame::Yard3D).unwrap();
    reopened.sync();
    assert_eq!(reopened.rows().len(), 4);
    assert!(reopened.row_of(&spawned).is_some());
    assert_eq!(document_bytes(&mut reopened), bytes);
    assert_eq!(reopened.checksum(), editor.checksum());
    assert_eq!(
        field(&mut reopened, &spawned, "pos"),
        Value::Vec3(pos(4, 5, 6))
    );

    let bad = temp_path("yard-rejected.scene.yaml");
    std::fs::write(&bad, "schema: orr.scene/1\nentities: [\n").unwrap();
    let history = editor.history().clone();
    let selected = editor.selection().cloned();
    assert!(!editor.open_path(&bad));
    editor.sync();
    assert_eq!(editor.path(), Some(saved.clone()));
    assert_eq!(editor.selection(), selected.as_ref());
    assert_eq!(editor.history(), &history);
    assert_eq!(document_bytes(&mut editor), bytes);
    assert_eq!(editor.yard_frame().items.len(), 4);
    assert_eq!(
        editor.host_call("sim.state", J::Null).unwrap()["scene_path"],
        saved.display().to_string()
    );
}

#[test]
fn real_editor_play_pause_step_seek_stop_keeps_the_authored_document() {
    let mut editor = editor("yard-play.scene.yaml");
    let document = document_bytes(&mut editor);
    let original = editor.checksum();
    assert!(editor.start_play());
    editor.sync();
    assert_eq!(editor.mode(), Mode::Play);
    assert!(!editor.timeline().unwrap().playing);
    editor.step(8);
    editor.sync();
    assert_eq!(editor.timeline().unwrap().tick, 8);
    let recorded = checksums(&mut editor, 0, 8);
    assert_eq!(editor.checksum(), recorded[8]);
    assert!(!editor.spawn_yard_body(pos(1, 2, 3)));
    editor.play();
    let deadline = Instant::now() + Duration::from_secs(10);
    while editor.sim().head_tick <= 8 {
        assert!(Instant::now() < deadline, "Yard3D play did not advance");
        editor.pump();
        std::thread::sleep(Duration::from_millis(5));
    }
    editor.pause();
    editor.sync();
    assert!(!editor.timeline().unwrap().playing);
    let paused = editor.sim().head_tick;
    std::thread::sleep(Duration::from_millis(40));
    editor.sync();
    assert_eq!(editor.sim().head_tick, paused);
    editor.seek(3);
    editor.sync();
    assert_eq!(editor.timeline().unwrap().tick, 3);
    assert_eq!(editor.checksum(), recorded[3]);
    editor.seek(8);
    editor.sync();
    assert_eq!(editor.checksum(), recorded[8]);
    assert!(editor.stop().is_some());
    editor.sync();
    assert_eq!(editor.mode(), Mode::Edit);
    assert!(editor.timeline().is_none());
    assert_eq!(editor.checksum(), original);
    assert_eq!(document_bytes(&mut editor), document);
    assert_eq!(editor.yard_frame().items.len(), 3);
}

fn settle(harness: &mut Harness<'_, EditorApp>) {
    for _ in 0..3 {
        harness.state_mut().editor.sync();
        harness.run_steps(2);
    }
}

fn click_at(harness: &mut Harness<'_, EditorApp>, at: Pos2) {
    harness.event(Event::PointerMoved(at));
    harness.step();
    for pressed in [true, false] {
        harness.event(Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        harness.step();
    }
    settle(harness);
}

fn type_transform_field(harness: &mut Harness<'_, EditorApp>, label: &str, text: &str) {
    harness.get_by_label(label).focus();
    harness.run_steps(2);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.run_steps(2);
    harness.get_by_label(label).type_text(text);
    harness.run_steps(2);
    harness.key_press(Key::Enter);
    harness.run_steps(2);
    assert_eq!(harness.get_by_label(label).value().as_deref(), Some(text));
}

#[test]
fn main_inspector_exact_transform_form_applies_once_and_undo_restores_the_document() {
    let editor = editor("yard-form.scene.yaml");
    let mut harness = Harness::builder()
        .with_size([1500.0, 1000.0])
        .build_eframe(move |_| EditorApp::new(editor, None));
    settle(&mut harness);
    harness.get_by_label("box_left").click();
    settle(&mut harness);
    let target = target_named(&harness.state().editor, "box_left");
    harness.get_by_label("Exact 3D transform").click();
    settle(&mut harness);
    let initial = document_bytes(&mut harness.state_mut().editor);
    let position_text = ["1.125", "2.25", "-3.5"];
    let rotation_text = ["0", "0.6", "0", "0.8"];
    for (axis, text) in ["X", "Y", "Z"].into_iter().zip(position_text) {
        type_transform_field(&mut harness, &format!("Position {axis}"), text);
    }
    for (axis, text) in ["x", "y", "z", "w"].into_iter().zip(rotation_text) {
        type_transform_field(&mut harness, &format!("Rotation {axis}"), text);
    }
    assert_eq!(document_bytes(&mut harness.state_mut().editor), initial);
    assert!(harness.state().editor.history().entries.is_empty());
    harness.get_by_label("Apply XYZ + rotation").click();
    settle(&mut harness);
    let wanted = FPVec3::new(
        position_text[0].parse().unwrap(),
        position_text[1].parse().unwrap(),
        position_text[2].parse().unwrap(),
    );
    assert_eq!(
        field(&mut harness.state_mut().editor, &target, "pos"),
        Value::Vec3(wanted)
    );
    for (axis, value) in ["x", "y", "z", "w"].into_iter().zip(rotation_text) {
        assert_eq!(
            field(
                &mut harness.state_mut().editor,
                &target,
                &format!("rot.{axis}")
            ),
            Value::Fixed(value.parse().unwrap())
        );
    }
    assert_eq!(harness.state().editor.history().entries.len(), 1);
    assert_eq!(harness.state().editor.history().entries[0].op_count, 2);
    let edited = document_bytes(&mut harness.state_mut().editor);
    let history = harness.state().editor.history().clone();

    // The same production form must not leave its position patch behind if
    // the quaternion is invalid. No host writes happen before Apply.
    type_transform_field(&mut harness, "Position X", "9");
    for axis in ["x", "y", "z", "w"] {
        type_transform_field(&mut harness, &format!("Rotation {axis}"), "0");
    }
    harness.get_by_label("Apply XYZ + rotation").click();
    settle(&mut harness);
    assert_eq!(document_bytes(&mut harness.state_mut().editor), edited);
    assert_eq!(harness.state().editor.history(), &history);
    assert!(!harness.state().editor.sim().in_tx);
    // A rollback can also publish a view-recovery info message during settle.
    // The actionable refusal must remain in the log even if it is no longer
    // the most recent status-bar message.
    assert!(harness
        .state()
        .editor
        .log()
        .iter()
        .any(|message| message.error && message.text.contains("quaternion")));

    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    settle(&mut harness);
    assert_eq!(document_bytes(&mut harness.state_mut().editor), initial);
    assert_eq!(harness.state().editor.history().entries.len(), 1);
    assert!(harness.state().editor.history().entries[0].undone);
}

#[test]
fn headless_main_app_box_hierarchy_viewport_and_timeline_widgets_use_the_real_host() {
    let editor = editor("yard-widget.scene.yaml");
    let mut harness = Harness::builder()
        .with_size([1500.0, 1000.0])
        .build_eframe(move |_| EditorApp::new(editor, None));
    settle(&mut harness);
    assert!(harness.query_by_label("+ Body").is_none());
    harness.get_by_label("+ 3D Box").click();
    settle(&mut harness);
    assert_eq!(harness.state().editor.rows().len(), 4);
    assert_eq!(harness.state().editor.history().entries.len(), 1);
    let first = harness.state().editor.selected_guid().unwrap();
    harness.get_by_label("+ 3D Box").click();
    settle(&mut harness);
    assert_eq!(harness.state().editor.rows().len(), 5);
    assert_ne!(harness.state().editor.selected_guid().unwrap(), first);

    harness.get_by_label("box_left").click();
    settle(&mut harness);
    let left = target_named(&harness.state().editor, "box_left");
    assert_eq!(harness.state().editor.selection(), Some(&left));
    assert!(harness.query_by_label(BODY).is_some());
    assert!(harness.query_by_label("Exact 3D transform").is_some());
    let before = document_bytes(&mut harness.state_mut().editor);
    assert!(
        harness.query_by_label("Move right").is_none(),
        "Yard3D must not offer Vec2 nudges"
    );
    settle(&mut harness);
    assert_eq!(document_bytes(&mut harness.state_mut().editor), before);

    let viewport = harness.state().ui.viewport_rect.unwrap();
    let pixels = harness.state().ui.viewport_px;
    let point = harness
        .state()
        .editor
        .camera3d
        .camera()
        .world_to_screen([2.0, 3.0, 0.0], pixels)
        .unwrap();
    let at = viewport.min + egui::vec2(point[0], point[1]);
    assert!(viewport.contains(at));
    click_at(&mut harness, at);
    let right = target_named(&harness.state().editor, "box_right");
    assert_eq!(harness.state().editor.selection(), Some(&right));

    let edit_sum = harness.state().editor.checksum();
    harness.get_by_label(LBL_STEP).click();
    settle(&mut harness);
    assert_eq!(harness.state().editor.timeline().unwrap().tick, 1);
    harness.get_by_label(LBL_PLAY).click();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !harness
        .state()
        .editor
        .timeline()
        .is_some_and(|time| time.playing && time.tick > 3)
    {
        assert!(
            Instant::now() < deadline,
            "Play widget did not drive the Yard3D host"
        );
        harness.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    harness.get_by_label(LBL_PAUSE).click();
    settle(&mut harness);
    assert!(!harness.state().editor.timeline().unwrap().playing);
    harness.get_by_label("\u{23EE}").click();
    settle(&mut harness);
    assert_eq!(harness.state().editor.timeline().unwrap().tick, 0);
    harness.get_by_label(LBL_STOP).click();
    settle(&mut harness);
    assert_eq!(harness.state().editor.mode(), Mode::Edit);
    assert_eq!(harness.state().editor.checksum(), edit_sum);
    assert_eq!(harness.state().editor.yard_frame().items.len(), 5);
    assert_eq!(
        harness
            .state_mut()
            .editor
            .host_call("scene.save", json!({}))
            .unwrap()["checksum"],
        format!("0x{edit_sum:016x}")
    );
}
