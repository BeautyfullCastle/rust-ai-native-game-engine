//! The renderer is a pure function: a tiny synthetic frame has a pinned ASCII picture.

use orr_viewstream::{
    EntityRecord, EntityRecord3, Pose3, ViewFrame, ViewFrame3, MODE_NONE, MODE_PREDICTION, SHAPE_CAPSULE, SHAPE_CIRCLE, SHAPE_QUAD,
    SHAPE3_BOX, SHAPE3_CAPSULE, SHAPE3_PLANE, SHAPE3_SPHERE,
};

use crate::render::{pose, render, Camera};
use crate::render3d::{pose as pose3, render as render3, Camera3};
use crate::schema::{KindInfo, ViewSchema};

fn schema() -> ViewSchema {
    let kind = |id, name: &str| KindInfo { id, name: name.to_string(), props: Vec::new() };
    ViewSchema {
        game: "T".into(),
        dimensions: 2,
        tick_rate: 60,
        player_count: 1,
        kinds: vec![kind(0, "static"), kind(1, "dynamic"), kind(3, "paddle")],
        input_size: 0,
        input_fields: Vec::new(),
        events: Vec::new(),
    }
}

fn schema3() -> ViewSchema {
    let kind = |id, name: &str| KindInfo { id, name: name.to_string(), props: Vec::new() };
    ViewSchema {
        game: "Yard3D".into(),
        dimensions: 3,
        tick_rate: 60,
        player_count: 1,
        kinds: vec![kind(0, "ground"), kind(1, "ball"), kind(2, "box"), kind(3, "capsule"), kind(4, "plane")],
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

fn rec3(kind: u16, shape: u8, mode: u8, size: [f32; 3], pos: [f32; 3]) -> EntityRecord3 {
    EntityRecord3 {
        id: u64::from(kind),
        kind,
        shape,
        mode,
        size,
        rgba: [220, 180, 80, 255],
        roughness: 128,
        metallic: 0,
        style_flags: 0,
        prev: Pose3 { pos, rot: Pose3::IDENTITY.rot },
        cur: Pose3 { pos, rot: Pose3::IDENTITY.rot },
    }
}

fn frame3() -> ViewFrame3 {
    ViewFrame3 {
        flags: 0,
        tick: 1,
        verified_tick: 1,
        seq: 1,
        rollback: None,
        entities: vec![
            rec3(0, SHAPE3_PLANE, MODE_NONE, [5.0, 0.0, 5.0], [0.0, 0.0, 0.0]),
            rec3(1, SHAPE3_SPHERE, MODE_PREDICTION, [0.75, 0.0, 0.0], [-2.0, 3.0, 0.0]),
            rec3(2, SHAPE3_BOX, MODE_PREDICTION, [0.7, 1.0, 0.7], [0.0, 3.0, 0.0]),
            rec3(3, SHAPE3_CAPSULE, MODE_PREDICTION, [0.35, 0.8, 0.0], [2.0, 3.0, 0.0]),
        ],
        props: Vec::new(),
    }
}

#[test]
fn xz_renderer_projects_the_four_wire_shapes_and_ignores_world_y() {
    let f = frame3();
    let cam = Camera3::fit(&f);
    let drawn = render3(&f, &schema3(), &cam, 1.0, 40, 12, false);
    let chars: String = drawn.cells.iter().map(|cell| cell.ch).collect();
    for expected in ['#', 'o', 'B', '='] {
        assert!(chars.contains(expected), "expected {expected:?} in XZ projection\n{}", drawn.text());
    }

    let mut raised = f.clone();
    for entity in &mut raised.entities {
        entity.prev.pos[1] += 17.0;
        entity.cur.pos[1] += 17.0;
    }
    assert_eq!(Camera3::fit(&raised), cam, "vertical translation is ignored by orthographic XZ fit");
    assert_eq!(render3(&raised, &schema3(), &cam, 1.0, 40, 12, false), drawn);
}

#[test]
fn pose3_slerps_shortest_arc_and_mode_none_uses_current_pose() {
    let mut e = rec3(1, SHAPE3_BOX, MODE_PREDICTION, [1.0; 3], [0.0; 3]);
    let half = 85.0_f32.to_radians();
    e.prev.rot = [0.0, half.sin(), 0.0, half.cos()];
    e.cur.rot = [0.0, -half.sin(), 0.0, half.cos()];
    e.cur.pos = [2.0, 4.0, 6.0];
    let blended = pose3(&e, 0.5);
    assert_eq!(blended.pos, [1.0, 2.0, 3.0]);
    assert!(blended.rot[1].abs() > 0.99, "shortest arc crosses 180 degrees: {:?}", blended.rot);
    e.mode = MODE_NONE;
    assert_eq!(pose3(&e, 0.0), e.cur);
}

#[test]
fn translated_box_and_plane_fill_their_xz_footprint_away_from_the_origin() {
    let camera = Camera3 { min: [-5.0; 2], max: [5.0; 2] };
    for shape in [SHAPE3_BOX, SHAPE3_PLANE] {
        for (pos, columns, rows) in [([2.0, 7.0, -2.0], 24..32, 12..16), ([-2.0, 7.0, 2.0], 8..16, 4..8)] {
            let size = [1.0, if shape == SHAPE3_PLANE { 0.0 } else { 1.0 }, 1.0];
            let mut frame = frame3();
            frame.entities = vec![rec3(2, shape, MODE_NONE, size, pos)];
            let grid = render3(&frame, &schema3(), &camera, 1.0, 40, 20, false);
            for row in 0..20 {
                for col in 0..40 {
                    assert_eq!(grid.cells[row * 40 + col].ch != ' ', rows.contains(&row) && columns.contains(&col),
                        "shape {shape} at {pos:?}, cell ({col},{row})\n{}", grid.text());
                }
            }
        }
    }
}
