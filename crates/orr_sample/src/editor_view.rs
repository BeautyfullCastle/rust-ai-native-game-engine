//! Game-independent 2D geometry shared by editor view adapters.

use orr_ecs::Entity;
use orr_view::Style;

/// The collider outline of a body in its own frame (local to the body pose).
#[derive(Clone, Debug, PartialEq)]
pub enum Outline {
    /// A disc around the origin.
    Circle {
        /// Radius.
        radius: f32,
    },
    /// A segment `a..b` with a radius.
    Capsule {
        /// First end.
        a: [f32; 2],
        /// Second end.
        b: [f32; 2],
        /// Radius.
        radius: f32,
    },
    /// A convex polygon, counter-clockwise.
    Polygon(Vec<[f32; 2]>),
}

fn rot(v: [f32; 2], angle: f32) -> [f32; 2] {
    let (s, c) = angle.sin_cos();
    [c * v[0] - s * v[1], s * v[0] + c * v[1]]
}

impl Outline {
    /// Is the point (in the body's own frame) inside the shape?
    pub fn contains(&self, local: [f32; 2]) -> bool {
        match self {
            Outline::Circle { radius } => local[0].hypot(local[1]) <= *radius,
            Outline::Capsule { a, b, radius } => {
                let (abx, aby) = (b[0] - a[0], b[1] - a[1]);
                let len2 = abx * abx + aby * aby;
                let t = if len2 > 0.0 { (((local[0] - a[0]) * abx + (local[1] - a[1]) * aby) / len2).clamp(0.0, 1.0) } else { 0.0 };
                let (cx, cy) = (a[0] + abx * t, a[1] + aby * t);
                (local[0] - cx).hypot(local[1] - cy) <= *radius
            }
            Outline::Polygon(verts) => {
                let n = verts.len();
                (0..n).all(|i| {
                    let (a, b) = (verts[i], verts[(i + 1) % n]);
                    (b[0] - a[0]) * (local[1] - a[1]) - (b[1] - a[1]) * (local[0] - a[0]) >= 0.0
                })
            }
        }
    }

    /// Largest distance of the shape from its origin (a bounding radius).
    pub fn extent(&self) -> f32 {
        match self {
            Outline::Circle { radius } => *radius,
            Outline::Capsule { a, b, radius } => a.iter().chain(b.iter()).fold(0.0f32, |m, c| m.max(c.abs())) + radius,
            Outline::Polygon(verts) => verts.iter().map(|v| v[0].hypot(v[1])).fold(0.0f32, f32::max),
        }
    }
}

/// One drawable of a game frame: pose, draw style and hit/outline geometry.
#[derive(Clone, Debug)]
pub struct Drawable {
    /// The frame entity.
    pub entity: Entity,
    /// World position.
    pub pos: [f32; 2],
    /// Body angle in radians.
    pub angle: f32,
    /// How the renderer draws it (see [`crate::physics_view::body_look`]).
    pub style: Style,
    /// Angle added to `angle` for the renderer (see [`crate::physics_view::style_of`]).
    pub turn: f32,
    /// The collider shape, for outlines and picking.
    pub outline: Outline,
}

impl Drawable {
    /// Is the world point inside the collider?
    pub fn hit(&self, world: [f32; 2]) -> bool {
        self.outline.contains(rot([world[0] - self.pos[0], world[1] - self.pos[1]], -self.angle))
    }
}
