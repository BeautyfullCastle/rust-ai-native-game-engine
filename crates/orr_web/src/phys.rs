//! The physics sample in the browser: the bot and the drawing data, integers only.

use orr_ecs::Frame;
use orr_fp::FP;
use orr_games::physics_game::{layout, PaddleTag, PhysConfig};
use orr_physics::{Body, Collider, BODY_DYNAMIC, BODY_KINEMATIC, SHAPE_CAPSULE, SHAPE_CIRCLE};

/// Integers per body in [`render_phys`].
pub const PHYS_STRIDE: usize = 8;

/// `shape` field: a circle (`size` = radius).
pub const DRAW_CIRCLE: i32 = 0;
/// `shape` field: a box (`size`, `half_y` = half extents; polygons use their bounding box).
pub const DRAW_QUAD: i32 = 1;
/// `shape` field: a capsule drawn along local x (`size` = half length, `half_y` = radius).
pub const DRAW_CAPSULE: i32 = 2;

fn q8(v: FP) -> i32 {
    (v.raw() >> 8) as i32
}

/// Drawing data of a frame, [`PHYS_STRIDE`] integers per body:
/// `shape` (`DRAW_*`), `class` (0 dynamic, 1 static, 2 bar, `3 + slot` paddle),
/// `x`, `y` (1/256 world units), `angle` (1/256 rad), `size`, `half_y` (1/256 units),
/// `speed` (1/32 units per second, at most 255).
pub fn render_phys(frame: &Frame) -> Vec<i32> {
    let (entities, bodies) = frame.dense::<Body>();
    let mut out = Vec::with_capacity(entities.len() * PHYS_STRIDE);
    for (&e, body) in entities.iter().zip(bodies) {
        let Some(collider) = frame.get::<Collider>(e) else { continue };
        let class = match body.kind {
            BODY_DYNAMIC => 0,
            BODY_KINEMATIC => frame.get::<PaddleTag>(e).map_or(2, |t| 3 + t.slot as i32),
            _ => 1,
        };
        let shape = &collider.shape;
        let (kind, size, half_y, turn) = if shape.kind == SHAPE_CIRCLE {
            (DRAW_CIRCLE, shape.radius, FP::ZERO, FP::ZERO)
        } else if shape.kind == SHAPE_CAPSULE {
            let d = shape.verts[1] - shape.verts[0];
            (DRAW_CAPSULE, d.length() / 2, shape.radius, d.y.atan2(d.x))
        } else {
            let (mut hx, mut hy) = (FP::ZERO, FP::ZERO);
            for v in &shape.verts[..shape.count as usize] {
                hx = hx.max(v.x.abs());
                hy = hy.max(v.y.abs());
            }
            (DRAW_QUAD, hx, hy, FP::ZERO)
        };
        let speed = (body.vel.x.raw().abs().max(body.vel.y.raw().abs()) >> 11).min(255) as i32;
        out.extend_from_slice(&[kind, class, q8(body.pos.x), q8(body.pos.y), q8(body.angle + turn), q8(size), q8(half_y), speed]);
    }
    out
}

/// The box of a scene: half width and height in world units.
pub fn scene_box(config: &PhysConfig) -> [i32; 2] {
    let l = layout(config.bodies);
    [l.half_w.to_int(), l.height.to_int()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_games::physics_game::{PhysGame, SceneMode};
    use orr_sim::Simulation;

    #[test]
    fn a_fresh_scene_draws_every_body() {
        let cfg = PhysConfig::new(60, SceneMode::Pile);
        let mut sim = Simulation::<PhysGame>::new(cfg, 60, 1);
        let drawn = render_phys(sim.frame_mut());
        assert_eq!(drawn.len() % PHYS_STRIDE, 0);
        let bodies = drawn.len() / PHYS_STRIDE;
        assert!(bodies >= 60 + 3, "{bodies} bodies drawn");
        let class = |i: usize| drawn[i * PHYS_STRIDE + 1];
        assert!((0..bodies).any(|i| class(i) == 0), "dynamic bodies");
        assert!((0..bodies).any(|i| class(i) == 1), "static walls");
        assert!((0..bodies).any(|i| class(i) == 3) && (0..bodies).any(|i| class(i) == 4), "both paddles");
        assert!((0..bodies).all(|i| (0..=2).contains(&drawn[i * PHYS_STRIDE]) && (0..=255).contains(&drawn[i * PHYS_STRIDE + 7])));
        let [w, h] = scene_box(&cfg);
        assert!(w > 0 && h > 0);
    }
}
