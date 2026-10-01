use core::ops::Mul;

use bytemuck::{Pod, Zeroable};

use crate::fp::FP;
use crate::vec::FPVec3;

/// A unit quaternion over [`FP`] components, `x*i + y*j + z*k + w`.
///
/// Rotation helpers assume `self` is (approximately) normalized; call
/// [`FPQuat::normalize`] after accumulating error (e.g. many small
/// integrations) to keep it that way.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Pod, Zeroable, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FPQuat {
    /// X (i) component.
    pub x: FP,
    /// Y (j) component.
    pub y: FP,
    /// Z (k) component.
    pub z: FP,
    /// W (real) component.
    pub w: FP,
}

impl Default for FPQuat {
    #[inline]
    fn default() -> FPQuat {
        FPQuat::IDENTITY
    }
}

impl FPQuat {
    /// The identity rotation.
    pub const IDENTITY: FPQuat = FPQuat { x: FP::ZERO, y: FP::ZERO, z: FP::ZERO, w: FP::ONE };

    /// Build directly from components.
    #[inline]
    #[must_use]
    pub const fn new(x: FP, y: FP, z: FP, w: FP) -> FPQuat {
        FPQuat { x, y, z, w }
    }

    /// Build a rotation of `angle` radians about `axis`. `axis` is assumed
    /// to already be a unit vector (use [`FPVec3::normalize`] first if
    /// it isn't).
    #[must_use]
    pub fn from_axis_angle(axis: FPVec3, angle: FP) -> FPQuat {
        let half = angle / FP::TWO;
        let (s, c) = half.sin_cos();
        FPQuat::new(axis.x * s, axis.y * s, axis.z * s, c)
    }

    /// Hamilton product `self * rhs` (apply `rhs` first, then `self`).
    #[must_use]
    pub fn hamilton_mul(self, rhs: FPQuat) -> FPQuat {
        FPQuat::new(
            self.w * rhs.x + self.x * rhs.w + self.y * rhs.z - self.z * rhs.y,
            self.w * rhs.y - self.x * rhs.z + self.y * rhs.w + self.z * rhs.x,
            self.w * rhs.z + self.x * rhs.y - self.y * rhs.x + self.z * rhs.w,
            self.w * rhs.w - self.x * rhs.x - self.y * rhs.y - self.z * rhs.z,
        )
    }

    /// Rotate a vector by this quaternion (assumes `self` is unit
    /// length): `v + 2*w*cross(qv, v) + 2*cross(qv, cross(qv, v))`.
    #[must_use]
    pub fn rotate_vec3(self, v: FPVec3) -> FPVec3 {
        let qv = FPVec3::new(self.x, self.y, self.z);
        let t = qv.cross(v) * FP::TWO;
        v + t * self.w + qv.cross(t)
    }

    /// The conjugate (for a unit quaternion, equal to the inverse).
    #[inline]
    #[must_use]
    pub fn conjugate(self) -> FPQuat {
        FPQuat::new(-self.x, -self.y, -self.z, self.w)
    }

    /// Squared length of the 4-component vector.
    #[inline]
    #[must_use]
    pub fn length_sq(self) -> FP {
        self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w
    }

    /// Normalize to unit length. Panics if `self` is (numerically) the
    /// zero quaternion.
    #[must_use]
    pub fn normalize(self) -> FPQuat {
        let len = self.length_sq().sqrt();
        FPQuat::new(self.x / len, self.y / len, self.z / len, self.w / len)
    }

    /// Normalized linear interpolation towards `other` by `t`. Cheaper
    /// than a true `slerp` and adequate for small per-frame rotation
    /// deltas; picks the shorter path by flipping `other` when the
    /// quaternions are more than 90 degrees apart.
    #[must_use]
    pub fn nlerp(self, other: FPQuat, t: FP) -> FPQuat {
        let dot = self.x * other.x + self.y * other.y + self.z * other.z + self.w * other.w;
        let other = if dot.raw() < 0 {
            FPQuat::new(-other.x, -other.y, -other.z, -other.w)
        } else {
            other
        };
        FPQuat::new(
            self.x.lerp(other.x, t),
            self.y.lerp(other.y, t),
            self.z.lerp(other.z, t),
            self.w.lerp(other.w, t),
        )
        .normalize()
    }

    /// Build a rotation from yaw (about Z), pitch (about Y) and roll
    /// (about X), all in radians, composed as `yaw * pitch * roll`
    /// (intrinsic Z-Y-X / "roll, pitch, yaw" Tait-Bryan convention).
    #[must_use]
    pub fn from_euler(yaw: FP, pitch: FP, roll: FP) -> FPQuat {
        let (sy, cy) = (yaw / FP::TWO).sin_cos();
        let (sp, cp) = (pitch / FP::TWO).sin_cos();
        let (sr, cr) = (roll / FP::TWO).sin_cos();

        FPQuat::new(
            sr * cp * cy - cr * sp * sy,
            cr * sp * cy + sr * cp * sy,
            cr * cp * sy - sr * sp * cy,
            cr * cp * cy + sr * sp * sy,
        )
    }

    /// Recover `(yaw, pitch, roll)` radians, inverse of
    /// [`FPQuat::from_euler`]. Near the pitch = +-pi/2 gimbal-lock poles,
    /// `pitch` saturates to +-pi/2 and `yaw`/`roll` become degenerate (as
    /// with any Euler-angle extraction).
    #[must_use]
    pub fn to_euler(self) -> (FP, FP, FP) {
        let (x, y, z, w) = (self.x, self.y, self.z, self.w);

        let sinr_cosp = (w * x + y * z) * FP::TWO;
        let cosr_cosp = FP::ONE - (x * x + y * y) * FP::TWO;
        let roll = sinr_cosp.atan2(cosr_cosp);

        let sinp = ((w * y - z * x) * FP::TWO).clamp(FP::MINUS_ONE, FP::ONE);
        let pitch = sinp.asin();

        let siny_cosp = (w * z + x * y) * FP::TWO;
        let cosy_cosp = FP::ONE - (y * y + z * z) * FP::TWO;
        let yaw = siny_cosp.atan2(cosy_cosp);

        (yaw, pitch, roll)
    }
}

impl Mul for FPQuat {
    type Output = FPQuat;
    #[inline]
    fn mul(self, rhs: FPQuat) -> FPQuat {
        FPQuat::hamilton_mul(self, rhs)
    }
}

impl FPQuat {
    /// Rotate a vector by the inverse of this (unit) quaternion.
    #[must_use]
    pub fn inverse_rotate_vec3(self, v: FPVec3) -> FPVec3 {
        self.conjugate().rotate_vec3(v)
    }

    /// Normalize, or return the identity for a (numerically) zero
    /// quaternion instead of panicking.
    #[must_use]
    pub fn normalize_or_identity(self) -> FPQuat {
        let len = self.length_sq().sqrt();
        if len.raw() == 0 {
            FPQuat::IDENTITY
        } else {
            FPQuat::new(self.x / len, self.y / len, self.z / len, self.w / len)
        }
    }

    /// Advance the orientation by the angular velocity `omega` (world
    /// frame, radians per second) over `dt` seconds, with the first-order
    /// update `q' = normalize(q + dt/2 * (omega, 0) * q)`. Deterministic
    /// and cheap; accurate for small `omega * dt` (the physics step uses
    /// it at 60 Hz). The result is renormalized.
    #[must_use]
    pub fn integrate_angular(self, omega: FPVec3, dt: FP) -> FPQuat {
        let h = dt / FP::TWO;
        let wq = FPQuat::new(omega.x * h, omega.y * h, omega.z * h, FP::ZERO);
        let d = wq.hamilton_mul(self);
        FPQuat::new(self.x + d.x, self.y + d.y, self.z + d.z, self.w + d.w).normalize_or_identity()
    }

    /// Axis (unit) and angle in `[0, pi]` of this unit quaternion. The
    /// identity gives `(X, 0)`.
    #[must_use]
    pub fn to_axis_angle(self) -> (FPVec3, FP) {
        let q = if self.w.raw() < 0 { FPQuat::new(-self.x, -self.y, -self.z, -self.w) } else { self };
        let v = FPVec3::new(q.x, q.y, q.z);
        let s = v.length();
        if s.raw() == 0 {
            return (FPVec3::X, FP::ZERO);
        }
        (v / s, s.atan2(q.w) * FP::TWO)
    }

    /// The rotation matrix of this unit quaternion.
    #[must_use]
    pub fn to_mat3(self) -> crate::mat3::FPMat3 {
        crate::mat3::FPMat3::from_quat(self)
    }
}
