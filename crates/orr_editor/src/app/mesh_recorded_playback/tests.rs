use super::{valid_clock, PlaybackSpeed, State, MAX_SEEK_TICKS_PER_FRAME};

fn seek_target(
    anchor: u64,
    last: u64,
    rate: u32,
    start: f64,
    now: f64,
    requested: u64,
) -> Result<u64, &'static str> {
    super::seek_target(
        anchor,
        last,
        rate,
        start,
        now,
        requested,
        PlaybackSpeed::Normal,
    )
}

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

#[test]
fn selected_speeds_scale_recorded_ticks_and_keep_the_frame_bound() {
    for (speed, ticks) in [
        (PlaybackSpeed::Quarter, 15),
        (PlaybackSpeed::Half, 30),
        (PlaybackSpeed::Normal, 60),
        (PlaybackSpeed::Double, 120),
        (PlaybackSpeed::Quadruple, 120),
    ] {
        assert_eq!(
            super::seek_target(0, 1000, 60, 0.0, 1.0, 0, speed),
            Ok(ticks)
        );
        assert_eq!(super::seek_target(0, 5, 60, 0.0, 1.0, 0, speed), Ok(5));
    }
    assert!(super::seek_target(0, 1000, 60, 0.0, f64::MAX, 0, PlaybackSpeed::Quadruple).is_err());
}

#[cfg(feature = "mesh-mission")]
mod local_editor_tests {
    use super::*;
    use crate::{
        editor::Editor,
        mesh_workspace::mission::{self, Settings},
    };

    fn recorded_editor() -> (tempfile::TempDir, Editor) {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = mission::prepare(
            &directory.path().join("seek.scene.yaml"),
            &Settings::default(),
        )
        .unwrap();
        assert!(editor.start_play());
        editor.step(60);
        editor.sync();
        assert!(!editor.sim().playing);
        (directory, editor)
    }

    #[test]
    fn scrub_backwards_and_forwards_preserves_recorded_future_and_checksum() {
        let (_directory, mut editor) = recorded_editor();
        let end_checksum = editor.checksum();
        let mut state = State::default();
        assert!(state.seek_recorded(&mut editor, 15, 0.0));
        editor.sync();
        let earlier_checksum = editor.checksum();
        assert!(state.active());
        assert!(!state.running());
        assert_eq!(editor.sim().last_tick, 60);
        state.pump(&mut editor, 10.0);
        assert_eq!(editor.sim().head_tick, 15);
        assert!(state.seek_recorded(&mut editor, 60, 10.0));
        editor.sync();
        assert_eq!(editor.checksum(), end_checksum);
        assert!(state.seek_recorded(&mut editor, 15, 10.0));
        editor.sync();
        assert_eq!(editor.checksum(), earlier_checksum);
        assert!(!editor.sim().playing);
        assert_eq!(editor.sim().last_tick, 60);
    }

    #[test]
    fn speed_change_reanchors_without_jumping_or_changing_host_speed() {
        let (_directory, mut editor) = recorded_editor();
        let host_speed = editor.sim().speed_permille;
        let mut state = State::default();
        assert!(state.seek_recorded(&mut editor, 0, 0.0));
        editor.sync();
        assert!(state.resume(&mut editor, 0.0));
        state.pump(&mut editor, 0.25);
        editor.sync();
        assert_eq!(editor.sim().head_tick, 15);
        assert!(state.set_speed(&editor, PlaybackSpeed::Double, 0.25));
        state.pump(&mut editor, 0.25);
        assert_eq!(editor.sim().head_tick, 15);
        state.pump(&mut editor, 0.5);
        editor.sync();
        assert_eq!(editor.sim().head_tick, 45);
        assert_eq!(editor.sim().speed_permille, host_speed);
        assert!(!editor.sim().playing);
        assert!(state.seek_recorded(&mut editor, 10, 0.5));
        assert!(state.set_speed(&editor, PlaybackSpeed::Half, 0.5));
        editor.sync();
        state.pump(&mut editor, 5.0);
        assert_eq!(editor.sim().head_tick, 10);
        state.pause(&mut editor);
        assert!(!state.active());
        assert_eq!(state.speed(), PlaybackSpeed::Half);
    }

    #[test]
    fn rejected_seek_does_not_clamp_or_start_live_play() {
        let (_directory, mut editor) = recorded_editor();
        let mut state = State::default();
        let epoch = editor.sim().epoch;
        assert!(!state.seek_recorded(&mut editor, 61, 0.0));
        assert_eq!(editor.sim().head_tick, 60);
        assert_eq!(editor.sim().epoch, epoch);
        assert!(!state.seek_recorded(&mut editor, 10, f64::NAN));
        assert_eq!(editor.sim().epoch, epoch);
        editor.play();
        assert!(!state.seek_recorded(&mut editor, 10, 1.0));
        assert!(editor.sim().playing);
    }

    #[test]
    fn paused_scrub_rejects_external_timeline_change_and_nonfinite_speed_clock() {
        let (_directory, mut editor) = recorded_editor();
        let mut state = State::default();
        assert!(state.seek_recorded(&mut editor, 10, 1.0));
        editor.seek(20);
        assert!(!state.seek_recorded(&mut editor, 5, 2.0));
        assert_eq!(editor.sim().head_tick, 20);
        assert!(!state.active());
        assert!(state.seek_recorded(&mut editor, 10, 2.0));
        assert!(!state.set_speed(&editor, PlaybackSpeed::Double, 1.0));
        assert_eq!(state.speed(), PlaybackSpeed::Normal);
        assert_eq!(editor.sim().head_tick, 10);
        assert!(state.seek_recorded(&mut editor, 60, 3.0));
        assert!(!state.resume(&mut editor, 3.0));
        assert!(!state.active());
        assert!(!editor.sim().playing);
    }

    #[test]
    fn branch_or_new_session_invalidates_paused_scrub_context() {
        let (_directory, mut editor) = recorded_editor();
        let mut state = State::default();
        assert!(state.seek_recorded(&mut editor, 20, 0.0));
        editor.branch();
        assert!(!state.set_speed(&editor, PlaybackSpeed::Double, 1.0));
        assert_eq!(editor.sim().last_tick, 20);
        assert!(state.seek_recorded(&mut editor, 5, 1.0));
        editor.stop();
        assert!(editor.start_play());
        state.pump(&mut editor, 2.0);
        assert!(!state.active());
        assert!(state.error().is_some());
    }

    #[test]
    fn reloading_same_scene_invalidates_recorded_identity() {
        let (_directory, mut editor) = recorded_editor();
        let path = editor.path().unwrap();
        let mut state = State::default();
        assert!(state.seek_recorded(&mut editor, 10, 0.0));
        editor.stop();
        assert!(editor.open_path(&path));
        assert!(editor.start_play());
        editor.step(60);
        editor.seek(10);
        assert!(!state.seek_recorded(&mut editor, 5, 1.0));
        assert_eq!(editor.sim().head_tick, 10);
        assert_eq!(
            state.error(),
            Some("Recorded playback stopped because the scene changed")
        );
    }
}
