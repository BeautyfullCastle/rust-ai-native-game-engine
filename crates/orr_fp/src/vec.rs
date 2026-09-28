use core::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

use bytemuck::{Pod, Zeroable};

use crate::fp::FP;

/// A 2D vector of [`FP`] fixed-point components.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Pod, Zeroable, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FPVec2 {
    /// X component.
    pub x: FP,
    /// Y component.
    pub y: FP,
}

impl FPVec2 {
    /// The zero vector.
    pub const ZERO: FPVec2 = FPVec2 { x: FP::ZERO, y: FP::ZERO };
    /// `(1, 0)`
    pub const X: FPVec2 = FPVec2 { x: FP::ONE, y: FP::ZERO };
    /// `(0, 1)`
    pub const Y: FPVec2 = FPVec2 { x: FP::ZERO, y: FP::ONE };
    /// `(1, 1)`
    pub const ONE: FPVec2 = FPVec2 { x: FP::ONE, y: FP::ONE };

    /// Build a new vector.
    #[inline]
    #[must_use]
    pub const fn new(x: FP, y: FP) -> FPVec2 {
        FPVec2 { x, y }
    }

    /// Component-wise splat.
    #[inline]
    #[must_use]
    pub const fn splat(v: FP) -> FPVec2 {
        FPVec2 { x: v, y: v }
    }

    /// Dot product.
    #[inline]
    #[must_use]
    pub fn dot(self, rhs: FPVec2) -> FP {
        self.x * rhs.x + self.y * rhs.y
    }

    /// 2D "cross product" / perpendicular dot product (`x1*y2 - y1*x2`);
    /// its magnitude is the area of the parallelogram spanned by the two
    /// vectors, and its sign tells which way `rhs` turns relative to
    /// `self`.
    #[inline]
    #[must_use]
    pub fn perp_dot(self, rhs: FPVec2) -> FP {
        self.x * rhs.y - self.y * rhs.x
    }

    /// Squared length (avoids the `sqrt` in [`FPVec2::length`]).
    #[inline]
    #[must_use]
    pub fn length_sq(self) -> FP {
        self.dot(self)
    }

    /// Length (magnitude).
    #[inline]
    #[must_use]
    pub fn length(self) -> FP {
        self.length_sq().sqrt()
    }

    /// Euclidean distance to `rhs`.
    #[inline]
    #[must_use]
    pub fn distance(self, rhs: FPVec2) -> FP {
        (self - rhs).length()
    }

    /// Normalize to unit length. Panics if `self` is the zero vector (see
    /// [`FPVec2::normalize_or_zero`] for a zero-safe variant).
    #[inline]
    #[must_use]
    pub fn normalize(self) -> FPVec2 {
        let len = self.length();
        self / len
    }

    /// Normalize to unit length, returning [`FPVec2::ZERO`] instead of
    /// panicking if `self` is (numerically) the zero vector.
    #[inline]
    #[must_use]
    pub fn normalize_or_zero(self) -> FPVec2 {
        let len = self.length();
        if len.raw() == 0 {
            FPVec2::ZERO
        } else {
            self / len
        }
    }

    /// Component-wise minimum.
    #[inline]
    #[must_use]
    pub fn min(self, rhs: FPVec2) -> FPVec2 {
        FPVec2::new(self.x.min(rhs.x), self.y.min(rhs.y))
    }

    /// Component-wise maximum.
    #[inline]
    #[must_use]
    pub fn max(self, rhs: FPVec2) -> FPVec2 {
        FPVec2::new(self.x.max(rhs.x), self.y.max(rhs.y))
    }

    /// Linear interpolation between `self` and `rhs` by `t`.
    #[inline]
    #[must_use]
    pub fn lerp(self, rhs: FPVec2, t: FP) -> FPVec2 {
        FPVec2::new(self.x.lerp(rhs.x, t), self.y.lerp(rhs.y, t))
    }

    /// Rotate by `angle` radians (counter-clockwise).
    #[inline]
    #[must_use]
    pub fn rotate(self, angle: FP) -> FPVec2 {
        let (s, c) = angle.sin_cos();
        FPVec2::new(self.x * c - self.y * s, self.x * s + self.y * c)
    }

    /// A unit vector pointing at `angle` radians from the positive
    /// x-axis.
    #[inline]
    #[must_use]
    pub fn from_angle(angle: FP) -> FPVec2 {
        let (s, c) = angle.sin_cos();
        FPVec2::new(c, s)
    }
}

impl Add for FPVec2 {
    type Output = FPVec2;
    #[inline]
    fn add(self, rhs: FPVec2) -> FPVec2 {
        FPVec2::new(self.x + rhs.x, self.y + rhs.y)
    }
}
impl Sub for FPVec2 {
    type Output = FPVec2;
    #[inline]
    fn sub(self, rhs: FPVec2) -> FPVec2 {
        FPVec2::new(self.x - rhs.x, self.y - rhs.y)
    }
}
impl Neg for FPVec2 {
    type Output = FPVec2;
    #[inline]
    fn neg(self) -> FPVec2 {
        FPVec2::new(-self.x, -self.y)
    }
}
impl Mul<FP> for FPVec2 {
    type Output = FPVec2;
    #[inline]
    fn mul(self, rhs: FP) -> FPVec2 {
        FPVec2::new(self.x * rhs, self.y * rhs)
    }
}
impl Div<FP> for FPVec2 {
    type Output = FPVec2;
    #[inline]
    fn div(self, rhs: FP) -> FPVec2 {
        FPVec2::new(self.x / rhs, self.y / rhs)
    }
}
impl Mul<FPVec2> for FPVec2 {
    type Output = FPVec2;
    #[inline]
    fn mul(self, rhs: FPVec2) -> FPVec2 {
        FPVec2::new(self.x * rhs.x, self.y * rhs.y)
    }
}
impl AddAssign for FPVec2 {
    #[inline]
    fn add_assign(&mut self, rhs: FPVec2) {
        *self = *self + rhs;
    }
}
impl SubAssign for FPVec2 {
    #[inline]
    fn sub_assign(&mut self, rhs: FPVec2) {
        *self = *self - rhs;
    }
}

/// A 3D vector of [`FP`] fixed-point components.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Pod, Zeroable, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FPVec3 {
    /// X component.
    pub x: FP,
    /// Y component.
    pub y: FP,
    /// Z component.
    pub z: FP,
}

impl FPVec3 {
    /// The zero vector.
    pub const ZERO: FPVec3 = FPVec3 { x: FP::ZERO, y: FP::ZERO, z: FP::ZERO };
    /// `(1, 0, 0)`
    pub const X: FPVec3 = FPVec3 { x: FP::ONE, y: FP::ZERO, z: FP::ZERO };
    /// `(0, 1, 0)`
    pub const Y: FPVec3 = FPVec3 { x: FP::ZERO, y: FP::ONE, z: FP::ZERO };
    /// `(0, 0, 1)`
    pub const Z: FPVec3 = FPVec3 { x: FP::ZERO, y: FP::ZERO, z: FP::ONE };
    /// `(1, 1, 1)`
    pub const ONE: FPVec3 = FPVec3 { x: FP::ONE, y: FP::ONE, z: FP::ONE };

    /// Build a new vector.
    #[inline]
    #[must_use]
    pub const fn new(x: FP, y: FP, z: FP) -> FPVec3 {
        FPVec3 { x, y, z }
    }

    /// Component-wise splat.
    #[inline]
    #[must_use]
    pub const fn splat(v: FP) -> FPVec3 {
        FPVec3 { x: v, y: v, z: v }
    }

    /// Dot product.
    #[inline]
    #[must_use]
    pub fn dot(self, rhs: FPVec3) -> FP {
        self.x * rhs.x + self.y * rhs.y + self.z * rhs.z
    }

    /// Cross product.
    #[inline]
    #[must_use]
    pub fn cross(self, rhs: FPVec3) -> FPVec3 {
        FPVec3::new(
            self.y * rhs.z - self.z * rhs.y,
            self.z * rhs.x - self.x * rhs.z,
            self.x * rhs.y - self.y * rhs.x,
        )
    }

    /// Squared length.
    #[inline]
    #[must_use]
    pub fn length_sq(self) -> FP {
        self.dot(self)
    }

    /// Length (magnitude).
    #[inline]
    #[must_use]
    pub fn length(self) -> FP {
        self.length_sq().sqrt()
    }

    /// Euclidean distance to `rhs`.
    #[inline]
    #[must_use]
    pub fn distance(self, rhs: FPVec3) -> FP {
        (self - rhs).length()
    }

    /// Normalize to unit length. Panics if `self` is the zero vector.
    #[inline]
    #[must_use]
    pub fn normalize(self) -> FPVec3 {
        let len = self.length();
        self / len
    }

    /// Normalize to unit length, returning [`FPVec3::ZERO`] instead of
    /// panicking if `self` is (numerically) the zero vector.
    #[inline]
    #[must_use]
    pub fn normalize_or_zero(self) -> FPVec3 {
        let len = self.length();
        if len.raw() == 0 {
            FPVec3::ZERO
        } else {
            self / len
        }
    }

    /// Component-wise minimum.
    #[inline]
    #[must_use]
    pub fn min(self, rhs: FPVec3) -> FPVec3 {
        FPVec3::new(self.x.min(rhs.x), self.y.min(rhs.y), self.z.min(rhs.z))
    }

    /// Component-wise maximum.
    #[inline]
    #[must_use]
    pub fn max(self, rhs: FPVec3) -> FPVec3 {
        FPVec3::new(self.x.max(rhs.x), self.y.max(rhs.y), self.z.max(rhs.z))
    }

    /// Linear interpolation between `self` and `rhs` by `t`.
    #[inline]
    #[must_use]
    pub fn lerp(self, rhs: FPVec3, t: FP) -> FPVec3 {
        FPVec3::new(self.x.lerp(rhs.x, t), self.y.lerp(rhs.y, t), self.z.lerp(rhs.z, t))
    }
}

impl Add for FPVec3 {
    type Output = FPVec3;
    #[inline]
    fn add(self, rhs: FPVec3) -> FPVec3 {
        FPVec3::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}
impl Sub for FPVec3 {
    type Output = FPVec3;
    #[inline]
    fn sub(self, rhs: FPVec3) -> FPVec3 {
        FPVec3::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}
impl Neg for FPVec3 {
    type Output = FPVec3;
    #[inline]
    fn neg(self) -> FPVec3 {
        FPVec3::new(-self.x, -self.y, -self.z)
    }
}
impl Mul<FP> for FPVec3 {
    type Output = FPVec3;
    #[inline]
    fn mul(self, rhs: FP) -> FPVec3 {
        FPVec3::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}
impl Div<FP> for FPVec3 {
    type Output = FPVec3;
    #[inline]
    fn div(self, rhs: FP) -> FPVec3 {
        FPVec3::new(self.x / rhs, self.y / rhs, self.z / rhs)
    }
}
impl Mul<FPVec3> for FPVec3 {
    type Output = FPVec3;
    #[inline]
    fn mul(self, rhs: FPVec3) -> FPVec3 {
        FPVec3::new(self.x * rhs.x, self.y * rhs.y, self.z * rhs.z)
    }
}
impl AddAssign for FPVec3 {
    #[inline]
    fn add_assign(&mut self, rhs: FPVec3) {
        *self = *self + rhs;
    }
}
impl SubAssign for FPVec3 {
    #[inline]
    fn sub_assign(&mut self, rhs: FPVec3) {
        *self = *self - rhs;
    }
}
