//! Exact helpers shared by the narrow phase and the queries: transforms,
//! 128-bit dot products and segment closest points.

use orr_fp::{FPMat3, FPVec3, FP};

use crate::types::{Shape, SHAPE_SPHERE};

/// Rigid transform of a body: center of mass and rotation matrix (the
/// columns are the body axes in world space).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Xf {
    pub p: FPVec3,
    pub r: FPMat3,
}

impl Xf {
    #[inline]
    pub fn axis(&self, i: usize) -> FPVec3 {
        self.r.col(i)
    }
    /// World point to body-local coordinates.
    #[inline]
    pub fn to_local(&self, w: FPVec3) -> FPVec3 {
        self.r.tmul_vec(w - self.p)
    }
    /// Body-local point to world coordinates.
    #[inline]
    pub fn to_world(&self, l: FPVec3) -> FPVec3 {
        self.p + self.r.mul_vec(l)
    }
}

/// Exact dot product of two vectors in raw units (Q32).
#[inline]
pub(crate) fn dotr(a: FPVec3, b: FPVec3) -> i128 {
    a.x.raw() as i128 * b.x.raw() as i128 + a.y.raw() as i128 * b.y.raw() as i128 + a.z.raw() as i128 * b.z.raw() as i128
}

/// `num / den` as a Q16 raw value (the ratio of two numbers of the same
/// scale). Reduces both numbers first if `num << 16` could overflow;
/// saturates if the divisor vanishes.
#[inline]
pub(crate) fn ratio_q16(num: i128, den: i128) -> i64 {
    let bits = 128 - num.unsigned_abs().leading_zeros();
    let (mut n, mut d) = (num, den);
    if bits > 110 {
        let s = bits - 110;
        n >>= s;
        d >>= s;
    }
    if d == 0 {
        return if n >= 0 { i64::MAX / 4 } else { i64::MIN / 4 };
    }
    let q = (n << 16) / d;
    q.clamp(i64::MIN as i128 / 4, i64::MAX as i128 / 4) as i64
}

#[inline]
fn clamp01(raw: i64) -> FP {
    FP::from_raw(raw.clamp(0, 65536))
}

/// Parameters `(s, t)` in `[0, 1]` of the closest points between the
/// segments `p1..q1` and `p2..q2` (Ericson), with exact 128-bit dots.
pub(crate) fn closest_seg_seg(p1: FPVec3, q1: FPVec3, p2: FPVec3, q2: FPVec3) -> (FP, FP) {
    const EPS: i128 = 1 << 10;
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = dotr(d1, d1);
    let e = dotr(d2, d2);
    let f = dotr(d2, r);
    if a <= EPS && e <= EPS {
        return (FP::ZERO, FP::ZERO);
    }
    if a <= EPS {
        return (FP::ZERO, clamp01(ratio_q16(f, e)));
    }
    let c = dotr(d1, r);
    if e <= EPS {
        return (clamp01(ratio_q16(-c, a)), FP::ZERO);
    }
    let b = dotr(d1, d2);
    let denom = a * e - b * b;
    let s = if denom > 0 { clamp01(ratio_q16(b * f - c * e, denom)) } else { FP::ZERO };
    // t = (b s + f) / e, with b*s in Q48 and f in Q32.
    let tn = b * s.raw() as i128 + (f << 16);
    let t_raw = (tn / e).clamp(i64::MIN as i128 / 4, i64::MAX as i128 / 4) as i64;
    if t_raw < 0 {
        (clamp01(ratio_q16(-c, a)), FP::ZERO)
    } else if t_raw > 65536 {
        (clamp01(ratio_q16(b - c, a)), FP::ONE)
    } else {
        (s, FP::from_raw(t_raw))
    }
}

/// End points of the capsule axis (a sphere gives two equal points).
#[inline]
pub(crate) fn segment_of(shape: &Shape, xf: &Xf) -> (FPVec3, FPVec3) {
    if shape.kind == SHAPE_SPHERE {
        return (xf.p, xf.p);
    }
    let a = xf.axis(1) * shape.half.y;
    (xf.p - a, xf.p + a)
}

/// Fallback unit vector perpendicular to `v` (or `+y` for a zero `v`).
pub(crate) fn any_perpendicular(v: FPVec3) -> FPVec3 {
    let n = v.normalize_or_zero();
    if n == FPVec3::ZERO {
        FPVec3::Y
    } else {
        n.orthonormal_basis().0
    }
}
