//! Transcendental functions for [`FP`], implemented with integer-only
//! lookup tables (`sin`/`cos`/`atan`, bit-identical to their CORDIC
//! references) and fixed-point series (for `exp`/`ln`), driven by `const`
//! tables.
//!
//! No floating point instruction appears anywhere in this module, so
//! results are bit-identical across x86_64, aarch64 and wasm32.
//!
//! # Algorithm notes
//!
//! `sin`/`cos`/`tan` run on a lookup table (see below). The CORDIC version
//! is kept as [`FP::sin_cos_cordic`] and is the reference the table path
//! reproduces bit for bit. Its recurrence is classic rotation-mode CORDIC: starting from the
//! vector `(K, 0)` (`K` is the CORDIC gain, pre-multiplied in so the final
//! `x`/`y` are directly `cos`/`sin`) and rotating it towards the target
//! angle in 17 micro-steps of `atan(2^-i)`, accumulated from a `const`
//! table. The angle is first reduced to `(-pi, pi]` and then folded into
//! `[0, pi/2]` using standard trig symmetries, since CORDIC rotation mode
//! only converges for angles in that range.
//!
//! The fast path (`sin_cos`) reduces the angle to `[0, pi/2]` (raw units
//! `2^-16` rad), then picks the nearest of 1610 table nodes spaced `2^-10`
//! rad apart. Each node stores `sin` and `cos` with 32 fractional bits.
//! The offset `d` (at most `2^-11` rad) is applied with a second-order
//! Taylor step: `sin(a+d) = S + C*d - S*d^2/2`, `cos(a+d) = C - S*d - C*d^2/2`.
//! The truncated cubic term is below `2^-35`, so the result before the
//! final round to `Q48.16` is accurate to about `2^-32`. That rounds to
//! the CORDIC result at every input except a few that CORDIC itself
//! rounds the wrong way. A small `const` list of `(raw input, +-1)`
//! fixes patches those, guarded by a bitmap so the common call pays one
//! bit test. `tests/trig_lut.rs` checks all `102945` inputs against
//! CORDIC and regenerates the tables (`src/trig_tables.rs`).
//!
//! `atan` runs on a lookup table too. It first reduces `|x| > 1` via
//! `atan(x) = pi/2 - atan(1/x)`, using the same truncating division as the
//! CORDIC version (as a 64-bit divide `2^32 / |x|`, which gives the same
//! value), so only `t` in `[0, 1]` (raw `[0, 65536]`) needs a table. That
//! range has 1025 nodes spaced `2^-10` apart, storing `atan(a)`, `1/(1+a^2)`
//! and `a/(1+a^2)^2`. The offset `d` (at most `2^-11`) uses the same
//! second-order step: `atan(a+d) = A + d/(1+a^2) - d^2 * a/(1+a^2)^2`, with
//! a truncated cubic term below `2^-34`. A `const` exception list guarded
//! by a bitmap (61 entries) patches the inputs where the rounded result
//! differs from CORDIC. `tests/trig_lut.rs` checks all `65537` reduced
//! inputs and a wide range of `atan2` inputs against the CORDIC references
//! [`FP::atan_cordic`] and [`FP::atan2_cordic`], which use vectoring-mode
//! CORDIC (same recurrence as rotation mode, opposite driving rule: drive
//! `y` to zero instead of `z`). `atan2` is the standard `atan(y/x)` plus
//! quadrant correction, and `asin`/`acos` go through
//! `asin(x) = atan(x / sqrt(1 - x^2))`.
//!
//! `exp`/`ln` use range reduction against `ln(2)` (`exp`: `x = n*ln2 + r`,
//! `2^n` applied as a raw bit shift, `exp(r)` via a degree-6 Maclaurin
//! polynomial for `|r| <= ln2/2`; `ln`: reduce `x = m * 2^e` with `m` in
//! `[1, 2)` by inspecting the raw value's highest set bit, then
//! `ln(m) = 2*atanh((m-1)/(m+1))`, a rapidly-converging series since
//! `t <= 1/3` on that range).
//!
//! All lookup-table constants below were generated once from `f64` (see
//! the doc comment on each table) and are committed as plain integer
//! literals — there is no runtime or build-time float dependency.

use crate::fp::FP;
use crate::trig_tables::{
    ATAN_D1, ATAN_D2, ATAN_FIX, ATAN_TAB, COS_FIX, COS_TAB, SIN_FIX, SIN_TAB,
};

/// Number of CORDIC iterations, run at the *extended* internal precision
/// (see [`EXT_SCALE`]). `atan(2^-i)` at that precision only underflows to
/// 0 raw units around `i ~= 33`; 26 iterations is already well past the
/// point where further iterations would change the final `Q48.16` result
/// (which is truncated back down from 32 fractional bits at the end of
/// each function), while keeping the per-call cost small.
const N: usize = 26;

/// `atan(2^-i) * 2^32`, rounded to nearest, for `i in 0..26`. Generated
/// once via `atan(2.0_f64.powi(-i)) * (1u64 << 32) as f64`.
const ATAN_TABLE: [i64; N] = [
    3_373_259_426,
    1_991_351_318,
    1_052_175_346,
    534_100_635,
    268_086_748,
    134_174_063,
    67_103_403,
    33_553_749,
    16_777_131,
    8_388_597,
    4_194_303,
    2_097_152,
    1_048_576,
    524_288,
    262_144,
    131_072,
    65_536,
    32_768,
    16_384,
    8_192,
    4_096,
    2_048,
    1_024,
    512,
    256,
    128,
];

/// CORDIC gain `K = prod_{i=0}^{25} 1/sqrt(1 + 2^-2i)`, times `2^32`,
/// rounded to nearest. Pre-multiplying it into the initial vector means
/// the CORDIC rotation directly produces `cos`/`sin` without a separate
/// gain-correction multiply.
const CORDIC_K: i64 = 2_608_131_496;

/// Internal CORDIC/series working precision: 32 fractional bits (double
/// `FP`'s own 16), used only inside this module so that per-iteration
/// rounding in the CORDIC recurrence, and in the `exp`/`ln` range
/// reduction, doesn't erode the final `Q48.16` result below its target
/// accuracy. Every internal value still comfortably fits in `i64` (the
/// largest magnitude used here, `pi/2 * 2^32`, is about `6.7e9`).
const EXT_BITS: u32 = 32;
const EXT_SCALE: i64 = 1 << EXT_BITS;
const EXT_FROM_FP_SHIFT: u32 = EXT_BITS - 16;

/// `ln(2) * 65536`, rounded to nearest.
const LN2_RAW: i64 = 45_426;

/// Round-shift `v` (assumed `>= 0`) right by `EXT_FROM_FP_SHIFT` bits,
/// converting an extended-precision (`2^32`) raw value back down to a
/// `Q48.16` (`2^16`) raw value.
#[inline]
fn round_down_from_ext(v: i64) -> i64 {
    (v + (1 << (EXT_FROM_FP_SHIFT - 1))) >> EXT_FROM_FP_SHIFT
}

/// Table node spacing: `2^SEG_SHIFT` raw units (`2^-10` rad).
pub(crate) const SEG_SHIFT: u32 = 6;

/// Exceptions are flagged per block of `2^FIX_BLOCK_SHIFT` raw inputs.
const FIX_BLOCK_SHIFT: u32 = 4;
const FIX_WORDS: usize = ((FP::HALF_PI.0 as usize >> FIX_BLOCK_SHIFT) >> 6) + 1;

const fn fix_bitmap(fix: &[(u32, i8)]) -> [u64; FIX_WORDS] {
    let mut map = [0u64; FIX_WORDS];
    let mut i = 0;
    while i < fix.len() {
        let block = (fix[i].0 >> FIX_BLOCK_SHIFT) as usize;
        map[block >> 6] |= 1u64 << (block & 63);
        i += 1;
    }
    map
}

const SIN_FIX_MAP: [u64; FIX_WORDS] = fix_bitmap(&SIN_FIX);
const COS_FIX_MAP: [u64; FIX_WORDS] = fix_bitmap(&COS_FIX);

/// The `atan` table covers `t` in `[0, 1]` (raw `[0, 2^16]`), nodes `2^-10` apart.
const ATAN_FIX_WORDS: usize = ((FP::ONE.0 as usize >> FIX_BLOCK_SHIFT) >> 6) + 1;

const fn atan_fix_bitmap(fix: &[(u32, i8)]) -> [u64; ATAN_FIX_WORDS] {
    let mut map = [0u64; ATAN_FIX_WORDS];
    let mut i = 0;
    while i < fix.len() {
        let block = (fix[i].0 >> FIX_BLOCK_SHIFT) as usize;
        map[block >> 6] |= 1u64 << (block & 63);
        i += 1;
    }
    map
}

const ATAN_FIX_MAP: [u64; ATAN_FIX_WORDS] = atan_fix_bitmap(&ATAN_FIX);

/// Correction for `raw` from a sorted exception list, guarded by `map`.
#[inline]
fn fix_delta(map: &[u64; FIX_WORDS], fix: &[(u32, i8)], raw: u32) -> i64 {
    let block = (raw >> FIX_BLOCK_SHIFT) as usize;
    if (map[block >> 6] >> (block & 63)) & 1 == 0 {
        return 0;
    }
    match fix.binary_search_by_key(&raw, |e| e.0) {
        Ok(i) => i64::from(fix[i].1),
        Err(_) => 0,
    }
}

/// Table interpolation without the exception fix-ups. `folded` must be in
/// `[0, HALF_PI]` raw. Returns `(cos_raw, sin_raw)`.
#[inline]
fn lut_sin_cos_unpatched(folded: i64) -> (i64, i64) {
    let i = ((folded + (1 << (SEG_SHIFT - 1))) >> SEG_SHIFT) as usize;
    let d = folded - ((i as i64) << SEG_SHIFT);
    let (s, c) = (SIN_TAB[i], COS_TAB[i]);
    let dd = d * d;
    let sin_ext = s + ((c * d) >> 16) - ((s * dd) >> 33);
    let cos_ext = c - ((s * d) >> 16) - ((c * dd) >> 33);
    (round_down_from_ext(cos_ext), round_down_from_ext(sin_ext))
}

/// Table `sin`/`cos` for `folded` in `[0, HALF_PI]` raw, equal to the
/// CORDIC result at every input. Returns `(cos_raw, sin_raw)`.
#[inline]
fn lut_sin_cos(folded: i64) -> (i64, i64) {
    let (cos, sin) = lut_sin_cos_unpatched(folded);
    let raw = folded as u32;
    (
        cos + fix_delta(&COS_FIX_MAP, &COS_FIX, raw),
        sin + fix_delta(&SIN_FIX_MAP, &SIN_FIX, raw),
    )
}

/// Table `atan` for `t` in `[0, ONE]` raw, without the exception fix-ups.
/// Second-order Taylor step around the nearest node `a`:
/// `atan(a+d) = A + d/(1+a^2) - d^2 * a/(1+a^2)^2`. `ATAN_D1` has 31
/// and `ATAN_D2` 32 fractional bits; `d` is in raw units (`2^-16`).
#[inline]
fn lut_atan_unpatched(t: i64) -> i64 {
    let i = ((t + (1 << (SEG_SHIFT - 1))) >> SEG_SHIFT) as usize;
    let d = t - ((i as i64) << SEG_SHIFT);
    let atan_ext = ATAN_TAB[i] + ((d * i64::from(ATAN_D1[i])) >> 15)
        - ((d * d * i64::from(ATAN_D2[i])) >> 32);
    round_down_from_ext(atan_ext)
}

/// Table `atan` for `t` in `[0, ONE]` raw, equal to the CORDIC result at
/// every input.
#[inline]
fn lut_atan(t: i64) -> i64 {
    let raw = t as u32;
    let block = (raw >> FIX_BLOCK_SHIFT) as usize;
    let mut r = lut_atan_unpatched(t);
    if (ATAN_FIX_MAP[block >> 6] >> (block & 63)) & 1 != 0 {
        if let Ok(i) = ATAN_FIX.binary_search_by_key(&raw, |e| e.0) {
            r += i64::from(ATAN_FIX[i].1);
        }
    }
    r
}

/// Rotation-mode CORDIC: rotate `(CORDIC_K, 0)` by `theta` (extended
/// precision raw, must be in `[0, HALF_PI]`). Returns
/// `(cos_raw, sin_raw)`, also in extended precision, both non-negative.
fn cordic_rotate_ext(theta: i64) -> (i64, i64) {
    let mut x: i64 = CORDIC_K;
    let mut y: i64 = 0;
    let mut z: i64 = theta;
    for i in 0..N {
        let d: i64 = if z >= 0 { 1 } else { -1 };
        let x_shift = x >> i;
        let y_shift = y >> i;
        let x_new = x - d * y_shift;
        let y_new = y + d * x_shift;
        x = x_new;
        y = y_new;
        z -= d * ATAN_TABLE[i];
    }
    (x, y)
}

/// Vectoring-mode CORDIC: rotate `(x0, y0)` (`x0 > 0`, `y0 >= 0`, both
/// extended precision) towards the x-axis, returning the accumulated
/// rotation angle (`atan2(y0, x0)`, extended precision raw).
fn cordic_vector_ext(x0: i64, y0: i64) -> i64 {
    let mut x = x0;
    let mut y = y0;
    let mut z: i64 = 0;
    for i in 0..N {
        let d: i64 = if y >= 0 { -1 } else { 1 };
        let x_shift = x >> i;
        let y_shift = y >> i;
        let x_new = x - d * y_shift;
        let y_new = y + d * x_shift;
        x = x_new;
        y = y_new;
        z -= d * ATAN_TABLE[i];
    }
    z
}

/// Classic bit-by-bit (digit-by-digit) integer square root: the exact
/// floor of the true square root of `n`, computed with only shifts,
/// comparisons, subtraction and addition.
fn isqrt_u128(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    let mut res: u128 = 0;
    let mut bit: u128 = 1u128 << 126;
    while bit > n {
        bit >>= 2;
    }
    let mut rem = n;
    while bit != 0 {
        if rem >= res + bit {
            rem -= res + bit;
            res = (res >> 1) + bit;
        } else {
            res >>= 1;
        }
        bit >>= 2;
    }
    res
}

impl FP {
    /// Reduce `self` (radians) into `(-pi, pi]`.
    #[must_use]
    pub fn wrap_angle(self) -> FP {
        let two_pi = FP::TWO_PI.0;
        if self.0 > -FP::PI.0 && self.0 <= FP::PI.0 {
            return self;
        }
        let mut r = self.0 % two_pi;
        if r > FP::PI.0 {
            r -= two_pi;
        } else if r <= -FP::PI.0 {
            r += two_pi;
        }
        FP(r)
    }

    /// Fold `self` into `[0, pi/2]`: `(input negative, cos sign, folded raw)`.
    #[inline]
    fn fold_quadrant(self) -> (bool, i64, i64) {
        let wrapped = self.wrap_angle();
        let a_abs_raw = wrapped.0.wrapping_abs();
        if a_abs_raw > FP::HALF_PI.0 {
            (wrapped.0 < 0, -1, FP::PI.0 - a_abs_raw)
        } else {
            (wrapped.0 < 0, 1, a_abs_raw)
        }
    }

    /// Sine and cosine of `self` (radians) computed together, from a
    /// lookup table (see the module docs). Bit-identical to
    /// [`FP::sin_cos_cordic`]. Accuracy: absolute error `<= 2^-14` versus
    /// `f64` (in fact at most `0.5016` raw units, about `2^-16`).
    #[must_use]
    pub fn sin_cos(self) -> (FP, FP) {
        let (neg, cos_sign, folded_raw) = self.fold_quadrant();
        // `TWO_PI` is one raw unit above `2 * PI`, so `wrap_angle` maps
        // `-PI - k*TWO_PI` to `PI + 1` and the fold lands on `-1`, outside
        // the table. CORDIC handles it, and matching it keeps goldens intact.
        if folded_raw < 0 {
            return self.sin_cos_cordic();
        }
        let (cos_raw, sin_raw) = lut_sin_cos(folded_raw);
        (FP(if neg { -sin_raw } else { sin_raw }), FP(cos_sign * cos_raw))
    }

    /// Reference `sin_cos` using 26-step CORDIC (about 15x slower than
    /// [`FP::sin_cos`]). Kept so tests can check the table path against it.
    #[must_use]
    pub fn sin_cos_cordic(self) -> (FP, FP) {
        let (neg, cos_sign, folded_raw) = self.fold_quadrant();
        let (cos_ext, sin_ext) = cordic_rotate_ext(folded_raw << EXT_FROM_FP_SHIFT);
        let cos_raw = round_down_from_ext(cos_ext);
        let sin_raw = round_down_from_ext(sin_ext);
        (FP(if neg { -sin_raw } else { sin_raw }), FP(cos_sign * cos_raw))
    }

    /// Table interpolation without exception fix-ups, for the table
    /// generator in `tests/trig_lut.rs`. `self` must be in `[0, pi/2]`.
    #[doc(hidden)]
    #[must_use]
    pub fn sin_cos_lut_unpatched(self) -> (FP, FP) {
        let (cos, sin) = lut_sin_cos_unpatched(self.0);
        (FP(sin), FP(cos))
    }

    /// Sine of `self` (radians). See [`FP::sin_cos`] for accuracy.
    #[must_use]
    pub fn sin(self) -> FP {
        self.sin_cos().0
    }

    /// Cosine of `self` (radians). See [`FP::sin_cos`] for accuracy.
    #[must_use]
    pub fn cos(self) -> FP {
        self.sin_cos().1
    }

    /// Tangent of `self` (radians). Panics if `cos(self)` is exactly zero
    /// (matches [`FP`]'s division-by-zero policy).
    #[must_use]
    pub fn tan(self) -> FP {
        let (s, c) = self.sin_cos();
        s / c
    }

    /// Arctangent of `self`, result in `(-pi/2, pi/2)` radians, from a
    /// lookup table (see the module docs). Bit-identical to
    /// [`FP::atan_cordic`]. Accuracy: absolute error `<= 2^-12` rad versus
    /// `f64` (in fact about `2^-16`).
    #[must_use]
    pub fn atan(self) -> FP {
        let t_abs = self.abs();
        // `FP::MIN` has no positive counterpart; only the reference handles it.
        if t_abs.0 < 0 {
            return self.atan_cordic();
        }
        let base = if t_abs.0 > FP::ONE.0 {
            // Same value as `FP::ONE / t_abs`, with a cheaper 64-bit divide.
            let inv = (1u64 << 32) / t_abs.0 as u64;
            FP::HALF_PI.0 - lut_atan(inv as i64)
        } else {
            lut_atan(t_abs.0)
        };
        FP(if self.0 < 0 { -base } else { base })
    }

    /// Reference `atan` using vectoring-mode CORDIC (much slower than
    /// [`FP::atan`]). Kept so tests can check the table path against it.
    #[must_use]
    pub fn atan_cordic(self) -> FP {
        let neg = self.0 < 0;
        let t_abs = self.abs();
        let (t2, add_half_pi) = if t_abs.0 > FP::ONE.0 {
            (FP::ONE / t_abs, true)
        } else {
            (t_abs, false)
        };
        let atan_ext = cordic_vector_ext(EXT_SCALE, t2.0 << EXT_FROM_FP_SHIFT);
        let base = FP(round_down_from_ext(atan_ext));
        let result = if add_half_pi { FP::HALF_PI - base } else { base };
        if neg {
            -result
        } else {
            result
        }
    }

    /// Table interpolation without exception fix-ups, for the table
    /// generator in `tests/trig_lut.rs`. `self` must be in `[0, 1]`.
    #[doc(hidden)]
    #[must_use]
    pub fn atan_lut_unpatched(self) -> FP {
        FP(lut_atan_unpatched(self.0))
    }

    /// `self / x` with a 64-bit divide when `|self| < 2^47`, so `self << 16`
    /// fits in `i64` (same result as [`FP`]'s 128-bit division). `x` must
    /// be non-zero.
    #[inline]
    fn div_fast(self, x: FP) -> FP {
        if self.0.unsigned_abs() < 1 << 47 {
            FP((self.0 << 16) / x.0)
        } else {
            self / x
        }
    }

    /// Two-argument arctangent `atan2(self, x)` (`self` is `y`), result in
    /// `(-pi, pi]` radians, following the standard quadrant convention.
    /// `atan2(0, 0)` is defined as `0`. Bit-identical to
    /// [`FP::atan2_cordic`]. Accuracy: absolute error `<= 2^-12` rad versus
    /// `f64` (away from the branch cut / origin).
    #[must_use]
    pub fn atan2(self, x: FP) -> FP {
        if x.0 > 0 {
            self.div_fast(x).atan()
        } else if x.0 < 0 {
            if self.0 >= 0 {
                self.div_fast(x).atan() + FP::PI
            } else {
                self.div_fast(x).atan() - FP::PI
            }
        } else if self.0 > 0 {
            FP::HALF_PI
        } else if self.0 < 0 {
            -FP::HALF_PI
        } else {
            FP::ZERO
        }
    }

    /// Reference `atan2` built on [`FP::atan_cordic`].
    #[must_use]
    pub fn atan2_cordic(self, x: FP) -> FP {
        if x.0 > 0 {
            (self / x).atan_cordic()
        } else if x.0 < 0 {
            if self.0 >= 0 {
                (self / x).atan_cordic() + FP::PI
            } else {
                (self / x).atan_cordic() - FP::PI
            }
        } else if self.0 > 0 {
            FP::HALF_PI
        } else if self.0 < 0 {
            -FP::HALF_PI
        } else {
            FP::ZERO
        }
    }

    /// Arcsine of `self` (domain `[-1, 1]`), result in `[-pi/2, pi/2]`.
    /// Accuracy: absolute error `<= 2^-12` rad versus `f64`.
    #[must_use]
    pub fn asin(self) -> FP {
        if self.0 >= FP::ONE.0 {
            return FP::HALF_PI;
        }
        if self.0 <= FP::MINUS_ONE.0 {
            return -FP::HALF_PI;
        }
        let one_minus_x2 = FP::ONE - self.mul(self);
        let denom = one_minus_x2.sqrt();
        (self / denom).atan()
    }

    /// Arccosine of `self` (domain `[-1, 1]`), result in `[0, pi]`.
    /// Accuracy: absolute error `<= 2^-12` rad versus `f64`.
    #[must_use]
    pub fn acos(self) -> FP {
        FP::HALF_PI - self.asin()
    }

    /// Exact floor of the true square root, via integer `isqrt` on the
    /// widened `u128` representation: `sqrt(raw * 65536)`. In debug
    /// builds, a negative input triggers a `debug_assert` failure
    /// (release builds return `FP::ZERO`, matching the "no NaN" policy of
    /// this crate).
    #[must_use]
    pub fn sqrt(self) -> FP {
        if self.0 < 0 {
            debug_assert!(false, "FP::sqrt: negative input ({self})");
            return FP::ZERO;
        }
        let widened = (self.0 as u128) << 16;
        FP(isqrt_u128(widened) as i64)
    }

    /// `1 / sqrt(self)`. Panics if `self <= 0` (via the internal division
    /// by `sqrt(self) == 0`).
    #[must_use]
    pub fn inv_sqrt(self) -> FP {
        FP::ONE / self.sqrt()
    }

    /// `e^self`. Accuracy: relative error `<= 2^-12` for inputs in a
    /// sensible simulation range (roughly `|self| < 20`); saturates to
    /// [`FP::MAX`] / [`FP::ZERO`] outside the representable range instead
    /// of overflowing.
    #[must_use]
    pub fn exp(self) -> FP {
        if self.0 == 0 {
            return FP::ONE;
        }
        let ln2 = FP(LN2_RAW);
        let n_fp = (self / ln2).round();
        let n = n_fp.to_int();
        let r = self - FP::from_int(n).mul(ln2);

        let r2 = r.mul(r);
        let r3 = r2.mul(r);
        let r4 = r3.mul(r);
        let r5 = r4.mul(r);
        let r6 = r5.mul(r);

        let mut acc = FP::ONE + r;
        acc += r2.div_int(2);
        acc += r3.div_int(6);
        acc += r4.div_int(24);
        acc += r5.div_int(120);
        acc += r6.div_int(720);

        if n >= 47 {
            FP::MAX
        } else if n >= 0 {
            FP(acc.0 << n)
        } else if n <= -63 {
            FP::ZERO
        } else {
            FP(acc.0 >> (-n))
        }
    }

    /// Natural log of `self`. Panics if `self <= 0`. Accuracy: relative
    /// error `<= 2^-12` across the representable positive range.
    #[must_use]
    pub fn ln(self) -> FP {
        assert!(self.0 > 0, "FP::ln: domain error, x must be positive");
        let bits = 63 - self.0.leading_zeros() as i32;
        let e = bits - 16;
        let m_raw = if e >= 0 { self.0 >> e } else { self.0 << (-e) };
        let m = FP(m_raw);

        let t = (m - FP::ONE) / (m + FP::ONE);
        let t2 = t.mul(t);
        let t3 = t2.mul(t);
        let t5 = t3.mul(t2);
        let t7 = t5.mul(t2);
        let t9 = t7.mul(t2);

        let atanh = t + t3.div_int(3) + t5.div_int(5) + t7.div_int(7) + t9.div_int(9);
        let ln_m = atanh.mul_int(2);
        ln_m + FP::from_int(e).mul(FP(LN2_RAW))
    }

    /// `self^n` for an integer exponent, via exponentiation by squaring.
    /// Negative `n` computes `1 / self^|n|`.
    #[must_use]
    pub fn pow_int(self, n: i32) -> FP {
        if n == 0 {
            return FP::ONE;
        }
        let neg = n < 0;
        let mut e = n.unsigned_abs();
        let mut base = self;
        let mut result = FP::ONE;
        while e > 0 {
            if e & 1 == 1 {
                result *= base;
            }
            base = base * base;
            e >>= 1;
        }
        if neg {
            FP::ONE / result
        } else {
            result
        }
    }
}
