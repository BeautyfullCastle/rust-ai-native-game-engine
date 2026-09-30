//! The physics scene seen by the view layer: which bodies to draw and how,
//! the local input mapping, and the camera.

use orr_bridge::FrameView;
use orr_ecs::Entity;
use orr_physics::{Body, Collider, BODY_DYNAMIC, BODY_KINEMATIC, SHAPE_CAPSULE, SHAPE_CIRCLE};
use orr_view::{fp_to_f32, fp_to_vec2, Extracted, Extractor, InterpMode, RenderItem, Shape, Style, Transform2, Vec2};

use crate::physics_game::{layout, PaddleTag, PhysInput};
use orr_render::Camera;

const PADDLE_COLORS: [[f32; 4]; 4] =
    [[0.25, 0.6, 1.0, 1.0], [1.0, 0.55, 0.2, 1.0], [0.4, 0.9, 0.4, 1.0], [0.95, 0.4, 0.75, 1.0]];
const STATIC_COLOR: [f32; 4] = [0.36, 0.38, 0.47, 1.0];
const BAR_COLOR: [f32; 4] = [0.72, 0.42, 0.86, 1.0];
const CIRCLE_COLOR: [f32; 3] = [0.25, 0.85, 0.75];
const BOX_COLOR: [f32; 3] = [0.95, 0.72, 0.28];
const CAPSULE_COLOR: [f32; 3] = [0.85, 0.42, 0.9];
/// Speed (world units per second) at which a body is drawn at full brightness.
const FULL_BRIGHT_SPEED: f32 = 8.0;

/// Reads bodies out of a physics frame. Dynamic bodies and the local paddle
/// are predicted (rollback corrections are smoothed away); walls and
/// obstacles never move; the remote paddle uses `remote_mode`.
pub struct PhysExtractor {
    pub remote_mode: InterpMode,
    /// Slot of the player at the keyboard (its paddle is predicted).
    pub local_slot: u8,
}

impl Extractor for PhysExtractor {
    fn extract(&self, frame: FrameView<'_>, out: &mut Vec<Extracted>) {
        out.reserve(frame.count::<Body>() as usize);
        for (entity, body) in frame.iter::<Body>() {
            let Some(collider) = frame.get::<Collider>(entity) else { continue };
            let (mode, color) = match body.kind {
                BODY_DYNAMIC => {
                    let speed = fp_to_vec2(body.vel).length();
                    let base = match collider.shape.kind {
                        SHAPE_CIRCLE => CIRCLE_COLOR,
                        SHAPE_CAPSULE => CAPSULE_COLOR,
                        _ => BOX_COLOR,
                    };
                    // Resting bodies are dim, moving ones bright.
                    let k = 0.4 + 0.6 * (speed / FULL_BRIGHT_SPEED).min(1.0);
                    (InterpMode::Prediction, [base[0] * k, base[1] * k, base[2] * k, 1.0])
                }
                BODY_KINEMATIC => match frame.get::<PaddleTag>(entity) {
                    Some(tag) => {
                        let mode = if tag.slot == u32::from(self.local_slot) { InterpMode::Prediction } else { self.remote_mode };
                        (mode, PADDLE_COLORS[tag.slot as usize % PADDLE_COLORS.len()])
                    }
                    None => (InterpMode::Prediction, BAR_COLOR),
                },
                _ => (InterpMode::None, STATIC_COLOR),
            };
            let (style, turn) = style_of(collider, color);
            let transform = Transform2::new(fp_to_vec2(body.pos), fp_to_f32(body.angle) + turn);
            out.push(Extracted { entity, transform, mode, style });
        }
    }
}

/// Draw shape of a collider: a circle, a capsule, or the local bounding box of a
/// polygon. The second value is an angle added to the body angle (a capsule's
/// segment need not lie along local x; the renderer draws it along x).
fn style_of(collider: &Collider, color: [f32; 4]) -> (Style, f32) {
    let shape = &collider.shape;
    if shape.kind == SHAPE_CIRCLE {
        return (Style { shape: Shape::Circle, size: fp_to_f32(shape.radius), half_y: 0.0, color }, 0.0);
    }
    if shape.kind == SHAPE_CAPSULE {
        let (a, b) = (fp_to_vec2(shape.verts[0]), fp_to_vec2(shape.verts[1]));
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let style = Style { shape: Shape::Capsule, size: dx.hypot(dy) * 0.5, half_y: fp_to_f32(shape.radius), color };
        return (style, dy.atan2(dx));
    }
    let (mut hx, mut hy) = (0.0f32, 0.0f32);
    for v in &shape.verts[..shape.count as usize] {
        hx = hx.max(fp_to_f32(v.x).abs());
        hy = hy.max(fp_to_f32(v.y).abs());
    }
    (Style { shape: Shape::Quad, size: hx, half_y: hy, color }, 0.0)
}

/// The dark rectangle of the box interior. Drawn first, not part of the sim.
pub fn scene_floor(bodies: u32) -> RenderItem {
    let l = layout(bodies);
    let (half_w, height) = (fp_to_f32(l.half_w), fp_to_f32(l.height));
    RenderItem {
        entity: Entity::NONE,
        transform: Transform2::new(Vec2::new(0.0, height / 2.0), 0.0),
        style: Style { shape: Shape::Quad, size: half_w, half_y: height / 2.0, color: [0.07, 0.07, 0.11, 1.0] },
    }
}

/// A camera that shows the whole box with a small margin.
pub fn scene_camera(bodies: u32) -> Camera {
    let l = layout(bodies);
    let (half_w, height) = (fp_to_f32(l.half_w), fp_to_f32(l.height));
    Camera::new([0.0, height / 2.0], half_w.max(height / 2.0) * 1.06)
}

/// The keys of the local player.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct PhysKeys {
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
    pub spin_left: bool,
    pub spin_right: bool,
    pub shoot: bool,
}

impl PhysKeys {
    pub fn to_input(self) -> PhysInput {
        let axis = |neg: bool, pos: bool| i32::from(pos) - i32::from(neg);
        PhysInput::new(
            axis(self.left, self.right),
            axis(self.down, self.up),
            axis(self.spin_right, self.spin_left),
            self.shoot,
        )
    }
}
