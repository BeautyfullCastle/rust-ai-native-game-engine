use super::{seek_target, valid_clock, State, MAX_SEEK_TICKS_PER_FRAME};

#[test]
fn seek_target_floors_elapsed_ticks() {
    assert_eq!(seek_target(10, 80, 8, 2.0, 2.0, 10), Ok(10));
    assert_eq!(seek_target(10, 80, 8, 2.0, 2.1875, 10), Ok(11));
    assert_eq!(seek_target(10, 80, 8, 2.0, 2.375, 11), Ok(13));
}

#[test]
fn seek_target_never_moves_backwards_and_caps_each_frame() {
    let target = seek_target(0, 10_000, 60, 0.0, 10.0, 7).unwrap();
    assert_eq!(target, 7 + MAX_SEEK_TICKS_PER_FRAME);
    assert_eq!(seek_target(0, 10_000, 60, 0.0, 0.0, target), Ok(target));
}

#[test]
fn seek_target_clamps_at_recorded_end() {
    assert_eq!(seek_target(20, 25, 60, 1.0, 5.0, 20), Ok(25));
    assert_eq!(seek_target(20, 25, 60, 1.0, 6.0, 25), Ok(25));
}

#[test]
fn seek_target_rejects_invalid_ranges_and_tick_rates() {
    assert!(seek_target(5, 4, 60, 0.0, 1.0, 5).is_err());
    assert!(seek_target(0, 4, 0, 0.0, 1.0, 0).is_err());
    assert!(seek_target(0, 4, 60, 0.0, 1.0, 5).is_err());
}

#[test]
fn seek_target_rejects_nonfinite_or_backwards_clock() {
    assert!(seek_target(0, 100, 60, 1.0, f64::NAN, 0).is_err());
    assert!(seek_target(0, 100, 60, 1.0, f64::INFINITY, 0).is_err());
    assert!(seek_target(0, 100, 60, 2.0, 1.0, 0).is_err());
}

#[test]
fn state_starts_inactive_and_clock_validation_is_monotonic() {
    let state = State::default();
    assert!(!state.active());
    assert_eq!(state.error(), None);
    assert!(valid_clock(1.0, 1.0));
    assert!(valid_clock(1.0, 2.0));
    assert!(!valid_clock(2.0, 1.0));
    assert!(!valid_clock(1.0, f64::NAN));
}
