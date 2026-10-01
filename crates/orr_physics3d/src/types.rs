//! Pod components and singletons that make up the 3D physics state in a
//! `Frame`.

use bytemuck::{Pod, Zeroable};
use orr_ecs::FrameList;
use orr_fp::{fp, FPQuat, FPVec3, FP};

/// `Shape::kind` for a sphere.
pub const SHAPE_SPHERE: u32 = 0;
/// `Shape::kind` for a capsule (segment along the local y axis, grown by
/// `radius`).
pub const SHAPE_CAPSULE: u32 = 1;
/// `Shape::kind` for a box (OBB).
pub const SHAPE_BOX: u32 = 2;

/// `Body::kind`: never moves, infinite mass.
pub const BODY_STATIC: u32 = 0;
/// `Body::kind`: simulated (gravity, contacts).
pub const BODY_DYNAMIC: u32 = 1;
/// `Body::kind`: moved only by its own velocity, pushes dynamic bodies.
pub const BODY_KINEMATIC: u32 = 2;

/// [`Body::sleep`] bit that marks a sleeping body.
pub const SLEEP_FLAG: u32 = 1 << 31;

/// A convex collision shape in body-local space (origin = center of mass).
///
/// - sphere: `radius`.
/// - capsule: the segment runs along the local y axis from
///   `-half.y` to `+half.y`, grown by `radius` (`half.x`, `half.z` are 0).
/// - box: half extents `half`, `radius` is 0.
///
/// Kinds are ordered sphere < capsule < box; the narrow phase relies on it.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Shape {
    /// [`SHAPE_SPHERE`], [`SHAPE_CAPSULE`] or [`SHAPE_BOX`].
    pub kind: u32,
    /// Explicit padding, keep 0.
    pub _pad: u32,
    /// Sphere or capsule radius (0 for boxes).
    pub radius: FP,
    /// Box half extents, or `(0, half_length, 0)` for a capsule.
    pub half: FPVec3,
}

/// Mass and the diagonal of the inertia tensor about the center of mass,
/// in the body frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MassData {
    /// Mass (`density * volume`).
    pub mass: FP,
    /// Principal moments of inertia (body frame; every shape here is
    /// diagonal about its center).
    pub inertia: FPVec3,
}

impl Shape {
    /// A sphere of radius `r`.
    pub fn sphere(r: FP) -> Shape {
        Shape { kind: SHAPE_SPHERE, _pad: 0, radius: r, half: FPVec3::ZERO }
    }

    /// A capsule along the local y axis: segment half length
    /// `half_length`, radius `radius`. Total height is
    /// `2 * (half_length + radius)`. `half_length == 0` gives a sphere.
    pub fn capsule(half_length: FP, radius: FP) -> Shape {
        if half_length <= FP::ZERO {
            return Shape::sphere(radius);
        }
        Shape { kind: SHAPE_CAPSULE, _pad: 0, radius, half: FPVec3::new(FP::ZERO, half_length, FP::ZERO) }
    }

    /// A box with half extents `hx`, `hy`, `hz`.
    pub fn cuboid(hx: FP, hy: FP, hz: FP) -> Shape {
        Shape { kind: SHAPE_BOX, _pad: 0, radius: FP::ZERO, half: FPVec3::new(hx, hy, hz) }
    }

    /// Mass and principal inertia for a uniform `density`.
    pub fn mass_data(&self, density: FP) -> MassData {
        let r = self.radius;
        let r2 = r * r;
        match self.kind {
            SHAPE_SPHERE => {
                let mass = density * FP::PI * r2 * r * 4 / 3;
                let i = mass * r2 * 2 / 5;
                MassData { mass, inertia: FPVec3::splat(i) }
            }
            SHAPE_CAPSULE => {
                let h = self.half.y;
                let len = h * 2;
                let m_cyl = density * FP::PI * r2 * len;
                let m_sph = density * FP::PI * r2 * r * 4 / 3;
                let iy = m_cyl * r2 / 2 + m_sph * r2 * 2 / 5;
                let ix = m_cyl * (r2 * 3 + len * len) / 12 + m_sph * (r2 * 2 / 5 + h * h + h * r * fp!(0.75));
                MassData { mass: m_cyl + m_sph, inertia: FPVec3::new(ix, iy, ix) }
            }
            _ => {
                let h = self.half;
                let mass = density * h.x * h.y * h.z * 8;
                let (x2, y2, z2) = (h.x * h.x * 4, h.y * h.y * 4, h.z * h.z * 4);
                MassData {
                    mass,
                    inertia: FPVec3::new(mass * (y2 + z2) / 12, mass * (x2 + z2) / 12, mass * (x2 + y2) / 12),
                }
            }
        }
    }

    /// Farthest extent from the origin (bounding radius).
    pub fn bounding_radius(&self) -> FP {
        match self.kind {
            SHAPE_SPHERE => self.radius,
            SHAPE_CAPSULE => self.half.y + self.radius,
            _ => self.half.length(),
        }
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
    pub pos: FPVec3,
    /// Orientation (unit quaternion, renormalized by the integrator).
    pub rot: FPQuat,
    /// Linear velocity.
    pub vel: FPVec3,
    /// Angular velocity (world frame, rad/s).
    pub omega: FPVec3,
    /// Inverse mass (0 = immovable).
    pub inv_mass: FP,
    /// Inverse principal moments of inertia in the body frame (0 = that
    /// axis is locked).
    pub inv_inertia: FPVec3,
    /// Per-second linear velocity damping (0 = none).
    pub linear_damping: FP,
    /// Per-second angular velocity damping (0 = none).
    pub angular_damping: FP,
    /// [`BODY_STATIC`], [`BODY_DYNAMIC`] or [`BODY_KINEMATIC`].
    pub kind: u32,
    /// Sleep state of a dynamic body. Bit 31 ([`SLEEP_FLAG`]) is set while
    /// the body sleeps; the low bits count consecutive still ticks. Use
    /// [`crate::wake`] and [`crate::is_asleep`] instead of touching it.
    pub sleep: u32,
    /// While a body sleeps: the id of its sleep island (lowest entity
    /// index of the group plus one). 0 for awake bodies.
    pub island: u32,
    /// Explicit padding, keep 0.
    pub _pad: u32,
}

impl Body {
    /// A static body at `pos`.
    pub fn new_static(pos: FPVec3) -> Body {
        Body { pos, rot: FPQuat::IDENTITY, kind: BODY_STATIC, ..Body::zeroed() }
    }

    /// A kinematic body (set `vel` / `omega` after).
    pub fn new_kinematic(pos: FPVec3) -> Body {
        Body { pos, rot: FPQuat::IDENTITY, kind: BODY_KINEMATIC, ..Body::zeroed() }
    }

    /// A dynamic body whose mass and inertia come from `shape` and
    /// `density`.
    pub fn new_dynamic(pos: FPVec3, shape: &Shape, density: FP) -> Body {
        let m = shape.mass_data(density);
        let inv = |v: FP| if v > FP::ZERO { FP::ONE / v } else { FP::ZERO };
        Body {
            pos,
            rot: FPQuat::IDENTITY,
            kind: BODY_DYNAMIC,
            inv_mass: inv(m.mass),
            inv_inertia: FPVec3::new(inv(m.inertia.x), inv(m.inertia.y), inv(m.inertia.z)),
            ..Body::zeroed()
        }
    }

    /// Same body with rotation locked (all three axes).
    pub fn with_fixed_rotation(mut self) -> Body {
        self.inv_inertia = FPVec3::ZERO;
        self
    }

    /// Same body with initial linear velocity.
    pub fn with_velocity(mut self, vel: FPVec3) -> Body {
        self.vel = vel;
        self
    }

    /// Same body with initial angular velocity.
    pub fn with_omega(mut self, omega: FPVec3) -> Body {
        self.omega = omega;
        self
    }

    /// Same body with initial orientation.
    pub fn with_rotation(mut self, rot: FPQuat) -> Body {
        self.rot = rot;
        self
    }

    /// Same body with damping coefficients (per second).
    pub fn with_damping(mut self, linear: FP, angular: FP) -> Body {
        self.linear_damping = linear;
        self.angular_damping = angular;
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
    /// Reserved flags, keep 0.
    pub flags: u32,
    /// Explicit padding, keep 0.
    pub _pad: u32,
}

impl Collider {
    /// A solid collider: friction 0.5, restitution 0, layer 1, mask all.
    pub fn new(shape: Shape) -> Collider {
        Collider { shape, restitution: FP::ZERO, friction: fp!(0.5), layer: 1, mask: u32::MAX, flags: 0, _pad: 0 }
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
    /// Same collider with layer and mask bits.
    pub fn with_filter(mut self, layer: u32, mask: u32) -> Collider {
        self.layer = layer;
        self.mask = mask;
        self
    }
}

/// Tunable solver settings. Part of the frame state (checksummed), so all
/// peers must start from the same values.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct PhysicsConfig {
    /// Gravity acceleration.
    pub gravity: FPVec3,
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
    /// Per-axis angular speed clamp for dynamic bodies.
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
            gravity: FPVec3::new(FP::ZERO, fp!(-10), FP::ZERO),
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
    /// Accumulated friction impulse, as a world-space vector (projected
    /// onto the new tangent basis when it is reused).
    pub tangent_impulse: FPVec3,
}

/// Singleton holding the config and the persistent (rollback-safe) cache.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct PhysicsState {
    /// Solver settings.
    pub config: PhysicsConfig,
    /// Warm-starting cache handle (`FrameList<ContactCache>`).
    pub contacts: FrameList<ContactCache>,
}
