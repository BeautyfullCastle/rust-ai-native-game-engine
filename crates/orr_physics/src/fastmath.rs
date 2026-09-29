//! Faster integer square root and division for the value ranges physics
//! uses. Both return exactly the same bits as `FP::sqrt` and `FP / FP`
//! for every input (checked by the tests below), and fall back to those
//! operators outside the range where the 64-bit path is exact.

use orr_fp::{FPVec2, FP};

/// `a * b`, bit-identical to the `FP` operator. The 64-bit product is
/// exact whenever it does not overflow (the common case), so the 128-bit
/// widening multiply only runs on the rare overflow path.
#[inline(always)]
pub(crate) fn mul(a: FP, b: FP) -> FP {
    match a.raw().checked_mul(b.raw()) {
        Some(p) => FP::from_raw(p >> 16),
        None => a * b,
    }
}

/// Narrow multiply for the solver: the 64-bit product is assumed not to
/// overflow. That holds for every state inside the documented ranges (the
/// real-valued product of two solver terms stays far below 2^31 units), and
/// then the result has the same bits as `a * b`. A debug build panics on
/// an overflowing product; a release build wraps the same way on every
/// platform, so out-of-range input stays deterministic, just meaningless.
#[inline(always)]
pub(crate) fn nmul(a: FP, b: FP) -> FP {
    debug_assert!(a.raw().checked_mul(b.raw()).is_some(), "orr_physics: solver product out of range");
    FP::from_raw(a.raw().wrapping_mul(b.raw()) >> 16)
}

/// Dot product built on [`mul`], same bits as `dot`.
pub(crate) trait FastVec {
    fn dotf(self, o: FPVec2) -> FP;
}

impl FastVec for FPVec2 {
    #[inline(always)]
    fn dotf(self, o: FPVec2) -> FP {
        mul(self.x, o.x) + mul(self.y, o.y)
    }
}

/// Upper bounds of `sqrt(m)` for normalized `m` in `[2^62, 2^64)`,
/// indexed by `(m >> 55) - 128`. Each entry covers a bucket of relative
/// width 2^-7, so its relative error is below 2^-8.
const SQRT_TABLE: [u64; 384] = build_table();

const fn isqrt_const(n: u128) -> u128 {
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

const fn build_table() -> [u64; 384] {
    let mut t = [0u64; 384];
    let mut i = 0;
    while i < 384 {
        let hi = ((i as u128) + 128 + 1) << 55;
        let mut r = isqrt_const(hi);
        if r * r < hi {
            r += 1;
        }
        t[i] = r as u64;
        i += 1;
    }
    t
}

/// Exact `floor(sqrt(n))`.
#[inline]
fn isqrt_u64(n: u64) -> u64 {
    if n < 2 {
        return n;
    }
    // Normalize by an even shift so the square root shifts by whole bits.
    let lz = n.leading_zeros() & !1;
    let m = n << lz;
    let half = lz / 2;
    let x0 = SQRT_TABLE[(m >> 55) as usize - 128];
    // Ceil-shift keeps the guess an upper bound of sqrt(n).
    let mut x = (x0 + ((1u64 << half) - 1)) >> half;
    // Newton steps from above never drop below floor(sqrt(n)). Two steps
    // bring a 2^-8 relative error far below one unit.
    x = (x + n / x) >> 1;
    x = (x + n / x) >> 1;
    while (x as u128) * (x as u128) > n as u128 {
        x -= 1;
    }
    while ((x + 1) as u128) * ((x + 1) as u128) <= n as u128 {
        x += 1;
    }
    x
}

/// Same result as [`FP::sqrt`].
#[inline]
pub(crate) fn sqrt(x: FP) -> FP {
    let r = x.raw();
    if (0..1i64 << 48).contains(&r) {
        FP::from_raw(isqrt_u64((r as u64) << 16) as i64)
    } else {
        x.sqrt()
    }
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
    fn sqrt_matches_reference() {
        let mut rng = FrameRng::new(1);
        let check = |raw: i64| {
            let x = FP::from_raw(raw);
            assert_eq!(sqrt(x), x.sqrt(), "raw {raw}");
        };
        for raw in 0..200_000i64 {
            check(raw);
        }
        for k in 0..63 {
            for d in -3..=3i64 {
                check(((1i64 << k) + d).max(0));
            }
        }
        check(i64::MAX);
        check((1i64 << 48) - 1);
        check(1i64 << 48);
        for _ in 0..3_000_000 {
            let bits = rng.next_u32() % 62;
            let raw = (((rng.next_u32() as u64) << 32 | rng.next_u32() as u64) >> (63 - bits)) as i64;
            check(raw);
        }
        // Perfect squares and their neighbours.
        for s in (1u64..(1 << 24)).step_by(97) {
            let sq = ((s * s) >> 16) as i64;
            check(sq);
            check(sq + 1);
            check((sq - 1).max(0));
        }
    }

    #[test]
    fn div_matches_operator() {
        let mut rng = FrameRng::new(2);
        let mut vals = vec![0i64, 1, -1, 2, -2, 65536, -65536, 1 << 46, -(1 << 46), (1 << 47) - 1, 1 << 47, -(1 << 47), i64::MAX, i64::MIN + 1];
        for _ in 0..2000 {
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
                let (fa, fb) = (FP::from_raw(a), FP::from_raw(b));
                if a as i128 * 65536 / b as i128 > i64::MAX as i128 || (a as i128 * 65536 / b as i128) < i64::MIN as i128 {
                    continue; // the operator itself overflows here
                }
                assert_eq!(div(fa, fb), fa / fb, "{a} / {b}");
            }
        }
    }
}
