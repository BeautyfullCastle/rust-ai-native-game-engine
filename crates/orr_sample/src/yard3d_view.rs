//! The Yard3D scene seen by the view layer: which bodies to draw and how,
//! the camera and light, debug lines, and the mapping from mouse and keys to
//! the sim input. No GPU code here; `yard3d_app` owns the window.

use orr_bridge::FrameView;
use orr_ecs::Entity;
use orr_physics3d::{Body, Collider, BODY_DYNAMIC, SHAPE_BOX, SHAPE_CAPSULE, SLEEP_FLAG};
use orr_render::math3::{quat_rotate, Vec3 as A3};
use orr_render::{Camera3D, Lighting, Material, OrbitCamera, RenderList3D};
use orr_view::{fp_to_f32, fp_to_transform3, fp_to_vec3, Extracted3, Extractor3, InterpMode, RenderItem3, Shape3, Style3};

use crate::yard3d_game::{YardInput, SHOOT, SPAWN_BALL, SPAWN_BOX, SPAWN_CAPSULE};

const WALL_COLOR: [f32; 3] = [0.20, 0.24, 0.34];
const GROUND_COLOR: [f32; 3] = [0.30, 0.34, 0.28];
const SPHERE_COLOR: [f32; 3] = [0.05, 0.50, 0.48];
const BOX_COLOR: [f32; 3] = [0.80, 0.26, 0.05];
const CAPSULE_COLOR: [f32; 3] = [0.50, 0.12, 0.70];
/// Speed (units per second) at which a body is shown at full brightness.
const FULL_BRIGHT_SPEED: f32 = 6.0;

/// Reads bodies out of a Yard3D frame. Dynamic bodies are predicted (rollback
/// corrections are smoothed away); the floor, walls and ramp never move.
#[derive(Clone, Copy, Default)]
pub struct YardExtractor;

impl Extractor3 for YardExtractor {
    fn extract(&self, frame: FrameView<'_>, out: &mut Vec<Extracted3>) {
        out.reserve(frame.count::<Body>() as usize);
        for (entity, body) in frame.iter::<Body>() {
            let Some(collider) = frame.get::<Collider>(entity) else { continue };
            out.push(extract_body(entity, body, collider));
        }
    }
}

/// Authoritative body poses for presentation bindings. Unlike the decorative
/// ground plane mapping, these retain the exact body's center. No write-back.
pub fn editor_body_poses(frame: FrameView<'_>) -> Vec<(Entity, orr_view::Transform3)> {
    frame.iter::<Body>().map(|(entity, body)| (entity, fp_to_transform3(body.pos, body.rot))).collect()
}

/// Read-only classification for static presentation baking. Classification,
/// exact body placement, and the rendered procedural style all come from the
/// same immutable frame; interpolation policy is deliberately not a body kind.
#[derive(Clone, Copy, Debug)]
pub struct EditorBakeBody {
    pub entity: Entity,
    pub is_static: bool,
    pub pose: orr_view::Transform3,
    pub item: Option<RenderItem3>,
    pub supported_shape: bool,
}

pub fn editor_bake_bodies(frame: FrameView<'_>) -> Result<Vec<EditorBakeBody>, String> {
    if frame.count::<Body>() > 65536 {
        return Err("Static diffuse bake exceeds 65536 classified bodies".into());
    }
    Ok(frame.iter::<Body>().map(|(entity, body)| {
        let collider = frame.get::<Collider>(entity);
        let item = collider.map(|collider| {
            let extracted = extract_body(entity, body, collider);
            RenderItem3 { entity, transform: extracted.transform, style: extracted.style }
        });
        EditorBakeBody {
            entity,
            is_static: body.kind == orr_physics3d::BODY_STATIC,
            pose: fp_to_transform3(body.pos, body.rot),
            item,
            supported_shape: collider.is_none_or(|collider| matches!(collider.shape.kind,
                orr_physics3d::SHAPE_BOX | orr_physics3d::SHAPE_SPHERE | orr_physics3d::SHAPE_CAPSULE)),
        }
    }).collect())
}

/// Allocation-free view of just the explicitly static bodies. Used by the
/// editor's bake cache key so Play ticks never copy excluded dynamic render
/// items or construct procedural meshes and texture buffers.
pub fn editor_static_bake_bodies<'a>(frame: FrameView<'a>) -> Result<impl Iterator<Item = EditorBakeBody> + 'a, String> {
    if frame.count::<Body>() > 65536 {
        return Err("Static diffuse bake exceeds 65536 classified bodies".into());
    }
    let (entities, bodies) = frame.dense::<Body>();
    Ok(entities.iter().copied().zip(bodies).filter(|(_, body)| body.kind == orr_physics3d::BODY_STATIC).map(move |(entity, body)| {
        let collider = frame.get::<Collider>(entity);
        EditorBakeBody {
            entity,
            is_static: true,
            pose: fp_to_transform3(body.pos, body.rot),
            item: collider.map(|collider| {
                let extracted = extract_body(entity, body, collider);
                RenderItem3 { entity, transform: extracted.transform, style: extracted.style }
            }),
            supported_shape: collider.is_none_or(|collider| matches!(collider.shape.kind,
                orr_physics3d::SHAPE_BOX | orr_physics3d::SHAPE_SPHERE | orr_physics3d::SHAPE_CAPSULE)),
        }
    }))
}

fn extract_body(entity: Entity, body: &Body, collider: &Collider) -> Extracted3 {
    let shape = &collider.shape;
    let mut transform = fp_to_transform3(body.pos, body.rot);
    let dynamic = body.kind == BODY_DYNAMIC;
    let (shape3, base, material) = match shape.kind {
        SHAPE_BOX => {
            let half = [fp_to_f32(shape.half.x), fp_to_f32(shape.half.y), fp_to_f32(shape.half.z)];
            if !dynamic && half[0] > 20.0 && half[2] > 20.0 {
                // The floor slab: draw its top face as a checkered plane.
                transform.pos.y += half[1];
                let style = Style3 {
                    shape: Shape3::Plane { half_x: half[0], half_z: half[2] },
                    color: [GROUND_COLOR[0], GROUND_COLOR[1], GROUND_COLOR[2], 1.0],
                    roughness: 0.95,
                    metallic: 0.0,
                    checker: true,
                };
                return Extracted3 { entity, transform, mode: InterpMode::None, style };
            }
            (Shape3::Box { half }, if dynamic { BOX_COLOR } else { WALL_COLOR }, (0.7, 0.0))
        }
        SHAPE_CAPSULE => (
            Shape3::Capsule { half_length: fp_to_f32(shape.half.y), radius: fp_to_f32(shape.radius) },
            CAPSULE_COLOR,
            (0.35, 0.45),
        ),
        _ => (Shape3::Sphere { radius: fp_to_f32(shape.radius) }, SPHERE_COLOR, (0.3, 0.1)),
    };
    let k = if dynamic {
        // Moving bodies are bright, resting ones dim, sleeping ones dimmest.
        let speed = fp_to_vec3(body.vel).length();
        let asleep = if body.sleep & SLEEP_FLAG != 0 { 0.85 } else { 1.0 };
        (0.62 + 0.38 * (speed / FULL_BRIGHT_SPEED).min(1.0)) * asleep
    } else {
        1.0
    };
    let style = Style3 {
        shape: shape3,
        color: [base[0] * k, base[1] * k, base[2] * k, 1.0],
        roughness: material.0,
        metallic: material.1,
        checker: false,
    };
    Extracted3 { entity, transform, mode: if dynamic { InterpMode::Prediction } else { InterpMode::None }, style }
}

/// Moves the view items into a render list (instances sorted by mesh kind by the list).
pub fn fill_list(items: &[RenderItem3], list: &mut RenderList3D) {
    for item in items {
        let (p, r) = (item.transform.pos.to_array(), item.transform.rot.to_array());
        let s = &item.style;
        let m = Material { color: [s.color[0], s.color[1], s.color[2]], roughness: s.roughness, metallic: s.metallic, emissive: 0.0, checker: s.checker };
        match s.shape {
            Shape3::Sphere { radius } => list.sphere(p, r, radius, &m),
            Shape3::Box { half } => list.cuboid(p, r, half, &m),
            Shape3::Capsule { half_length, radius } => list.capsule(p, r, half_length, radius, &m),
            Shape3::Plane { half_x, half_z } => list.plane(p, half_x, half_z, &m),
        }
    }
}

/// The sun and sky of the yard: low afternoon sun, a shadow map that covers the whole floor.
pub fn yard_lighting() -> Lighting {
    Lighting {
        direction: [-0.42, -0.72, -0.55],
        color: [1.0, 0.94, 0.82],
        intensity: 2.1,
        sky: [0.50, 0.66, 1.0],
        ground: [0.26, 0.24, 0.22],
        ambient: 0.30,
        shadows: true,
        shadow_center: [0.0, 0.0, 0.0],
        shadow_radius: 36.0,
        tonemap: true,
        exposure: 1.0,
    }
}

/// View settings for the yard: a correction longer than a few units cannot be a physical
/// rollback correction (bodies move a few units in a prediction window), it is a different
/// body that got the same entity index after a mispredicted spawn, so it is shown at once.
pub fn yard_view_config() -> orr_view::ViewConfig {
    orr_view::ViewConfig { snap_distance: 8.0, ..orr_view::ViewConfig::default() }
}

/// The orbit camera that frames the yard.
pub fn yard_camera() -> OrbitCamera {
    OrbitCamera::new([0.0, 1.0, 0.0], 0.55, 0.64, 40.0)
}

/// Debug lines of the newest frame: a box around every dynamic body (green
/// awake, blue asleep) and an arrow along the velocity of moving ones.
pub fn debug_lines(frame: FrameView<'_>, list: &mut RenderList3D) {
    for (entity, body) in frame.iter::<Body>() {
        if body.kind != BODY_DYNAMIC {
            continue;
        }
        let Some(collider) = frame.get::<Collider>(entity) else { continue };
        let pos = fp_to_vec3(body.pos).to_array();
        let rot = fp_to_transform3(body.pos, body.rot).rot.to_array();
        let s = &collider.shape;
        let r = fp_to_f32(s.radius);
        let half: A3 = match s.kind {
            SHAPE_BOX => {
                let h = [fp_to_f32(s.half.x), fp_to_f32(s.half.y), fp_to_f32(s.half.z)];
                let axes = [quat_rotate(rot, [1.0, 0.0, 0.0]), quat_rotate(rot, [0.0, 1.0, 0.0]), quat_rotate(rot, [0.0, 0.0, 1.0])];
                std::array::from_fn(|i| axes[0][i].abs() * h[0] + axes[1][i].abs() * h[1] + axes[2][i].abs() * h[2])
            }
            SHAPE_CAPSULE => {
                let up = quat_rotate(rot, [0.0, fp_to_f32(s.half.y), 0.0]);
                [up[0].abs() + r, up[1].abs() + r, up[2].abs() + r]
            }
            _ => [r; 3],
        };
        let asleep = body.sleep & SLEEP_FLAG != 0;
        let color = if asleep { [0.3, 0.5, 1.0, 0.8] } else { [0.2, 1.0, 0.3, 0.9] };
        list.aabb([pos[0] - half[0], pos[1] - half[1], pos[2] - half[2]], [pos[0] + half[0], pos[1] + half[1], pos[2] + half[2]], 1.0, color);
        let vel = fp_to_vec3(body.vel);
        if vel.length() > 0.5 {
            list.arrow(pos, [vel.x * 0.3, vel.y * 0.3, vel.z * 0.3], 2.0, [1.0, 0.85, 0.2, 1.0]);
        }
    }
}

/// Which spawn and shoot keys are held.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct YardKeys {
    pub shoot: bool,
    pub spawn_box: bool,
    pub spawn_ball: bool,
    pub spawn_capsule: bool,
}

impl YardKeys {
    pub fn buttons(self) -> u32 {
        (if self.shoot { SHOOT } else { 0 })
            | (if self.spawn_box { SPAWN_BOX } else { 0 })
            | (if self.spawn_ball { SPAWN_BALL } else { 0 })
            | (if self.spawn_capsule { SPAWN_CAPSULE } else { 0 })
    }
}

/// The sim input for held `keys` with the cursor at `cursor` (pixels) in a
/// `viewport` seen through `camera`: the camera ray as centimeters and
/// thousandths.
pub fn input_for(keys: YardKeys, camera: &Camera3D, cursor: [f32; 2], viewport: (u32, u32)) -> YardInput {
    let buttons = keys.buttons();
    if buttons == 0 {
        return YardInput::default();
    }
    let (o, d) = camera.screen_ray(cursor, viewport);
    let q = |v: f32, scale: f32| (v * scale).round().clamp(-20_000.0, 20_000.0) as i32;
    YardInput {
        buttons,
        _pad: 0,
        origin: [q(o[0], 100.0), q(o[1], 100.0), q(o[2], 100.0)],
        dir: [q(d[0], 1000.0), q(d[1], 1000.0), q(d[2], 1000.0)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bake_classification_uses_body_kind_and_exact_same_frame_mesh() {
        use orr_ecs::{ComponentRegistryBuilder, Frame};
        use orr_fp::{fp, FPVec3};
        use orr_physics3d::Shape;
        let mut registry = ComponentRegistryBuilder::new();
        orr_physics3d::register(&mut registry);
        let mut frame = Frame::new(registry.build());
        let shape = Shape::cuboid(fp!(24), fp!(0.5), fp!(24));
        let position = FPVec3::new(fp!(0), fp!(-0.5), fp!(0));
        for body in [Body::new_static(position), Body::new_kinematic(position),
            Body::new_dynamic(position, &shape, fp!(1))] {
            let entity = frame.spawn();
            frame.add(entity, body);
            frame.add(entity, Collider::new(shape));
        }
        let view = FrameView::of(&frame);
        let classified = editor_bake_bodies(view).unwrap();
        assert_eq!(classified.iter().filter(|body| body.is_static).count(), 1);
        let ground = classified.iter().find(|body| body.is_static).unwrap();
        assert_eq!(ground.pose.pos.y, -0.5);
        assert_eq!(ground.item.unwrap().transform.pos.y, 0.0);
        assert!(ground.item.unwrap().style.checker);
        let mut rendered = Vec::new();
        YardExtractor.extract(view, &mut rendered);
        for body in classified {
            let item = body.item.unwrap();
            let visible = rendered.iter().find(|item| item.entity == body.entity).unwrap();
            assert_eq!(item.transform, visible.transform);
            assert_eq!(item.style, visible.style);
        }
    }

    #[test]
    fn input_ray_round_trips_through_the_integer_encoding() {
        let cam = yard_camera().camera();
        let vp = (800u32, 600u32);
        let cursor = [500.0, 380.0];
        let input = input_for(YardKeys { shoot: true, ..Default::default() }, &cam, cursor, vp);
        assert_eq!(input.buttons, SHOOT);
        let o = input.origin.map(|c| c as f32 / 100.0);
        let d = input.dir.map(|c| c as f32 / 1000.0);
        let (o2, d2) = cam.screen_ray(cursor, vp);
        for i in 0..3 {
            assert!((o[i] - o2[i]).abs() < 0.01 && (d[i] - d2[i]).abs() < 0.002, "{o:?} {d:?} vs {o2:?} {d2:?}");
        }
        assert_eq!(input_for(YardKeys::default(), &cam, cursor, vp), YardInput::default());
    }
}
