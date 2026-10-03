//! `orr_fp`: deterministic fixed-point math for the Orrery simulation.
//!
//! # Why fixed point
//!
//! Floating point (`f32`/`f64`) arithmetic is **not** guaranteed to produce
//! bit-identical results across x86_64, aarch64 and wasm32 (differing FMA
//! usage, differing intermediate precision, differing transcendental
//! function implementations). Orrery's simulation must be deterministic
//! across all supported platforms, so the simulation layer is forbidden
//! from using `f32`/`f64` arithmetic at all. This crate is the *only*
//! numeric type simulation code is allowed to touch.
//!
//! `#![deny(clippy::float_arithmetic)]` is set crate-wide; the only
//! exception is the opt-in [`float_interop`] module (feature
//! `float-interop`), which exists purely to convert to/from `f32`/`f64` at
//! the *view* layer (rendering, UI, editor tooling, content authoring) and
//! must never be used in a way that feeds back into simulation state.
//!
//! Repository CI also runs `tools/check_sim_float_types.py`: a library-only
//! `disallowed_types` check rejects arithmetic-free float type declarations,
//! with the same narrow `float_interop` conversion exception. This is a
//! repository lint contract, not a proof about arbitrary downstream POD types.
//!
//! # The type
//!
//! [`FP`] is a `Q48.16` fixed-point number backed by an `i64`: 16
//! fractional bits, 48 integer bits (well, 47 + sign). Its raw
//! representation is `value * 2^16`, stored as a plain `i64`.
//!
//! [`FP32`] is a smaller `Q16.16` fixed-point number backed by an `i32`,
//! provided for cases that need a compact, `Pod`-friendly 32-bit value
//! (e.g. tightly packed network/save data) with a reduced range.
//!
//! # Rounding rules (read this before touching the arithmetic)
//!
//! - **Multiplication** (`Mul` for `FP`): the two `i64` raw values are
//!   widened to `i128`, multiplied exactly, then arithmetic-shifted right
//!   by 16 (i.e. floor-divided by `2^16`). This means `FP` multiplication
//!   rounds **towards negative infinity**, not towards zero and not to
//!   nearest. This is deliberate: it is cheap, exactly reproducible with
//!   plain integer ops on every target, and matches Rust's `>>` semantics
//!   for signed integers (arithmetic shift).
//! - **Division** (`Div` for `FP`): `((a as i128) << 16) / b`, using
//!   `i128`'s truncating (towards zero) division. Division by zero panics
//!   (there is no sensible deterministic fallback; callers who need a
//!   fallback should use [`FP::checked_div`]).
//! - **`from_ratio`**: rounds to nearest, ties away from zero (see its
//!   doc comment for the exact algorithm). This differs from `Div`
//!   deliberately: `from_ratio` is meant for *converting* a ratio into a
//!   value (e.g. loading content data), where "nearest" is the more
//!   useful default, whereas `Div` is a hot simulation-arithmetic op where
//!   flooring is cheaper and the rounding direction only needs to be
//!   *consistent*, not "nicest".
//! - **`sqrt`**: computed as the exact integer floor of the true square
//!   root via `isqrt` on the widened `u128` representation (see
//!   [`FP::sqrt`] for the exact formula), i.e. it always rounds down.
//! - **Literal parsing** (`fp!` macro and [`FP::parse`]): both go through
//!   the same integer-only decimal parser and round to nearest, ties away
//!   from zero, at `2^-16` granularity. This routine is a `const fn`, so
//!   `fp!(1.5)` is evaluated entirely at compile time via a `const`
//!   binding — there is no runtime float parsing anywhere in this crate.
//! - **Display**: prints a decimal approximation, rounded to at most 5
//!   fractional digits (trailing zeros trimmed), using only integer
//!   arithmetic. This is a display convenience, not a lossless
//!   round-trip: values with mantissas that don't terminate cleanly in 5
//!   decimal digits are rounded for display purposes only.
//!
//! # Determinism guarantees
//!
//! Every operation in this crate (arithmetic, trig, `sqrt`, `exp`/`ln`,
//! the RNG) is implemented with plain integer arithmetic (`i64`/`i128`
//! wrapping/checked ops, integer shifts, integer comparisons) and contains
//! **no** floating point instructions in its execution path (outside of
//! `float-interop`, tests, and benches, which are excluded from the
//! `deny(clippy::float_arithmetic)]` lint). Integer arithmetic has
//! identical, IEEE/architecture-independent semantics on every target
//! Rust supports, so a given sequence of `orr_fp` operations produces the
//! exact same bit pattern on x86_64, aarch64 and wasm32. Transcendental
//! functions ([`FP::sin`], [`FP::cos`], [`FP::atan2`], [`FP::exp`],
//! [`FP::ln`], ...) are implemented with CORDIC / fixed-point polynomial
//! approximations driven entirely by `const` integer lookup tables
//! (see `trig.rs` and `trig_tables.rs`), rather than by
//! calling into libm, so their results are likewise platform-independent.
//!
//! `tests/determinism_golden.rs` pins a checksum over a fixed sequence of
//! operations; CI is expected to run that test on all three target
//! platforms and confirm the checksum matches.

#![cfg_attr(not(any(feature = "std", test)), no_std)]
#![deny(clippy::float_arithmetic)]
#![deny(clippy::disallowed_types)]
#![warn(missing_docs)]
#![allow(clippy::needless_range_loop)]

mod fp;
mod fp32;
mod mat3;
mod parse;
mod quat;
mod rng;
mod trig;
mod trig_tables;
mod vec;

#[cfg(feature = "float-interop")]
pub mod float_interop;

pub use fp::{ParseError, FP};
pub use fp32::FP32;
pub use mat3::FPMat3;
pub use quat::FPQuat;
pub use rng::FrameRng;
pub use vec::{FPVec2, FPVec3};

/// Compile-time fixed-point literal.
///
/// `fp!(1.5)`, `fp!(-0.25)`, `fp!(3)` expand to an [`FP`] value produced by
/// parsing the decimal text of the literal with a `const fn` integer-only
/// parser ([`FP::from_decimal_str`]) evaluated in a `const` binding, so the
/// parse happens entirely at compile time — an invalid literal is a
/// compile error, not a runtime panic.
#[macro_export]
macro_rules! fp {
    ($v:expr) => {{
        const VAL: $crate::FP = $crate::FP::from_decimal_str(::core::stringify!($v));
        VAL
    }};
}
