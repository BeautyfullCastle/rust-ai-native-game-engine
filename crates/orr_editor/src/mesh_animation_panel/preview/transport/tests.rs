use super::*;
#[test]
fn pause_resume_and_restart_keep_clock_out_of_host() {
    let mut t = Transport::default();
    t.play(10.0, 1, 60, false);
    t.advance(10.25, 2, true, 60, 60, false);
    assert_eq!(t.tick, 15);
    t.pause();
    t.advance(99.0, 3, true, 60, 60, false);
    assert_eq!(t.tick, 15);
    t.play(99.0, 3, 60, false);
    t.advance(99.25, 4, true, 60, 60, false);
    assert_eq!(t.tick, 30);
    t.restart(99.25, 4, 60, false);
    assert_eq!(t.tick, 0);
    assert!(t.running);
    t.pause();
    t.restart(100.0, 5, 60, false);
    assert_eq!(t.tick, 0);
    assert!(!t.running);
}
#[test]
fn speed_changes_preserve_fraction_without_jump() {
    let mut t = Transport::default();
    t.play(0.0, 0, 600, true);
    t.advance(0.125, 1, true, 60, 600, true);
    assert_eq!(t.tick, 7);
    t.speed = Speed::Double;
    assert_eq!(t.tick, 7);
    t.advance(0.25, 2, true, 60, 600, true);
    assert_eq!(t.tick, 22);
    t.speed = Speed::Half;
    t.advance(0.5, 3, true, 60, 600, true);
    assert_eq!(t.tick, 30);
    t.speed = Speed::Quarter;
    t.advance(1.0, 4, true, 60, 600, true);
    assert_eq!(t.tick, 37);
}
#[test]
fn once_stops_at_ceil_endpoint_loop_does_not_round_modulo() {
    let mut t = Transport::default();
    t.play(0.0, 0, 13, false);
    t.advance(0.25, 1, true, 60, 13, false);
    assert_eq!(t.tick, 13);
    assert!(!t.running);
    t.play(1.0, 2, 13, false);
    assert_eq!(t.tick, 0);
    t.advance(1.25, 3, true, 60, 13, true);
    assert_eq!(t.tick, 15);
    assert!(t.running);
}
#[test]
fn invalid_backward_focus_missing_frames_and_suspension_pause_without_catchup() {
    for (now, frame, focus) in [
        (f64::NAN, 2, true),
        (f64::INFINITY, 2, true),
        (-1.0, 2, true),
        (2.0, 2, true),
        (0.25, 3, true),
        (0.25, 0, true),
        (0.25, 2, false),
    ] {
        let mut t = Transport::default();
        t.seek(7);
        t.play(0.0, 1, 600, true);
        t.advance(now, frame, focus, 60, 600, true);
        assert_eq!(t.tick, 7);
        assert!(!t.running);
    }
}
#[test]
fn zero_duration_overflow_and_duplicate_egui_pass_are_safe() {
    let mut t = Transport::default();
    t.seek(9);
    t.play(0.0, 0, 0, true);
    assert_eq!(t.tick, 0);
    assert!(!t.running);
    t.seek(u64::MAX - 1);
    t.play(0.0, 0, 1, true);
    t.advance(0.25, 1, true, 60, 1, true);
    assert_eq!(t.tick, u64::MAX - 1);
    assert!(!t.running);
    t.seek(0);
    t.play(0.0, 1, 600, true);
    t.advance(0.25, 1, true, 60, 600, true);
    assert_eq!(t.tick, 0);
    t.advance(0.25, 2, true, 60, 600, true);
    assert_eq!(t.tick, 15);
}
