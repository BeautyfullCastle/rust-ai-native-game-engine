//! `f32`/`f64` conversions for [`FP`] — **view layer only**.
//!
//! Everything in this module is gated behind the `float-interop` feature
//! and is the *only* place in this crate where float arithmetic is
//! permitted (`#![deny(clippy::float_arithmetic)]` is `allow`ed locally).
//! Use it to get simulation values onto the screen (rendering, UI,
//! editor/debug tooling) or to bring authored float content in as a
//! one-time conversion — never in a loop that feeds results back into
//! simulation state, since float arithmetic is not guaranteed
//! bit-identical across platforms and would reintroduce
//! non-determinism.

#![allow(clippy::float_arithmetic)]

use crate::fp::{FP, SCALE};

impl FP {
    /// Convert from an `f32`. Not deterministic across platforms if fed
    /// back into simulation state — view-layer use only.
    #[must_use]
    pub fn from_f32(v: f32) -> FP {
        FP((v * SCALE as f32) as i64)
    }

    /// Convert to an `f32` (lossy for values needing more than `f32`'s 24
    /// bits of mantissa precision).
    #[must_use]
    pub fn to_f32(self) -> f32 {
        self.0 as f32 / SCALE as f32
    }

    /// Convert from an `f64`. Not deterministic across platforms if fed
    /// back into simulation state — view-layer use only.
    #[must_use]
    pub fn from_f64(v: f64) -> FP {
        FP((v * SCALE as f64) as i64)
    }

    /// Convert to an `f64` (exact for magnitudes small enough to fit in
    /// `f64`'s 52-bit mantissa; lossy only for very large raw values).
    #[must_use]
    pub fn to_f64(self) -> f64 {
        self.0 as f64 / SCALE as f64
    }
}
