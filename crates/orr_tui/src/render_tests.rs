//! The renderer is a pure function: a tiny synthetic frame has a pinned ASCII picture.

use orr_viewstream::{EntityRecord, ViewFrame, MODE_NONE, MODE_PREDICTION, SHAPE_CAPSULE, SHAPE_CIRCLE, SHAPE_QUAD};

use crate::render::{pose, render, Camera};
use crate::schema::{KindInfo, ViewSchema};

fn schema() -> ViewSchema {
    let kind = |id, name: &str| KindInfo { id, name: name.to_string(), props: Vec::new() };
    ViewSchema {
        game: "T".into(),
        tick_rate: 60,
        player_count: 1,
        kinds: vec![kind(0, "static"), kind(1, "dynamic"), kind(3, "paddle")],
        input_size: 0,
        input_fields: Vec::new(),
        events: Vec::new(),
    }
}

fn rec(kind: u16, shape: u8, mode: u8, size: f32, half_y: f32, prev: [f32; 3], cur: [f32; 3]) -> EntityRecord {
    EntityRecord { id: u64::from(kind), kind, shape, mode, size, half_y, rgba: [200, 100, 50, 255], prev, cur }
}

/// A floor, a ball that moved from x = -2 to x = 2 and a paddle.
fn frame() -> ViewFrame {
    ViewFrame {
        flags: 0,
        tick: 1,
        verified_tick: 1,
        seq: 1,
        rollback: None,
        entities: vec![
            rec(0, SHAPE_QUAD, MODE_NONE, 5.0, 0.5, [0.0, -2.5, 0.0], [0.0, -2.5, 0.0]),
            rec(1, SHAPE_CIRCLE, MODE_PREDICTION, 1.0, 0.0, [-2.0, 1.0, 0.0], [2.0, 1.0, 0.0]),
            rec(3, SHAPE_CAPSULE, MODE_PREDICTION, 1.5, 0.3, [-3.0, -1.5, 0.0], [-3.0, -1.5, 0.0]),
        ],
        props: Vec::new(),
    }
}

#[test]
fn a_tiny_frame_renders_to_this_ascii_grid() {
    let f = frame();
    let cam = Camera { min: [-5.0, -3.0], max: [5.0, 3.0] };
    let at_end = render(&f, &schema(), &cam, 1.0, 20, 6, false).text();
    let golden_end = "\n            oooo\n            oooo\n\n========\n####################\n";
    assert_eq!(at_end, golden_end);
    // Halfway (the viewer's own alpha), the ball is at x = 0.
    let halfway = render(&f, &schema(), &cam, 0.5, 20, 6, false).text();
    let golden_half = "\n        oooo\n        oooo\n\n========\n####################\n";
    assert_eq!(halfway, golden_half);
}

#[test]
fn camera_fit_uses_the_static_entities() {
    let cam = Camera::fit(&frame());
    // The floor spans x -5..5, y -3..-2 (reach counts the half diagonal, so a bit more), the ball at x=2 is inside it.
    assert!(cam.min[0] < -5.0 && cam.max[0] > 5.0);
    assert!(cam.max[1] < 2.0, "the ball at y = 1 does not stretch a camera fitted to the walls: {cam:?}");
}

#[test]
fn pose_blends_position_and_takes_the_short_way_round() {
    let e = rec(1, SHAPE_CIRCLE, MODE_PREDICTION, 1.0, 0.0, [0.0, 0.0, 3.0], [2.0, 4.0, -3.0]);
    let p = pose(&e, 0.5);
    assert_eq!((p[0], p[1]), (1.0, 2.0));
    assert!((p[2].abs() - std::f32::consts::PI).abs() < 0.1, "3 rad to -3 rad goes through pi, not through 0: {}", p[2]);
    assert_eq!(pose(&e, 1.0), e.cur);
    let still = rec(0, SHAPE_QUAD, MODE_NONE, 1.0, 0.0, [0.0; 3], [5.0, 5.0, 0.0]);
    assert_eq!(pose(&still, 0.3), still.cur, "mode none shows the newest tick as it is");
}
