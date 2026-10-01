//! 3D `f32` math of the view layer: vector, unit quaternion, and a pose
//! (position + orientation) that interpolates with slerp.

use std::ops::{Add, AddAssign, Mul, Neg, Sub};

/// A 3D `f32` vector (view layer).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };

    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub fn dot(self, o: Vec3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(self.y * o.z - self.z * o.y, self.z * o.x - self.x * o.z, self.x * o.y - self.y * o.x)
    }

    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    pub fn lerp(self, other: Vec3, t: f32) -> Vec3 {
        self + (other - self) * t
    }

    pub fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, r: Vec3) -> Vec3 {
        Vec3::new(self.x + r.x, self.y + r.y, self.z + r.z)
    }
}
impl AddAssign for Vec3 {
    fn add_assign(&mut self, r: Vec3) {
        *self = *self + r;
    }
}
impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, r: Vec3) -> Vec3 {
        Vec3::new(self.x - r.x, self.y - r.y, self.z - r.z)
    }
}
impl Mul<f32> for Vec3 {
    type Output = Vec3;
    fn mul(self, s: f32) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }
}
impl Neg for Vec3 {
    type Output = Vec3;
    fn neg(self) -> Vec3 {
        Vec3::new(-self.x, -self.y, -self.z)
    }
}

/// A rotation as a unit quaternion `x*i + y*j + z*k + w`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Default for Quat {
    fn default() -> Self {
        Quat::IDENTITY
    }
}

impl Quat {
    pub const IDENTITY: Quat = Quat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 };

    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    /// Rotation of `angle` radians about the unit vector `axis`.
    pub fn from_axis_angle(axis: Vec3, angle: f32) -> Quat {
        let (s, c) = (angle * 0.5).sin_cos();
        Quat::new(axis.x * s, axis.y * s, axis.z * s, c)
    }

    pub fn to_array(self) -> [f32; 4] {
        [self.x, self.y, self.z, self.w]
    }

    pub fn dot(self, o: Quat) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z + self.w * o.w
    }

    pub fn conjugate(self) -> Quat {
        Quat::new(-self.x, -self.y, -self.z, self.w)
    }

    /// Unit length again; the identity for a (near) zero quaternion.
    pub fn normalize(self) -> Quat {
        let l = self.dot(self).sqrt();
        if l > 1e-12 {
            Quat::new(self.x / l, self.y / l, self.z / l, self.w / l)
        } else {
            Quat::IDENTITY
        }
    }

    /// Rotates `v`.
    pub fn rotate(self, v: Vec3) -> Vec3 {
        let q = Vec3::new(self.x, self.y, self.z);
        let t = q.cross(v) * 2.0;
        v + t * self.w + q.cross(t)
    }

    /// The angle (radians, `0..=pi`) of the shortest rotation from `self` to `o`.
    pub fn angle_to(self, o: Quat) -> f32 {
        2.0 * self.dot(o).abs().clamp(0.0, 1.0).acos()
    }

    fn negated(self) -> Quat {
        Quat::new(-self.x, -self.y, -self.z, -self.w)
    }

    /// Spherical interpolation along the shortest arc (`q` and `-q` are the
    /// same rotation, so a flipped sign never makes the long way round).
    pub fn slerp(self, other: Quat, t: f32) -> Quat {
        let mut b = other;
        let mut d = self.dot(b);
        if d < 0.0 {
            b = b.negated();
            d = -d;
        }
        if d > 0.9995 {
            // Nearly the same: a normalized lerp is exact enough and avoids 0/0.
            return Quat::new(
                self.x + (b.x - self.x) * t,
                self.y + (b.y - self.y) * t,
                self.z + (b.z - self.z) * t,
                self.w + (b.w - self.w) * t,
            )
            .normalize();
        }
        let theta = d.clamp(-1.0, 1.0).acos();
        let s = theta.sin();
        let (wa, wb) = (((1.0 - t) * theta).sin() / s, (t * theta).sin() / s);
        Quat::new(
            self.x * wa + b.x * wb,
            self.y * wa + b.y * wb,
            self.z * wa + b.z * wb,
            self.w * wa + b.w * wb,
        )
        .normalize()
    }
}

/// `a * b`: apply `b` first, then `a`.
impl Mul<Quat> for Quat {
    type Output = Quat;
    fn mul(self, r: Quat) -> Quat {
        Quat::new(
            self.w * r.x + self.x * r.w + self.y * r.z - self.z * r.y,
            self.w * r.y - self.x * r.z + self.y * r.w + self.z * r.x,
            self.w * r.z + self.x * r.y - self.y * r.x + self.z * r.w,
            self.w * r.w - self.x * r.x - self.y * r.y - self.z * r.z,
        )
    }
}

/// Position and orientation of one entity at one tick.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Transform3 {
    pub pos: Vec3,
    pub rot: Quat,
}

impl Transform3 {
    pub fn new(pos: Vec3, rot: Quat) -> Self {
        Self { pos, rot }
    }

    /// Linear position, slerp rotation.
    pub fn lerp(self, other: Transform3, t: f32) -> Transform3 {
        Transform3 { pos: self.pos.lerp(other.pos, t), rot: self.rot.slerp(other.rot, t) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn about_y(a: f32) -> Quat {
        Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), a)
    }

    #[test]
    fn slerp_halfway_is_the_half_angle() {
        let q = about_y(0.0).slerp(about_y(1.2), 0.5);
        assert!(q.angle_to(about_y(0.6)) < 1e-4, "{q:?}");
        let end = about_y(0.0).slerp(about_y(1.2), 1.0);
        assert!(end.angle_to(about_y(1.2)) < 1e-4);
    }

    #[test]
    fn slerp_has_constant_angular_speed() {
        let (a, b) = (about_y(0.1), Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), 2.0));
        let total = a.angle_to(b);
        for i in 1..10 {
            let t = i as f32 / 10.0;
            let q = a.slerp(b, t);
            assert!((a.angle_to(q) - total * t).abs() < 1e-3, "t={t}");
        }
    }

    #[test]
    fn slerp_takes_the_short_way_even_with_a_flipped_sign() {
        let a = about_y(0.1);
        let b = about_y(0.3).negated(); // the same rotation as 0.3, opposite sign
        let q = a.slerp(b, 0.5);
        assert!(q.angle_to(about_y(0.2)) < 1e-4, "{q:?}");
        // And across the +-pi wrap: 3.0 to -3.0 rad goes through pi, a 0.28 rad arc.
        let q = about_y(3.0).slerp(about_y(-3.0), 0.5);
        assert!(q.angle_to(about_y(std::f32::consts::PI)) < 1e-3, "{q:?}");
    }

    #[test]
    fn slerp_of_equal_rotations_is_stable() {
        let q = about_y(0.7);
        let r = q.slerp(q, 0.3);
        assert!(r.angle_to(q) < 1e-6 && (r.dot(r) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn rotate_agrees_with_axis_angle() {
        let v = about_y(std::f32::consts::FRAC_PI_2).rotate(Vec3::new(1.0, 0.0, 0.0));
        assert!((v.x).abs() < 1e-6 && (v.z + 1.0).abs() < 1e-6, "{v:?}");
    }

    #[test]
    fn composition_applies_the_right_factor_first() {
        let a = about_y(0.4);
        let b = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), 0.9);
        let v = Vec3::new(0.3, -1.0, 2.0);
        let direct = a.rotate(b.rotate(v));
        let composed = (a * b).rotate(v);
        assert!((direct - composed).length() < 1e-5);
    }
}
