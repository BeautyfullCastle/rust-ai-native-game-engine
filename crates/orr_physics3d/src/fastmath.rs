//! Narrow multiply and fast division for the value ranges physics uses.
//! Both return exactly the same bits as the `FP` operators wherever the
//! 64-bit path is exact.

use orr_fp::FP;

/// `a * b` for the solver: the 64-bit product is assumed not to overflow.
/// That holds for every state inside the documented ranges (the real-valued
/// product of two solver terms stays far below 2^31 units), and then the
/// result has the same bits as `a * b`. A debug build panics on an
/// overflowing product; a release build wraps the same way on every
/// platform, so out-of-range input stays deterministic, just meaningless.
#[inline(always)]
pub(crate) fn nmul(a: FP, b: FP) -> FP {
    debug_assert!(a.raw().checked_mul(b.raw()).is_some(), "orr_physics3d: solver product out of range");
    FP::from_raw(a.raw().wrapping_mul(b.raw()) >> 16)
}

/// `a * b` rounded to nearest (ties up) instead of floored. Used where a
/// tiny value is added every tick (position integration), so the rounding
/// does not push resting bodies towards negative coordinates.
#[inline]
pub(crate) fn mul_round(a: FP, b: FP) -> FP {
    FP::from_raw(((a.raw() as i128 * b.raw() as i128 + 32768) >> 16) as i64)
}

/// Same result as `a / b` (truncating). Panics on division by zero like
/// the operator.
#[inline]
pub(crate) fn div(a: FP, b: FP) -> FP {
    let (ar, br) = (a.raw(), b.raw());
    if ar.unsigned_abs() < 1u64 << 47 && br != 0 && br != -1 {
        FP::from_raw((ar << 16) / br)
    } else {
        a / b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_fp::FrameRng;

    #[test]
    fn div_matches_operator() {
        let mut rng = FrameRng::new(2);
        let mut vals = vec![0i64, 1, -1, 2, -2, 65536, -65536, 1 << 46, -(1 << 46), (1 << 47) - 1, 1 << 47, -(1 << 47)];
        for _ in 0..500 {
            let bits = rng.next_u32() % 63;
            let v = (((rng.next_u32() as u64) << 32 | rng.next_u32() as u64) >> (63 - bits)) as i64;
            vals.push(v);
            vals.push(-v);
        }
        for &a in &vals {
            for &b in &vals {
                if b == 0 {
                    continue;
                }
                let q = a as i128 * 65536 / b as i128;
                if q > i64::MAX as i128 || q < i64::MIN as i128 {
                    continue;
                }
                assert_eq!(div(FP::from_raw(a), FP::from_raw(b)), FP::from_raw(a) / FP::from_raw(b), "{a} / {b}");
            }
        }
    }
}
