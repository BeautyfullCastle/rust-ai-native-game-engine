//! Pod components and singletons that make up the physics state in a `Frame`.

use bytemuck::{Pod, Zeroable};
use orr_ecs::{Entity, FrameList};
use orr_fp::{fp, FPVec2, FP};

/// Maximum vertex count of a convex polygon shape.
pub const MAX_POLY_VERTS: usize = 8;

/// `Shape::kind` for a circle.
pub const SHAPE_CIRCLE: u32 = 0;
/// `Shape::kind` for a convex polygon (boxes included).
pub const SHAPE_POLYGON: u32 = 1;

/// `Shape::kind` for a capsule: a segment (`verts[0]` to `verts[1]`) grown by
/// `radius`.
pub const SHAPE_CAPSULE: u32 = 2;

/// `Body::kind`: never moves, infinite mass.
pub const BODY_STATIC: u32 = 0;
/// `Body::kind`: simulated (gravity, contacts).
pub const BODY_DYNAMIC: u32 = 1;
/// `Body::kind`: moved only by its own velocity, pushes dynamic bodies.
pub const BODY_KINEMATIC: u32 = 2;

/// `Collider::flags` bit: sensor (trigger). Detects overlaps, never collides.
pub const COLLIDER_SENSOR: u32 = 1;

/// A convex collision shape in body-local space (origin = center of mass).
///
/// A capsule stores its segment end points in `verts[0]` and `verts[1]`
/// (`count == 2`), the unit axis `verts[0] -> verts[1]` in `normals[0]` and
/// the segment length in `normals[1].x`.
///
/// Polygon vertices are counter-clockwise; `normals[i]` is the outward unit
/// normal of the edge `verts[i] -> verts[i + 1]`. Both are stored so
/// contact generation never has to recompute them.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Shape {
    /// [`SHAPE_CIRCLE`], [`SHAPE_POLYGON`] or [`SHAPE_CAPSULE`].
    pub kind: u32,
    /// Number of used entries in `verts` / `normals` (0 for a circle, 2 for
    /// a capsule).
    pub count: u32,
    /// Circle or capsule radius (0 for polygons).
    pub radius: FP,
    /// Local vertices, CCW, centered on the centroid.
    pub verts: [FPVec2; MAX_POLY_VERTS],
    /// Local outward edge normals.
    pub normals: [FPVec2; MAX_POLY_VERTS],
}

/// Mass and rotational inertia about the center of mass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MassData {
    /// Mass (`density * area`).
    pub mass: FP,
    /// Moment of inertia about the center of mass.
    pub inertia: FP,
}

fn edge_normal(a: FPVec2, b: FPVec2) -> FPVec2 {
    let e = b - a;
    FPVec2::new(e.y, -e.x).normalize_or_zero()
}

impl Shape {
    /// A circle of radius `r`.
    pub fn circle(r: FP) -> Shape {
        let mut s = Shape::zeroed();
        s.kind = SHAPE_CIRCLE;
        s.radius = r;
        s
    }

    /// An upright capsule: the segment runs along the local y axis from
    /// `-half_length` to `+half_length`, grown by `radius`. Rotate it with
    /// `Body::angle`. `half_length == 0` gives a circle. The overall height
    /// is `2 * (half_length + radius)`.
    pub fn capsule(half_length: FP, radius: FP) -> Shape {
        Shape::capsule_segment(FPVec2::new(FP::ZERO, -half_length), FPVec2::new(FP::ZERO, half_length), radius)
    }

    /// A capsule around the segment `a`-`b`, grown by `radius`. The points
    /// are re-centered on the segment midpoint, so the body origin is the
    /// center of mass. Two equal points give a circle.
    pub fn capsule_segment(a: FPVec2, b: FPVec2, radius: FP) -> Shape {
        let d = b - a;
        let len = d.length();
        if len == FP::ZERO {
            return Shape::circle(radius);
        }
        let mid = (a + b) * FP::HALF;
        let mut s = Shape::zeroed();
        s.kind = SHAPE_CAPSULE;
        s.count = 2;
        s.radius = radius;
        s.verts[0] = a - mid;
        s.verts[1] = b - mid;
        s.normals[0] = d / len;
        s.normals[1] = FPVec2::new(len, FP::ZERO);
        s
    }

    /// A box with half extents `hx`, `hy` centered on the body origin. With
    /// fixed rotation and `angle == 0` this is an AABB; otherwise an OBB.
    pub fn box_shape(hx: FP, hy: FP) -> Shape {
        Shape::polygon(&[
            FPVec2::new(-hx, -hy),
            FPVec2::new(hx, -hy),
            FPVec2::new(hx, hy),
            FPVec2::new(-hx, hy),
        ])
        .expect("box half extents must be positive")
    }

    /// Alias of [`Shape::box_shape`] (axis-aligned use).
    pub fn aabb(hx: FP, hy: FP) -> Shape {
        Shape::box_shape(hx, hy)
    }

    /// A convex polygon from CCW vertices (3 to [`MAX_POLY_VERTS`]).
    /// Returns `None` for a non-convex, clockwise, degenerate or oversize
    /// input. Vertices are re-centered on the polygon centroid, so the
    /// body origin is the center of mass.
    pub fn polygon(pts: &[FPVec2]) -> Option<Shape> {
        let n = pts.len();
        if !(3..=MAX_POLY_VERTS).contains(&n) {
            return None;
        }
        for i in 0..n {
            let e1 = pts[(i + 1) % n] - pts[i];
            let e2 = pts[(i + 2) % n] - pts[(i + 1) % n];
            if e1.perp_dot(e2) <= FP::ZERO {
                return None;
            }
        }
        // Centroid via triangle fan from pts[0].
        let mut area2 = FP::ZERO;
        let mut c = FPVec2::ZERO;
        for i in 1..n - 1 {
            let e1 = pts[i] - pts[0];
            let e2 = pts[i + 1] - pts[0];
            let a = e1.perp_dot(e2);
            area2 += a;
            c += (e1 + e2) * a;
        }
        if area2 <= FP::ZERO {
            return None;
        }
        let centroid = pts[0] + c / (area2 * 3);
        let mut s = Shape::zeroed();
        s.kind = SHAPE_POLYGON;
        s.count = n as u32;
        for (dst, p) in s.verts.iter_mut().zip(pts) {
            *dst = *p - centroid;
        }
        for i in 0..n {
            s.normals[i] = edge_normal(s.verts[i], s.verts[(i + 1) % n]);
        }
        Some(s)
    }

    /// Mass and inertia about the center of mass for a uniform `density`.
    pub fn mass_data(&self, density: FP) -> MassData {
        if self.kind == SHAPE_CIRCLE {
            let mass = density * FP::PI * self.radius * self.radius;
            return MassData { mass, inertia: mass * self.radius * self.radius / 2 };
        }
        if self.kind == SHAPE_CAPSULE {
            // Rectangle `len x 2r` plus a disc, with the parallel axis terms
            // of the two half discs (centroid 4r / 3pi from the flat side).
            let r = self.radius;
            let len = self.normals[1].x;
            let m_rect = density * len * r * 2;
            let m_disc = density * FP::PI * r * r;
            let k = fp!(0.4244131815783876); // 4 / (3 pi)
            let i_rect = m_rect * (len * len + r * r * 4) / 12;
            let i_disc = m_disc * (r * r / 2 + len * len / 4 + len * r * k);
            return MassData { mass: m_rect + m_disc, inertia: i_rect + i_disc };
        }
        let n = self.count as usize;
        let mut area2 = FP::ZERO;
        let mut inertia12 = FP::ZERO;
        for i in 0..n {
            let a = self.verts[i];
            let b = self.verts[(i + 1) % n];
            let cr = a.perp_dot(b);
            area2 += cr;
            inertia12 += cr * (a.dot(a) + a.dot(b) + b.dot(b));
        }
        MassData { mass: density * area2 / 2, inertia: density * inertia12 / 12 }
    }

    /// Farthest extent from the origin (bounding radius).
    pub fn bounding_radius(&self) -> FP {
        if self.kind == SHAPE_CIRCLE {
            return self.radius;
        }
        if self.kind == SHAPE_CAPSULE {
            return self.verts[0].length_sq().max(self.verts[1].length_sq()).sqrt() + self.radius;
        }
        let mut m = FP::ZERO;
        for i in 0..self.count as usize {
            m = m.max(self.verts[i].length_sq());
        }
        m.sqrt()
    }
}

/// Dynamics state of one body. Attach together with a [`Collider`].
///
/// `inv_mass` / `inv_inertia` are ignored (treated as 0) unless
/// `kind == BODY_DYNAMIC`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Body {
    /// World position of the center of mass.
    pub pos: FPVec2,
    /// Rotation in radians, kept in `(-pi, pi]` by the integrator.
    pub angle: FP,
    /// Linear velocity.
    pub vel: FPVec2,
    /// Angular velocity (rad/s).
    pub omega: FP,
    /// Inverse mass (0 = immovable).
    pub inv_mass: FP,
    /// Inverse rotational inertia (0 = fixed rotation).
    pub inv_inertia: FP,
    /// Per-second linear velocity damping (0 = none).
    pub linear_damping: FP,
    /// Per-second angular velocity damping (0 = none).
    pub angular_damping: FP,
    /// [`BODY_STATIC`], [`BODY_DYNAMIC`] or [`BODY_KINEMATIC`].
    pub kind: u32,
    /// Sleep state of a dynamic body. Bit 31 ([`SLEEP_FLAG`]) is set while
    /// the body sleeps. The low bits count consecutive ticks below the
    /// sleep speed limits, capped at `PhysicsConfig::sleep_ticks`. Use
    /// [`crate::wake`] and [`crate::is_asleep`] instead of touching it.
    pub sleep: u32,
    /// While a body sleeps: the id of its sleep island (lowest entity
    /// index of the group plus one). Waking one member wakes every body
    /// with the same id. 0 for awake bodies.
    pub island: u32,
    /// Explicit padding, keep 0.
    pub _pad: u32,
}

/// [`Body::sleep`] bit that marks a sleeping body.
pub const SLEEP_FLAG: u32 = 1 << 31;

impl Body {
    /// A static body at `pos` with rotation `angle`.
    pub fn new_static(pos: FPVec2, angle: FP) -> Body {
        Body { pos, angle, kind: BODY_STATIC, ..Body::zeroed() }
    }

    /// A kinematic body moving with `vel` / `omega` (set the fields after).
    pub fn new_kinematic(pos: FPVec2) -> Body {
        Body { pos, kind: BODY_KINEMATIC, ..Body::zeroed() }
    }

    /// A dynamic body whose mass comes from `shape` and `density`.
    pub fn new_dynamic(pos: FPVec2, shape: &Shape, density: FP) -> Body {
        let m = shape.mass_data(density);
        let inv_mass = if m.mass > FP::ZERO { FP::ONE / m.mass } else { FP::ZERO };
        let inv_inertia = if m.inertia > FP::ZERO { FP::ONE / m.inertia } else { FP::ZERO };
        Body { pos, kind: BODY_DYNAMIC, inv_mass, inv_inertia, ..Body::zeroed() }
    }

    /// Same body with rotation locked.
    pub fn with_fixed_rotation(mut self) -> Body {
        self.inv_inertia = FP::ZERO;
        self
    }

    /// Same body with initial velocity.
    pub fn with_velocity(mut self, vel: FPVec2) -> Body {
        self.vel = vel;
        self
    }

    /// Same body with initial angle.
    pub fn with_angle(mut self, angle: FP) -> Body {
        self.angle = angle;
        self
    }
}

/// Collision shape and material of one body.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Collider {
    /// Local shape.
    pub shape: Shape,
    /// Bounciness in `[0, 1]`. Pairs use the larger value.
    pub restitution: FP,
    /// Coulomb friction coefficient. Pairs use the geometric mean.
    pub friction: FP,
    /// Layer bits of this collider.
    pub layer: u32,
    /// Layers this collider can touch. A pair collides only if each one's
    /// `layer` intersects the other's `mask`.
    pub mask: u32,
    /// [`COLLIDER_SENSOR`] bit.
    pub flags: u32,
    /// Explicit padding, keep 0.
    pub _pad: u32,
}

impl Collider {
    /// A solid collider: friction 0.5, restitution 0, layer 1, mask all.
    pub fn new(shape: Shape) -> Collider {
        Collider {
            shape,
            restitution: FP::ZERO,
            friction: fp!(0.5),
            layer: 1,
            mask: u32::MAX,
            flags: 0,
            _pad: 0,
        }
    }
    /// Same collider with restitution `e`.
    pub fn with_restitution(mut self, e: FP) -> Collider {
        self.restitution = e;
        self
    }
    /// Same collider with friction `f`.
    pub fn with_friction(mut self, f: FP) -> Collider {
        self.friction = f;
        self
    }
    /// Same collider as a sensor (trigger).
    pub fn sensor(mut self) -> Collider {
        self.flags |= COLLIDER_SENSOR;
        self
    }
    /// Same collider with layer and mask bits.
    pub fn with_filter(mut self, layer: u32, mask: u32) -> Collider {
        self.layer = layer;
        self.mask = mask;
        self
    }
    /// True if this collider is a sensor.
    pub fn is_sensor(&self) -> bool {
        self.flags & COLLIDER_SENSOR != 0
    }
}

/// Tunable solver settings. Part of the frame state (checksummed), so all
/// peers must start from the same values.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct PhysicsConfig {
    /// Gravity acceleration.
    pub gravity: FPVec2,
    /// Fixed time step in seconds.
    pub dt: FP,
    /// Position error correction factor (Baumgarte), `0.1..0.3`.
    pub baumgarte: FP,
    /// Penetration tolerated without correction.
    pub linear_slop: FP,
    /// Contacts are created up to this distance (speculative margin).
    pub contact_margin: FP,
    /// Closing speed below which restitution is ignored.
    pub restitution_threshold: FP,
    /// Cap on the velocity added by position correction.
    pub max_correction_speed: FP,
    /// Per-axis speed clamp for dynamic bodies.
    pub max_linear_speed: FP,
    /// Angular speed clamp for dynamic bodies.
    pub max_angular_speed: FP,
    /// Sequential impulse iterations per tick.
    pub velocity_iterations: u32,
    /// A group of touching bodies falls asleep after every body in it has
    /// been slower than the two sleep speeds for this many ticks. 0 turns
    /// sleeping off.
    pub sleep_ticks: u32,
    /// Sleep limit for the linear speed.
    pub sleep_linear_speed: FP,
    /// Sleep limit for the angular speed (rad/s).
    pub sleep_angular_speed: FP,
}

impl Default for PhysicsConfig {
    fn default() -> Self {
        PhysicsConfig {
            gravity: FPVec2::new(FP::ZERO, fp!(-10)),
            dt: FP::from_ratio(1, 60),
            baumgarte: fp!(0.2),
            linear_slop: fp!(0.01),
            contact_margin: fp!(0.02),
            restitution_threshold: fp!(1),
            max_correction_speed: fp!(6),
            max_linear_speed: fp!(500),
            max_angular_speed: fp!(60),
            velocity_iterations: 8,
            sleep_ticks: 30,
            sleep_linear_speed: fp!(0.05),
            sleep_angular_speed: fp!(0.05),
        }
    }
}

/// One warm-starting entry: the impulses a contact point ended the last
/// tick with. Sorted by `(a, b, id)` inside [`PhysicsState::contacts`].
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct ContactCache {
    /// Entity index of the lower-indexed body.
    pub a: u32,
    /// Entity index of the higher-indexed body.
    pub b: u32,
    /// Feature id of the contact point (stable across ticks).
    pub id: u32,
    /// Explicit padding.
    pub _pad: u32,
    /// Accumulated normal impulse.
    pub normal_impulse: FP,
    /// Accumulated friction impulse.
    pub tangent_impulse: FP,
}

/// One overlapping trigger pair, sorted by entity index in
/// [`PhysicsState::overlaps`].
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct OverlapPair {
    /// Lower-indexed entity.
    pub a: Entity,
    /// Higher-indexed entity.
    pub b: Entity,
}

/// [`TriggerEvent::kind`] when a pair starts overlapping.
pub const TRIGGER_ENTER: u32 = 1;
/// [`TriggerEvent::kind`] when a pair stops overlapping (or one entity was
/// despawned).
pub const TRIGGER_EXIT: u32 = 2;

/// A sensor overlap started or ended.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct TriggerEvent {
    /// Lower-indexed entity of the pair.
    pub a: Entity,
    /// Higher-indexed entity of the pair.
    pub b: Entity,
    /// [`TRIGGER_ENTER`] or [`TRIGGER_EXIT`].
    pub kind: u32,
    /// Explicit padding.
    pub _pad: u32,
}

/// Singleton holding the config and the persistent (rollback-safe) caches.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct PhysicsState {
    /// Solver settings.
    pub config: PhysicsConfig,
    /// Warm-starting cache handle (`FrameList<ContactCache>`).
    pub contacts: FrameList<ContactCache>,
    /// Trigger overlap set handle (`FrameList<OverlapPair>`).
    pub overlaps: FrameList<OverlapPair>,
}
