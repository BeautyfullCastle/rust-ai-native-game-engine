use core::ops::{Add, Mul};

use bytemuck::{Pod, Zeroable};

use crate::fp::FP;
use crate::quat::FPQuat;
use crate::vec::FPVec3;

/// A 3x3 matrix of [`FP`], stored as three rows.
///
/// Used for rotation matrices and world-space inertia tensors. Every
/// operation is integer-only and follows the rounding rules of [`FP`]
/// (each product is floored once, sums of products are summed after the
/// per-term rounding, exactly like writing the formula with `FP`
/// operators).
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Pod, Zeroable, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FPMat3 {
    /// First row.
    pub r0: FPVec3,
    /// Second row.
    pub r1: FPVec3,
    /// Third row.
    pub r2: FPVec3,
}

impl Default for FPMat3 {
    #[inline]
    fn default() -> FPMat3 {
        FPMat3::IDENTITY
    }
}

impl FPMat3 {
    /// All zeros.
    pub const ZERO: FPMat3 = FPMat3 { r0: FPVec3::ZERO, r1: FPVec3::ZERO, r2: FPVec3::ZERO };
    /// The identity matrix.
    pub const IDENTITY: FPMat3 = FPMat3 { r0: FPVec3::X, r1: FPVec3::Y, r2: FPVec3::Z };

    /// Build from rows.
    #[inline]
    #[must_use]
    pub const fn from_rows(r0: FPVec3, r1: FPVec3, r2: FPVec3) -> FPMat3 {
        FPMat3 { r0, r1, r2 }
    }

    /// Build from columns.
    #[inline]
    #[must_use]
    pub const fn from_cols(c0: FPVec3, c1: FPVec3, c2: FPVec3) -> FPMat3 {
        FPMat3 {
            r0: FPVec3::new(c0.x, c1.x, c2.x),
            r1: FPVec3::new(c0.y, c1.y, c2.y),
            r2: FPVec3::new(c0.z, c1.z, c2.z),
        }
    }

    /// Diagonal matrix.
    #[inline]
    #[must_use]
    pub const fn from_diag(d: FPVec3) -> FPMat3 {
        FPMat3 {
            r0: FPVec3::new(d.x, FP::ZERO, FP::ZERO),
            r1: FPVec3::new(FP::ZERO, d.y, FP::ZERO),
            r2: FPVec3::new(FP::ZERO, FP::ZERO, d.z),
        }
    }

    /// Rotation matrix of a unit quaternion (columns are the rotated
    /// basis vectors).
    #[must_use]
    pub fn from_quat(q: FPQuat) -> FPMat3 {
        let (x, y, z, w) = (q.x, q.y, q.z, q.w);
        let (xx, yy, zz) = (x * x, y * y, z * z);
        let (xy, xz, yz) = (x * y, x * z, y * z);
        let (wx, wy, wz) = (w * x, w * y, w * z);
        let two = FP::TWO;
        FPMat3 {
            r0: FPVec3::new(FP::ONE - (yy + zz) * two, (xy - wz) * two, (xz + wy) * two),
            r1: FPVec3::new((xy + wz) * two, FP::ONE - (xx + zz) * two, (yz - wx) * two),
            r2: FPVec3::new((xz - wy) * two, (yz + wx) * two, FP::ONE - (xx + yy) * two),
        }
    }

    /// Column `i` (0..3).
    #[inline]
    #[must_use]
    pub fn col(&self, i: usize) -> FPVec3 {
        match i {
            0 => FPVec3::new(self.r0.x, self.r1.x, self.r2.x),
            1 => FPVec3::new(self.r0.y, self.r1.y, self.r2.y),
            _ => FPVec3::new(self.r0.z, self.r1.z, self.r2.z),
        }
    }

    /// Row `i` (0..3).
    #[inline]
    #[must_use]
    pub fn row(&self, i: usize) -> FPVec3 {
        match i {
            0 => self.r0,
            1 => self.r1,
            _ => self.r2,
        }
    }

    /// Transpose (the inverse of a rotation matrix).
    #[inline]
    #[must_use]
    pub fn transpose(&self) -> FPMat3 {
        FPMat3::from_cols(self.r0, self.r1, self.r2)
    }

    /// Matrix times column vector.
    #[inline]
    #[must_use]
    pub fn mul_vec(&self, v: FPVec3) -> FPVec3 {
        FPVec3::new(self.r0.dot(v), self.r1.dot(v), self.r2.dot(v))
    }

    /// Transpose times vector (`R^T v`, the inverse rotation for a
    /// rotation matrix) without building the transpose.
    #[inline]
    #[must_use]
    pub fn tmul_vec(&self, v: FPVec3) -> FPVec3 {
        self.r0 * v.x + self.r1 * v.y + self.r2 * v.z
    }

    /// Matrix product `self * rhs`.
    #[must_use]
    pub fn mul_mat(&self, rhs: &FPMat3) -> FPMat3 {
        let (c0, c1, c2) = (rhs.col(0), rhs.col(1), rhs.col(2));
        FPMat3::from_cols(self.mul_vec(c0), self.mul_vec(c1), self.mul_vec(c2))
    }

    /// `R * diag(d) * R^T`: the world-space tensor of a body whose tensor
    /// is the diagonal `d` in its local frame, with `self` the body
    /// rotation. Exactly symmetric (the off-diagonal terms are computed
    /// once and mirrored).
    #[must_use]
    pub fn rotate_diag(&self, d: FPVec3) -> FPMat3 {
        let (a, b, c) = (self.col(0), self.col(1), self.col(2));
        let xx = a.x * a.x * d.x + b.x * b.x * d.y + c.x * c.x * d.z;
        let yy = a.y * a.y * d.x + b.y * b.y * d.y + c.y * c.y * d.z;
        let zz = a.z * a.z * d.x + b.z * b.z * d.y + c.z * c.z * d.z;
        let xy = a.x * a.y * d.x + b.x * b.y * d.y + c.x * c.y * d.z;
        let xz = a.x * a.z * d.x + b.x * b.z * d.y + c.x * c.z * d.z;
        let yz = a.y * a.z * d.x + b.y * b.z * d.y + c.y * c.z * d.z;
        FPMat3 { r0: FPVec3::new(xx, xy, xz), r1: FPVec3::new(xy, yy, yz), r2: FPVec3::new(xz, yz, zz) }
    }

    /// Determinant.
    #[must_use]
    pub fn determinant(&self) -> FP {
        self.r0.dot(self.r1.cross(self.r2))
    }

    /// Inverse by the adjugate, or `None` if the determinant is zero.
    #[must_use]
    pub fn inverse(&self) -> Option<FPMat3> {
        let c0 = self.r1.cross(self.r2);
        let c1 = self.r2.cross(self.r0);
        let c2 = self.r0.cross(self.r1);
        let det = self.r0.dot(c0);
        if det.raw() == 0 {
            return None;
        }
        // The cross products are the columns of the adjugate.
        Some(FPMat3::from_cols(c0 / det, c1 / det, c2 / det))
    }

    /// The skew-symmetric matrix `[v]x` with `[v]x * u == v.cross(u)`.
    #[must_use]
    pub fn skew(v: FPVec3) -> FPMat3 {
        FPMat3 {
            r0: FPVec3::new(FP::ZERO, -v.z, v.y),
            r1: FPVec3::new(v.z, FP::ZERO, -v.x),
            r2: FPVec3::new(-v.y, v.x, FP::ZERO),
        }
    }
}

impl Mul<FPVec3> for FPMat3 {
    type Output = FPVec3;
    #[inline]
    fn mul(self, v: FPVec3) -> FPVec3 {
        self.mul_vec(v)
    }
}

impl Mul<FPMat3> for FPMat3 {
    type Output = FPMat3;
    #[inline]
    fn mul(self, rhs: FPMat3) -> FPMat3 {
        self.mul_mat(&rhs)
    }
}

impl Mul<FP> for FPMat3 {
    type Output = FPMat3;
    #[inline]
    fn mul(self, s: FP) -> FPMat3 {
        FPMat3 { r0: self.r0 * s, r1: self.r1 * s, r2: self.r2 * s }
    }
}

impl Add for FPMat3 {
    type Output = FPMat3;
    #[inline]
    fn add(self, rhs: FPMat3) -> FPMat3 {
        FPMat3 { r0: self.r0 + rhs.r0, r1: self.r1 + rhs.r1, r2: self.r2 + rhs.r2 }
    }
}
