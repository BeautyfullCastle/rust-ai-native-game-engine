use super::*;

#[test]
fn input_origin_preserves_exact_fixed_point_coordinates_at_bounds() {
    assert_eq!(
        input_origin([
            FP::from_raw(-(256 * FP::SCALE)),
            FP::from_raw(256 * FP::SCALE),
            FP::from_raw(-1),
        ])
        .unwrap(),
        [-(256 * FP::SCALE) as i32, (256 * FP::SCALE) as i32, -1]
    );
}

#[test]
fn input_origin_rejects_out_of_range_without_saturating() {
    for position in [
        [FP::from_raw(256 * FP::SCALE + 1), FP::ZERO, FP::ZERO],
        [FP::ZERO, FP::from_raw(-256 * FP::SCALE - 1), FP::ZERO],
        [FP::ZERO, FP::ZERO, FP::from_raw(i64::MAX)],
    ] {
        assert!(input_origin(position).is_err());
    }
}

#[test]
fn scene_reload_and_graph_revision_changes_are_distinct_contexts() {
    let original = Context {
        scene: PathBuf::from("C:/mesh/arena.scene.yaml"),
        epoch: 4,
        graph_revision: [1; 32],
    };
    let reloaded = Context {
        epoch: 5,
        ..original.clone()
    };
    let edited_graph = Context {
        graph_revision: [2; 32],
        ..original.clone()
    };
    assert_ne!(original, reloaded);
    assert_ne!(original, edited_graph);
}

fn key_event(key: egui::Key, pressed: bool, repeat: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat,
        modifiers: egui::Modifiers::NONE,
    }
}

fn read_direction(context: &egui::Context, events: Vec<egui::Event>) -> [i8; 2] {
    let mut direction = [0, 0];
    let _output = context.run_ui(
        egui::RawInput {
            events,
            focused: true,
            ..Default::default()
        },
        |ui| direction = ui.input(keyboard_direction),
    );
    direction
}

#[test]
fn keyboard_direction_maps_world_axes_for_wasd_and_arrow_events() {
    for (key, expected) in [
        (egui::Key::W, [0, -1]),
        (egui::Key::ArrowUp, [0, -1]),
        (egui::Key::S, [0, 1]),
        (egui::Key::ArrowDown, [0, 1]),
        (egui::Key::A, [-1, 0]),
        (egui::Key::ArrowLeft, [-1, 0]),
        (egui::Key::D, [1, 0]),
        (egui::Key::ArrowRight, [1, 0]),
    ] {
        assert_eq!(
            read_direction(&egui::Context::default(), vec![key_event(key, true, false)]),
            expected
        );
    }
}

#[test]
fn keyboard_steps_require_press_or_repeat_not_render_frames_or_release() {
    let context = egui::Context::default();
    assert_eq!(
        read_direction(&context, vec![key_event(egui::Key::D, true, false)]),
        [1, 0]
    );
    assert_eq!(read_direction(&context, vec![]), [0, 0]);
    assert_eq!(
        read_direction(&context, vec![key_event(egui::Key::D, true, true)]),
        [1, 0]
    );
    assert_eq!(
        read_direction(&context, vec![key_event(egui::Key::D, false, false)]),
        [0, 0]
    );
}

#[test]
fn keyboard_opposites_cancel_and_diagonals_use_current_held_axes() {
    let context = egui::Context::default();
    assert_eq!(
        read_direction(&context, vec![key_event(egui::Key::A, true, false)]),
        [-1, 0]
    );
    assert_eq!(
        read_direction(&context, vec![key_event(egui::Key::D, true, false)]),
        [0, 0]
    );
    assert_eq!(
        read_direction(
            &context,
            vec![
                key_event(egui::Key::A, false, false),
                key_event(egui::Key::W, true, false),
            ]
        ),
        [1, -1]
    );
}

#[test]
fn keyboard_focus_request_is_explicit_and_consumed_once() {
    let mut panel = Panel::default();
    assert!(!panel.take_keyboard_focus());
    panel.keyboard_focus_requested = true;
    assert!(panel.take_keyboard_focus());
    assert!(!panel.take_keyboard_focus());
}
