//! Segment geometry for capsules and shape casts.
//!
//! Positions along a segment are kept as Q32 fractions (`i128`, 1.0 =
//! `1 << 32`) and computed from exact `i128` dot products of the raw
//! coordinates. A closest point is therefore accurate to one raw unit
//! (1/65536), no matter how long the segment is. The products stay inside
//! `i128` for shapes up to the documented size limit (1000 units) and
//! world positions up to 30 000 units.

use orr_fp::{FPVec2, FP};

const ONE32: i128 = 1 << 32;

#[inline]
fn dot_raw(a: FPVec2, b: FPVec2) -> i128 {
    a.x.raw() as i128 * b.x.raw() as i128 + a.y.raw() as i128 * b.y.raw() as i128
}

/// `num / den` as a Q32 fraction clamped to `[0, 1]`. `den` must be > 0.
///
/// Both operands are scaled down until the divisor has 31 bits, so the
/// division is a 64-bit one (a 128-bit division costs several times more).
/// The scaling error is a relative 2^-30 of the result, far below one raw
/// unit of position for any segment inside the documented sizes.
#[inline]
fn ratio(num: i128, den: i128) -> i128 {
    if num <= 0 {
        return 0;
    }
    if num >= den {
        return ONE32;
    }
    let sh = (128 - den.leading_zeros()).saturating_sub(31);
    let (n, d) = ((num >> sh) as u64, (den >> sh) as u64);
    ((n << 32) / d) as i128
}

/// `a + ab * s` for a Q32 fraction `s`.
#[inline]
fn lerp32(a: FPVec2, ab: FPVec2, s: i128) -> FPVec2 {
    let f = |a: FP, d: FP| FP::from_raw((a.raw() as i128 + ((d.raw() as i128 * s) >> 32)) as i64);
    FPVec2::new(f(a.x, ab.x), f(a.y, ab.y))
}

/// Counter-clockwise perpendicular.
#[inline]
pub(crate) fn perp(v: FPVec2) -> FPVec2 {
    FPVec2::new(-v.y, v.x)
}

/// Closest point to `p` on the segment `a`-`b`.
pub(crate) fn closest_on_segment(p: FPVec2, a: FPVec2, b: FPVec2) -> FPVec2 {
    let ab = b - a;
    let den = dot_raw(ab, ab);
    if den == 0 {
        return a;
    }
    lerp32(a, ab, ratio(dot_raw(p - a, ab), den))
}

/// Closest points of the segments `p1`-`q1` and `p2`-`q2`. Ties (parallel
/// segments) resolve toward the start of the first segment.
pub(crate) fn seg_seg_closest(p1: FPVec2, q1: FPVec2, p2: FPVec2, q2: FPVec2) -> (FPVec2, FPVec2) {
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = dot_raw(d1, d1);
    let e = dot_raw(d2, d2);
    let f = dot_raw(d2, r);
    let (s, t);
    if a == 0 && e == 0 {
        return (p1, p2);
    } else if a == 0 {
        s = 0;
        t = ratio(f, e);
    } else {
        let c = dot_raw(d1, r);
        if e == 0 {
            t = 0;
            s = ratio(-c, a);
        } else {
            let b = dot_raw(d1, d2);
            let denom = a * e - b * b;
            let mut s0 = if denom > 0 { ratio(b * f - c * e, denom) } else { 0 };
            // t = (b * s + f) / e, with s in Q32.
            let tn = ((b * s0) >> 32) + f;
            let t0 = if tn < 0 {
                s0 = ratio(-c, a);
                0
            } else if tn > e {
                s0 = ratio(b - c, a);
                ONE32
            } else {
                ratio(tn, e)
            };
            s = s0;
            t = t0;
        }
    }
    (lerp32(p1, d1, s), lerp32(p2, d2, t))
}

/// The point of segment `a`-`b` whose scalar coordinate is `s`, where the
/// coordinate runs linearly from `from` (at `a`) to `to` (at `b`) and
/// `from != to`. `s` is clamped into that range.
pub(crate) fn point_at(a: FPVec2, b: FPVec2, from: FP, to: FP, s: FP) -> FPVec2 {
    let (num, den) = if to > from {
        ((s.raw() - from.raw()) as i128, (to.raw() - from.raw()) as i128)
    } else {
        ((from.raw() - s.raw()) as i128, (from.raw() - to.raw()) as i128)
    };
    lerp32(a, b - a, ratio(num, den))
}
