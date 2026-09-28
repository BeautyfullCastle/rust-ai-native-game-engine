use orr_fp::{fp, FP};

#[test]
fn min_max_constants() {
    assert_eq!(FP::MAX.raw(), i64::MAX);
    assert_eq!(FP::MIN.raw(), i64::MIN);
    assert!(FP::MAX > FP::ZERO);
    assert!(FP::MIN < FP::ZERO);
}

#[test]
fn saturating_add_sub_at_bounds() {
    assert_eq!(FP::MAX.saturating_add(FP::ONE), FP::MAX);
    assert_eq!(FP::MIN.saturating_sub(FP::ONE), FP::MIN);
    assert_eq!(FP::MIN.saturating_add(FP::MIN), FP::MIN);
    assert_eq!(FP::MAX.saturating_add(FP::MAX), FP::MAX);
}

#[test]
fn checked_add_sub_overflow() {
    assert_eq!(FP::MAX.checked_add(FP::ONE), None);
    assert_eq!(FP::MIN.checked_sub(FP::ONE), None);
    assert_eq!(FP::ZERO.checked_add(FP::ONE), Some(FP::ONE));
}

#[test]
#[should_panic(expected = "division by zero")]
fn div_by_zero_panics() {
    let _ = FP::ONE / FP::ZERO;
}

#[test]
fn checked_div_by_zero_is_none() {
    assert_eq!(FP::ONE.checked_div(FP::ZERO), None);
}

#[test]
fn saturating_div_by_zero_saturates() {
    assert_eq!(FP::ONE.saturating_div(FP::ZERO), FP::MAX);
    assert_eq!((-FP::ONE).saturating_div(FP::ZERO), FP::MIN);
}

#[test]
fn negative_floor_ceil_round() {
    assert_eq!(fp!(-1.5).floor(), fp!(-2));
    assert_eq!(fp!(-1.5).ceil(), fp!(-1));
    assert_eq!(fp!(-1.5).round(), fp!(-2)); // ties away from zero
    assert_eq!(fp!(1.5).round(), fp!(2));
    assert_eq!(fp!(-1.4).round(), fp!(-1));
    assert_eq!(fp!(-1.6).round(), fp!(-2));
    assert_eq!(fp!(-0.5).floor(), fp!(-1));
    assert_eq!(fp!(-0.5).ceil(), FP::ZERO);
    assert_eq!(fp!(2).floor(), fp!(2));
    assert_eq!(fp!(2).ceil(), fp!(2));
}

#[test]
fn fract_matches_floor_identity() {
    for v in [fp!(1.75), fp!(-1.75), fp!(0), fp!(-0.1), fp!(5)] {
        assert_eq!(v.floor() + v.fract(), v, "v={v:?}");
        assert!(v.fract() >= FP::ZERO && v.fract() < FP::ONE, "v={v:?} fract={:?}", v.fract());
    }
}

#[test]
fn atan2_quadrants_and_axes() {
    assert_eq!(FP::ZERO.atan2(FP::ONE), FP::ZERO);
    assert_eq!(FP::ONE.atan2(FP::ZERO), FP::HALF_PI);
    assert_eq!((-FP::ONE).atan2(FP::ZERO), -FP::HALF_PI);
    assert_eq!(FP::ZERO.atan2(FP::ZERO), FP::ZERO);

    // Quadrant I: y>0, x>0 -> (0, pi/2)
    let q1 = fp!(1).atan2(fp!(1));
    assert!(q1 > FP::ZERO && q1 < FP::HALF_PI);

    // Quadrant II: y>0, x<0 -> (pi/2, pi)
    let q2 = fp!(1).atan2(fp!(-1));
    assert!(q2 > FP::HALF_PI && q2 < FP::PI);

    // Quadrant III: y<0, x<0 -> (-pi, -pi/2)
    let q3 = fp!(-1).atan2(fp!(-1));
    assert!(q3 < -FP::HALF_PI && q3 > -FP::PI);

    // Quadrant IV: y<0, x>0 -> (-pi/2, 0)
    let q4 = fp!(-1).atan2(fp!(1));
    assert!(q4 < FP::ZERO && q4 > -FP::HALF_PI);
}

#[test]
fn mul_rounding_floors() {
    // 1/3 * 3 in this crate's flooring semantics should stay <= 1.0.
    let third = FP::from_ratio(1, 3);
    let back = third * fp!(3);
    assert!(back <= FP::ONE);
}

#[test]
fn from_ratio_basic() {
    assert_eq!(FP::from_ratio(1, 2), fp!(0.5));
    assert_eq!(FP::from_ratio(-1, 2), fp!(-0.5));
    assert_eq!(FP::from_ratio(1, 1), FP::ONE);
    assert_eq!(FP::from_ratio(0, 5), FP::ZERO);
}

#[test]
#[should_panic(expected = "negative input")]
fn sqrt_negative_panics_in_debug() {
    let _ = fp!(-1).sqrt();
}

#[test]
#[should_panic(expected = "domain error")]
fn ln_nonpositive_panics() {
    let _ = FP::ZERO.ln();
}

#[test]
fn wrap_angle_large_values() {
    // sin/cos of a very large angle should still match the wrapped value.
    let big = fp!(1000);
    let (s, c) = big.sin_cos();
    let wrapped = big.wrap_angle();
    let (ws, wc) = wrapped.sin_cos();
    assert_eq!(s, ws);
    assert_eq!(c, wc);
}
