use core::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Rem, RemAssign, Sub, SubAssign};

use bytemuck::{Pod, Zeroable};

/// Number of fractional bits: `Q48.16`.
pub(crate) const FRAC_BITS: u32 = 16;
/// `2^16`, the fixed-point scale factor.
pub(crate) const SCALE: i64 = 1 << FRAC_BITS;

/// A deterministic `Q48.16` fixed-point number backed by an `i64`.
///
/// See the crate-level docs for the full rounding-rule and determinism
/// contract. In short: `raw() as f64 / 65536.0` is the represented value,
/// and every operation on `FP` is implemented with plain integer
/// arithmetic only.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Pod, Zeroable)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FP(pub i64);

/// Error returned by [`FP::parse`] when a string is not a valid decimal
/// fixed-point literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// The input string was empty (or empty after a sign).
    Empty,
    /// A character was encountered that is not `[0-9.+-]`.
    InvalidChar,
    /// More than one `.` was found.
    TooManyDots,
    /// The integer part overflowed `i64`.
    Overflow,
}

impl FP {
    /// The raw scale factor, `2^16`.
    pub const SCALE: i64 = SCALE;

    /// `0.0`
    pub const ZERO: FP = FP(0);
    /// `1.0`
    pub const ONE: FP = FP(SCALE);
    /// `0.5`
    pub const HALF: FP = FP(SCALE / 2);
    /// `2.0`
    pub const TWO: FP = FP(SCALE * 2);
    /// `-1.0`
    pub const MINUS_ONE: FP = FP(-SCALE);
    /// The smallest representable positive value (one raw unit, `2^-16`).
    pub const EPSILON: FP = FP(1);
    /// The largest representable value.
    pub const MAX: FP = FP(i64::MAX);
    /// The smallest (most negative) representable value.
    pub const MIN: FP = FP(i64::MIN);

    /// `pi`, rounded to nearest at `2^-16` granularity.
    pub const PI: FP = FP(205_887);
    /// `2 * pi`, rounded to nearest at `2^-16` granularity.
    pub const TWO_PI: FP = FP(411_775);
    /// `pi / 2`, rounded to nearest at `2^-16` granularity.
    pub const HALF_PI: FP = FP(102_944);
    /// Multiply a degrees value by this to get radians (`pi / 180`).
    pub const DEG2RAD: FP = FP(1_144);
    /// Multiply a radians value by this to get degrees (`180 / pi`).
    pub const RAD2DEG: FP = FP(3_754_936);

    /// Build an `FP` directly from its raw `i64` representation
    /// (`value = raw as f64 / 65536.0`).
    #[inline]
    #[must_use]
    pub const fn from_raw(raw: i64) -> FP {
        FP(raw)
    }

    /// The raw `i64` representation (`value * 65536`, floored).
    #[inline]
    #[must_use]
    pub const fn raw(self) -> i64 {
        self.0
    }

    /// Build an `FP` from an integer.
    #[inline]
    #[must_use]
    pub const fn from_int(v: i32) -> FP {
        FP((v as i64) * SCALE)
    }

    /// Truncate towards negative infinity to the nearest integer (floor).
    #[inline]
    #[must_use]
    pub const fn to_int(self) -> i32 {
        // Arithmetic right shift on a signed integer floors.
        (self.0 >> FRAC_BITS) as i32
    }

    /// Build an `FP` from a ratio `num / den`, rounding to nearest with
    /// ties rounding away from zero.
    ///
    /// This is distinct from [`FP::div`](Div) (which floors towards
    /// negative infinity for speed): `from_ratio` is intended for
    /// converting authored/loaded ratios (e.g. content data) into an `FP`
    /// value, where "round to nearest" is the more intuitive default.
    ///
    /// Algorithm: compute `num * 65536` exactly in `i128`, then divide by
    /// `den` rounding half-away-from-zero: add `den/2` (with the sign of
    /// the result) before truncating division.
    ///
    /// Panics if `den == 0` or the result overflows `i64`.
    #[inline]
    #[must_use]
    pub const fn from_ratio(num: i64, den: i64) -> FP {
        assert!(den != 0, "FP::from_ratio: division by zero");
        let numerator = (num as i128) * (SCALE as i128);
        let denom = den as i128;
        let half = denom / 2;
        let biased = if (numerator >= 0) == (denom >= 0) {
            numerator + half
        } else {
            numerator - half
        };
        let result = biased / denom;
        assert!(
            result >= i64::MIN as i128 && result <= i64::MAX as i128,
            "FP::from_ratio: overflow"
        );
        FP(result as i64)
    }

    // ---- basic queries ----------------------------------------------

    /// Absolute value.
    #[inline]
    #[must_use]
    pub const fn abs(self) -> FP {
        FP(self.0.wrapping_abs())
    }

    /// `-1`, `0` or `1` depending on the sign.
    #[inline]
    #[must_use]
    pub const fn signum(self) -> FP {
        if self.0 > 0 {
            FP::ONE
        } else if self.0 < 0 {
            FP::MINUS_ONE
        } else {
            FP::ZERO
        }
    }

    /// The smaller of two values.
    #[inline]
    #[must_use]
    pub const fn min(self, other: FP) -> FP {
        if self.0 < other.0 {
            self
        } else {
            other
        }
    }

    /// The larger of two values.
    #[inline]
    #[must_use]
    pub const fn max(self, other: FP) -> FP {
        if self.0 > other.0 {
            self
        } else {
            other
        }
    }

    /// Clamp `self` into `[lo, hi]`.
    #[inline]
    #[must_use]
    pub const fn clamp(self, lo: FP, hi: FP) -> FP {
        debug_assert!(lo.0 <= hi.0, "FP::clamp: lo > hi");
        self.max(lo).min(hi)
    }

    /// Floor to the nearest integer, as an `FP`.
    #[inline]
    #[must_use]
    pub const fn floor(self) -> FP {
        FP((self.0 >> FRAC_BITS) << FRAC_BITS)
    }

    /// Ceiling to the nearest integer, as an `FP`.
    #[inline]
    #[must_use]
    pub const fn ceil(self) -> FP {
        let floored = self.floor();
        if floored.0 == self.0 {
            floored
        } else {
            FP(floored.0 + SCALE)
        }
    }

    /// Round to the nearest integer, ties away from zero.
    #[inline]
    #[must_use]
    pub const fn round(self) -> FP {
        if self.0 >= 0 {
            FP(((self.0 + SCALE / 2) >> FRAC_BITS) << FRAC_BITS)
        } else {
            FP(-((((-self.0) + SCALE / 2) >> FRAC_BITS) << FRAC_BITS))
        }
    }

    /// The fractional part, `self - self.floor()`, always in `[0, 1)`.
    #[inline]
    #[must_use]
    pub const fn fract(self) -> FP {
        FP(self.0 - self.floor().0)
    }

    /// Linear interpolation between `self` and `other` by `t` (not
    /// clamped: `t` outside `[0, 1]` extrapolates).
    #[inline]
    #[must_use]
    pub const fn lerp(self, other: FP, t: FP) -> FP {
        // self + (other - self) * t, using widened intermediate math to
        // avoid intermediate overflow on large values.
        let diff = (other.0 as i128) - (self.0 as i128);
        let scaled = (diff * (t.0 as i128)) >> FRAC_BITS;
        FP((self.0 as i128 + scaled) as i64)
    }

    // ---- multiplication / division -----------------------------------

    /// Widened `i128` multiply: exact product, floor-shifted right by 16
    /// bits. This is the rounding rule used by the `Mul` operator.
    #[inline]
    #[must_use]
    pub const fn mul(self, rhs: FP) -> FP {
        let product = (self.0 as i128) * (rhs.0 as i128);
        FP((product >> FRAC_BITS) as i64)
    }

    /// Fast multiplication using only `i64` intermediate math (no `i128`
    /// widening). Faster than [`FP::mul`] but only safe when the true
    /// mathematical product fits in `i64` before the shift; a
    /// `debug_assert` catches overflow in debug builds only.
    #[inline]
    #[must_use]
    pub fn mul_fast(self, rhs: FP) -> FP {
        let a = self.0;
        let b = rhs.0;
        let (product, overflowed) = a.overflowing_mul(b);
        debug_assert!(!overflowed, "FP::mul_fast: i64 overflow, use FP::mul");
        FP(product >> FRAC_BITS)
    }

    /// Division, truncating towards zero. Panics if `rhs` is zero.
    #[inline]
    #[must_use]
    pub const fn div(self, rhs: FP) -> FP {
        assert!(rhs.0 != 0, "FP division by zero");
        let numerator = (self.0 as i128) << FRAC_BITS;
        FP((numerator / (rhs.0 as i128)) as i64)
    }

    /// Multiply by a plain integer.
    #[inline]
    #[must_use]
    pub const fn mul_int(self, rhs: i32) -> FP {
        FP(self.0 * (rhs as i64))
    }

    /// Divide by a plain integer. Panics if `rhs == 0`.
    #[inline]
    #[must_use]
    pub const fn div_int(self, rhs: i32) -> FP {
        assert!(rhs != 0, "FP division by zero");
        FP(self.0 / (rhs as i64))
    }

    // ---- checked ops ---------------------------------------------------

    /// Checked addition.
    #[inline]
    #[must_use]
    pub const fn checked_add(self, rhs: FP) -> Option<FP> {
        match self.0.checked_add(rhs.0) {
            Some(v) => Some(FP(v)),
            None => None,
        }
    }

    /// Checked subtraction.
    #[inline]
    #[must_use]
    pub const fn checked_sub(self, rhs: FP) -> Option<FP> {
        match self.0.checked_sub(rhs.0) {
            Some(v) => Some(FP(v)),
            None => None,
        }
    }

    /// Checked multiplication (see [`FP::mul`] for rounding rule).
    #[inline]
    #[must_use]
    pub const fn checked_mul(self, rhs: FP) -> Option<FP> {
        let product = (self.0 as i128) * (rhs.0 as i128);
        let shifted = product >> FRAC_BITS;
        if shifted > i64::MAX as i128 || shifted < i64::MIN as i128 {
            None
        } else {
            Some(FP(shifted as i64))
        }
    }

    /// Checked division. Returns `None` if `rhs` is zero or the result
    /// overflows.
    #[inline]
    #[must_use]
    pub const fn checked_div(self, rhs: FP) -> Option<FP> {
        if rhs.0 == 0 {
            return None;
        }
        let numerator = (self.0 as i128) << FRAC_BITS;
        let result = numerator / (rhs.0 as i128);
        if result > i64::MAX as i128 || result < i64::MIN as i128 {
            None
        } else {
            Some(FP(result as i64))
        }
    }

    // ---- saturating ops -------------------------------------------------

    /// Saturating addition.
    #[inline]
    #[must_use]
    pub const fn saturating_add(self, rhs: FP) -> FP {
        FP(self.0.saturating_add(rhs.0))
    }

    /// Saturating subtraction.
    #[inline]
    #[must_use]
    pub const fn saturating_sub(self, rhs: FP) -> FP {
        FP(self.0.saturating_sub(rhs.0))
    }

    /// Saturating multiplication.
    #[inline]
    #[must_use]
    pub const fn saturating_mul(self, rhs: FP) -> FP {
        let product = (self.0 as i128) * (rhs.0 as i128);
        let shifted = product >> FRAC_BITS;
        if shifted > i64::MAX as i128 {
            FP::MAX
        } else if shifted < i64::MIN as i128 {
            FP::MIN
        } else {
            FP(shifted as i64)
        }
    }

    /// Saturating division. `rhs == 0` saturates towards `FP::MAX` (matching
    /// the sign of `self`), rather than panicking.
    #[inline]
    #[must_use]
    pub const fn saturating_div(self, rhs: FP) -> FP {
        if rhs.0 == 0 {
            return if self.0 >= 0 { FP::MAX } else { FP::MIN };
        }
        let numerator = (self.0 as i128) << FRAC_BITS;
        let result = numerator / (rhs.0 as i128);
        if result > i64::MAX as i128 {
            FP::MAX
        } else if result < i64::MIN as i128 {
            FP::MIN
        } else {
            FP(result as i64)
        }
    }
}

// ---- operator overloads -------------------------------------------------

impl Add for FP {
    type Output = FP;
    #[inline]
    fn add(self, rhs: FP) -> FP {
        FP(self.0 + rhs.0)
    }
}

impl Sub for FP {
    type Output = FP;
    #[inline]
    fn sub(self, rhs: FP) -> FP {
        FP(self.0 - rhs.0)
    }
}

impl Neg for FP {
    type Output = FP;
    #[inline]
    fn neg(self) -> FP {
        FP(-self.0)
    }
}

impl Mul for FP {
    type Output = FP;
    #[inline]
    fn mul(self, rhs: FP) -> FP {
        FP::mul(self, rhs)
    }
}

impl Div for FP {
    type Output = FP;
    #[inline]
    fn div(self, rhs: FP) -> FP {
        FP::div(self, rhs)
    }
}

impl Rem for FP {
    type Output = FP;
    #[inline]
    fn rem(self, rhs: FP) -> FP {
        FP(self.0 % rhs.0)
    }
}

impl Mul<i32> for FP {
    type Output = FP;
    #[inline]
    fn mul(self, rhs: i32) -> FP {
        FP::mul_int(self, rhs)
    }
}

impl Div<i32> for FP {
    type Output = FP;
    #[inline]
    fn div(self, rhs: i32) -> FP {
        FP::div_int(self, rhs)
    }
}

impl AddAssign for FP {
    #[inline]
    fn add_assign(&mut self, rhs: FP) {
        self.0 += rhs.0;
    }
}
impl SubAssign for FP {
    #[inline]
    fn sub_assign(&mut self, rhs: FP) {
        self.0 -= rhs.0;
    }
}
impl MulAssign for FP {
    #[inline]
    fn mul_assign(&mut self, rhs: FP) {
        *self = *self * rhs;
    }
}
impl DivAssign for FP {
    #[inline]
    fn div_assign(&mut self, rhs: FP) {
        *self = *self / rhs;
    }
}
impl RemAssign for FP {
    #[inline]
    fn rem_assign(&mut self, rhs: FP) {
        self.0 %= rhs.0;
    }
}
impl MulAssign<i32> for FP {
    #[inline]
    fn mul_assign(&mut self, rhs: i32) {
        *self = *self * rhs;
    }
}
impl DivAssign<i32> for FP {
    #[inline]
    fn div_assign(&mut self, rhs: i32) {
        *self = *self / rhs;
    }
}
