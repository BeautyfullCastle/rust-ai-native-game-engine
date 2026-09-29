use std::ops::{Add, AddAssign, Mul, Neg, Sub};

/// A 2D `f32` vector (view layer).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn length(self) -> f32 {
        self.x.hypot(self.y)
    }

    pub fn lerp(self, other: Vec2, t: f32) -> Vec2 {
        self + (other - self) * t
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, r: Vec2) -> Vec2 {
        Vec2::new(self.x + r.x, self.y + r.y)
    }
}
impl AddAssign for Vec2 {
    fn add_assign(&mut self, r: Vec2) {
        *self = *self + r;
    }
}
impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, r: Vec2) -> Vec2 {
        Vec2::new(self.x - r.x, self.y - r.y)
    }
}
impl Mul<f32> for Vec2 {
    type Output = Vec2;
    fn mul(self, s: f32) -> Vec2 {
        Vec2::new(self.x * s, self.y * s)
    }
}
impl Neg for Vec2 {
    type Output = Vec2;
    fn neg(self) -> Vec2 {
        Vec2::new(-self.x, -self.y)
    }
}

/// Position and rotation (radians) of one entity at one tick.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Transform2 {
    pub pos: Vec2,
    pub rot: f32,
}

impl Transform2 {
    pub fn new(pos: Vec2, rot: f32) -> Self {
        Self { pos, rot }
    }

    /// Linear position, shortest-path rotation.
    pub fn lerp(self, other: Transform2, t: f32) -> Transform2 {
        Transform2 { pos: self.pos.lerp(other.pos, t), rot: lerp_angle(self.rot, other.rot, t) }
    }
}

/// Wraps an angle to `[-pi, pi]`.
pub(crate) fn wrap_angle(a: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let mut r = a % tau;
    if r > std::f32::consts::PI {
        r -= tau;
    } else if r < -std::f32::consts::PI {
        r += tau;
    }
    r
}

/// Interpolates angles along the shorter way round (359 deg to 1 deg goes through 0).
pub fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    a + wrap_angle(b - a) * t
}
