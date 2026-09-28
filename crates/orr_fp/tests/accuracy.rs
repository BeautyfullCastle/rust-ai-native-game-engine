//! Accuracy sweeps of `orr_fp` transcendental functions vs `f64` libm.
//! This file is a test, not simulation code, so plain float arithmetic is
//! fine here.
#![allow(clippy::float_arithmetic)]

use orr_fp::{fp, FP};

fn to_f64(v: FP) -> f64 {
    v.raw() as f64 / 65536.0
}

fn from_f64(v: f64) -> FP {
    FP::from_raw((v * 65536.0).round() as i64)
}

const SIN_COS_TOL: f64 = 1.0 / (1u64 << 14) as f64; // 2^-14
const ATAN_TOL: f64 = 1.0 / (1u64 << 12) as f64; // 2^-12 rad
const EXP_LN_REL_TOL: f64 = 1.0 / (1u64 << 12) as f64; // 2^-12 relative

fn sweep(lo: f64, hi: f64, steps: i64) -> impl Iterator<Item = f64> {
    (0..=steps).map(move |i| lo + (hi - lo) * (i as f64) / (steps as f64))
}

#[test]
fn sin_cos_accuracy() {
    let mut max_sin_err = 0.0_f64;
    let mut max_cos_err = 0.0_f64;
    for a in sweep(-10.0, 10.0, 2000) {
        let x = from_f64(a);
        let (s, c) = x.sin_cos();
        let sin_err = (to_f64(s) - a.sin()).abs();
        let cos_err = (to_f64(c) - a.cos()).abs();
        max_sin_err = max_sin_err.max(sin_err);
        max_cos_err = max_cos_err.max(cos_err);
        assert!(sin_err <= SIN_COS_TOL, "sin({a}) err {sin_err} raw={s:?}");
        assert!(cos_err <= SIN_COS_TOL, "cos({a}) err {cos_err} raw={c:?}");
    }
    eprintln!("max sin err = {max_sin_err}, max cos err = {max_cos_err}");
}

#[test]
fn tan_accuracy_away_from_asymptotes() {
    for a in sweep(-1.4, 1.4, 500) {
        let x = from_f64(a);
        let t = x.tan();
        let err = (to_f64(t) - a.tan()).abs();
        // tan blows up near +-pi/2; allow a looser bound scaled by magnitude.
        let bound = SIN_COS_TOL * (1.0 + a.tan().abs());
        assert!(err <= bound, "tan({a}) err {err} bound {bound}");
    }
}

#[test]
fn atan_atan2_accuracy() {
    for a in sweep(-50.0, 50.0, 2000) {
        let x = from_f64(a);
        let r = x.atan();
        let err = (to_f64(r) - a.atan()).abs();
        assert!(err <= ATAN_TOL, "atan({a}) err {err}");
    }

    for &(y, x) in &[
        (1.0, 1.0),
        (1.0, -1.0),
        (-1.0, 1.0),
        (-1.0, -1.0),
        (0.0, 1.0),
        (0.0, -1.0),
        (1.0, 0.0),
        (-1.0, 0.0),
        (3.0, 4.0),
        (-3.0, 4.0),
        (3.0, -4.0),
        (-3.0, -4.0),
        (0.001, 5.0),
        (5.0, 0.001),
    ] {
        let fy = from_f64(y);
        let fx = from_f64(x);
        let got = to_f64(fy.atan2(fx));
        let want = y.atan2(x);
        let err = (got - want).abs();
        assert!(err <= ATAN_TOL, "atan2({y},{x}) err {err} got {got} want {want}");
    }
}

#[test]
fn asin_acos_accuracy() {
    for a in sweep(-0.999, 0.999, 1000) {
        let x = from_f64(a);
        let asin_err = (to_f64(x.asin()) - a.asin()).abs();
        let acos_err = (to_f64(x.acos()) - a.acos()).abs();
        assert!(asin_err <= ATAN_TOL, "asin({a}) err {asin_err}");
        assert!(acos_err <= ATAN_TOL, "acos({a}) err {acos_err}");
    }
    assert_eq!(FP::ONE.asin(), FP::HALF_PI);
    assert_eq!(FP::MINUS_ONE.asin(), -FP::HALF_PI);
}

#[test]
fn sqrt_accuracy() {
    for a in sweep(0.0, 10000.0, 3000) {
        let x = from_f64(a);
        let got = to_f64(x.sqrt());
        let want = a.sqrt();
        let err = (got - want).abs();
        // sqrt floors to the nearest 1/65536, so error is bounded by that
        // plus f64<->fixed conversion rounding.
        assert!(err <= 2.0 / 65536.0, "sqrt({a}) err {err}");
    }
    assert_eq!(FP::ZERO.sqrt(), FP::ZERO);
    assert_eq!(FP::ONE.sqrt(), FP::ONE);
    assert_eq!(fp!(4).sqrt(), fp!(2));
}

#[test]
fn exp_accuracy() {
    // Below about x = -2.77, exp(x) < 1/4096 in raw units, so a single
    // raw-unit rounding step in the Q48.16 *result* itself already
    // exceeds a 2^-12 relative budget — that's a representation floor,
    // not an algorithm error, so the sweep sticks to the range where
    // relative accuracy is meaningful.
    for a in sweep(-2.5, 15.0, 2000) {
        let x = from_f64(a);
        let got = to_f64(x.exp());
        let want = a.exp();
        let rel_err = ((got - want) / want).abs();
        assert!(rel_err <= EXP_LN_REL_TOL, "exp({a}) rel_err {rel_err} got {got} want {want}");
    }
    assert_eq!(FP::ZERO.exp(), FP::ONE);
}

#[test]
fn ln_accuracy() {
    // Same representation-floor consideration as `exp_accuracy`, but for
    // the *input*: below x ~ 0.01 (raw ~655), quantizing x into Q48.16
    // already introduces more relative error in x (and hence in ln(x))
    // than the accuracy budget allows.
    for a in sweep(0.01, 50000.0, 3000) {
        let x = from_f64(a);
        let got = to_f64(x.ln());
        let want = a.ln();
        let rel_err = if want.abs() > 1e-6 {
            ((got - want) / want).abs()
        } else {
            (got - want).abs()
        };
        assert!(rel_err <= EXP_LN_REL_TOL, "ln({a}) rel_err {rel_err} got {got} want {want}");
    }
    assert_eq!(FP::ONE.ln(), FP::ZERO);
}

#[test]
fn pow_int_matches_f64_powi() {
    // pow_int is exponentiation-by-squaring, so it groups multiplications
    // differently than naive repeated multiplication; with `FP::mul`'s
    // floor rounding at every step the two can differ by a couple of raw
    // units for larger n. Compare against f64 instead of a fixed-point
    // reference accumulation.
    let base = fp!(1.001);
    for n in 0..40 {
        let got = to_f64(base.pow_int(n));
        let want = 1.001_f64.powi(n);
        let err = (got - want).abs();
        // Each squaring step floors, so error can accumulate by a couple
        // of raw units per doubling; scale the budget with n accordingly.
        let budget = (2.0 + n as f64 * 0.5) / 65536.0;
        assert!(err <= budget, "n={n} got={got} want={want} err={err} budget={budget}");
    }
    // negative exponent is the reciprocal
    assert_eq!(base.pow_int(-3), FP::ONE / base.pow_int(3));
}
