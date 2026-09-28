use core::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign};

use bytemuck::{Pod, Zeroable};

use crate::fp::FP;

const FRAC_BITS: u32 = 16;
const SCALE: i32 = 1 << FRAC_BITS;

/// A compact `Q16.16` fixed-point number backed by an `i32`.
///
/// Minimal companion to [`FP`] for cases that need a small, `Pod`-friendly
/// 32-bit value (packed network/save data) at the cost of reduced range
/// (roughly `+-32767.99998`). Same rounding rules as `FP` (`Mul` widens to
/// `i64` then floor-shifts; `Div` widens to `i64` and truncates).
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Pod, Zeroable, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FP32(pub i32);

impl FP32 {
    /// `0.0`
    pub const ZERO: FP32 = FP32(0);
    /// `1.0`
    pub const ONE: FP32 = FP32(SCALE);
    /// The largest representable value.
    pub const MAX: FP32 = FP32(i32::MAX);
    /// The smallest representable value.
    pub const MIN: FP32 = FP32(i32::MIN);

    /// Build from a raw `i32` (`value = raw as f64 / 65536.0`).
    #[inline]
    #[must_use]
    pub const fn from_raw(raw: i32) -> FP32 {
        FP32(raw)
    }

    /// The raw `i32` representation.
    #[inline]
    #[must_use]
    pub const fn raw(self) -> i32 {
        self.0
    }

    /// Build from an integer.
    #[inline]
    #[must_use]
    pub const fn from_int(v: i16) -> FP32 {
        FP32((v as i32) * SCALE)
    }

    /// Convert from the wider [`FP`] (`Q48.16`) type, truncating any bits
    /// that don't fit in `i32`.
    #[inline]
    #[must_use]
    pub const fn from_fp(v: FP) -> FP32 {
        FP32(v.raw() as i32)
    }

    /// Widen to the full-range [`FP`] (`Q48.16`) type (exact, no loss).
    #[inline]
    #[must_use]
    pub const fn to_fp(self) -> FP {
        FP::from_raw(self.0 as i64)
    }

    /// Absolute value.
    #[inline]
    #[must_use]
    pub const fn abs(self) -> FP32 {
        FP32(self.0.wrapping_abs())
    }

    /// The smaller of two values.
    #[inline]
    #[must_use]
    pub const fn min(self, other: FP32) -> FP32 {
        if self.0 < other.0 {
            self
        } else {
            other
        }
    }

    /// The larger of two values.
    #[inline]
    #[must_use]
    pub const fn max(self, other: FP32) -> FP32 {
        if self.0 > other.0 {
            self
        } else {
            other
        }
    }

    /// Multiply, widened through `i64` then floor-shifted (same rounding
    /// rule as [`FP::mul`]).
    #[inline]
    #[must_use]
    pub const fn mul(self, rhs: FP32) -> FP32 {
        let product = (self.0 as i64) * (rhs.0 as i64);
        FP32((product >> FRAC_BITS) as i32)
    }

    /// Divide, widened through `i64`, truncating towards zero. Panics if
    /// `rhs == 0`.
    #[inline]
    #[must_use]
    pub const fn div(self, rhs: FP32) -> FP32 {
        assert!(rhs.0 != 0, "FP32 division by zero");
        let numerator = (self.0 as i64) << FRAC_BITS;
        FP32((numerator / (rhs.0 as i64)) as i32)
    }

    /// Exact floor of the true square root, computed by widening to
    /// [`FP`] and using [`FP::sqrt`].
    #[inline]
    #[must_use]
    pub fn sqrt(self) -> FP32 {
        FP32::from_fp(self.to_fp().sqrt())
    }
}

impl Add for FP32 {
    type Output = FP32;
    #[inline]
    fn add(self, rhs: FP32) -> FP32 {
        FP32(self.0 + rhs.0)
    }
}
impl Sub for FP32 {
    type Output = FP32;
    #[inline]
    fn sub(self, rhs: FP32) -> FP32 {
        FP32(self.0 - rhs.0)
    }
}
impl Neg for FP32 {
    type Output = FP32;
    #[inline]
    fn neg(self) -> FP32 {
        FP32(-self.0)
    }
}
impl Mul for FP32 {
    type Output = FP32;
    #[inline]
    fn mul(self, rhs: FP32) -> FP32 {
        FP32::mul(self, rhs)
    }
}
impl Div for FP32 {
    type Output = FP32;
    #[inline]
    fn div(self, rhs: FP32) -> FP32 {
        FP32::div(self, rhs)
    }
}
impl AddAssign for FP32 {
    #[inline]
    fn add_assign(&mut self, rhs: FP32) {
        self.0 += rhs.0;
    }
}
impl SubAssign for FP32 {
    #[inline]
    fn sub_assign(&mut self, rhs: FP32) {
        self.0 -= rhs.0;
    }
}
impl MulAssign for FP32 {
    #[inline]
    fn mul_assign(&mut self, rhs: FP32) {
        *self = *self * rhs;
    }
}
impl DivAssign for FP32 {
    #[inline]
    fn div_assign(&mut self, rhs: FP32) {
        *self = *self / rhs;
    }
}
