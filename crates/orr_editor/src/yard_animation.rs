//! Absolute, presentation-only animation time for the Yard3D host timeline.
//!
//! The host has already accounted for its playback speed by advancing ticks.
//! Therefore a clip is sampled from the snapshot's absolute tick and tick rate
//! at fixed 1x. No egui clock, incremental delta, or rolling-history origin is
//! involved. `None` from [`snapshot_clip_time`] means Edit/Stop and requests a
//! rest pose; tick zero in Play is the clip's time-zero pose.

use crate::model_bindings::PlaybackMode;
use orr_bridge::Snapshot;

pub use orr_model_bindings::animation_time::sample_clip_time;

/// Samples from the same immutable host snapshot that supplied Yard body
/// transforms. No timeline means the model should use its rest pose.
pub fn snapshot_clip_time(
    snapshot: &Snapshot,
    duration_seconds: f32,
    playback: PlaybackMode,
) -> Result<Option<f32>, String> {
    if snapshot.tick_rate() == 0 {
        return Err("Yard animation cannot sample a zero tick rate".into());
    }
    let frame = snapshot.predicted();
    if snapshot.tick() != frame.tick() {
        return Err("Yard animation snapshot tick does not match its predicted frame".into());
    }
    let Some(timeline) = snapshot.timeline() else {
        return Ok(None);
    };
    if timeline.tick != snapshot.tick() {
        return Err("Yard animation snapshot and timeline ticks do not match".into());
    }
    if timeline.first_tick > timeline.last_tick
        || snapshot.tick() < timeline.first_tick
        || snapshot.tick() > timeline.last_tick
    {
        return Err("Yard animation snapshot tick is outside timeline bounds".into());
    }
    if timeline.checksum != frame.checksum() {
        return Err("Yard animation snapshot and timeline checksums do not match".into());
    }
    sample_clip_time(
        snapshot.tick(),
        snapshot.tick_rate(),
        duration_seconds,
        playback,
    )
    .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_bridge::{BridgeStats, PlayMode, SnapshotParts, Speed, Timeline};
    use orr_ecs::{ComponentRegistryBuilder, Frame};
    use std::sync::Arc;

    fn snapshot(
        tick: u64,
        tick_rate: u32,
        with_timeline: bool,
        timeline_tick: Option<u64>,
        first_tick: u64,
        last_tick: u64,
        checksum_matches: bool,
    ) -> Snapshot {
        snapshot_with_frame_tick(
            tick,
            tick,
            tick_rate,
            with_timeline,
            timeline_tick,
            first_tick,
            last_tick,
            checksum_matches,
        )
    }

    #[allow(clippy::too_many_arguments)] // Explicit independent adversarial snapshot fields.
    fn snapshot_with_frame_tick(
        tick: u64,
        frame_tick: u64,
        tick_rate: u32,
        with_timeline: bool,
        timeline_tick: Option<u64>,
        first_tick: u64,
        last_tick: u64,
        checksum_matches: bool,
    ) -> Snapshot {
        let mut frame = Frame::new(ComponentRegistryBuilder::new().build());
        frame.set_tick(frame_tick);
        let checksum = frame.checksum();
        let timeline = with_timeline.then(|| Timeline {
            mode: PlayMode::Record,
            tick: timeline_tick.unwrap_or(tick),
            verified_tick: tick,
            first_tick,
            last_tick,
            playing: false,
            speed: Speed(250),
            checksum: if checksum_matches {
                checksum
            } else {
                checksum.wrapping_add(1)
            },
            keyframes: Arc::from(Vec::<u64>::new()),
            recent_checksums: Arc::from(Vec::<(u64, u64)>::new()),
            pending_edits: 0,
            branches: 0,
            epoch: 4,
        });
        Snapshot::from_parts(SnapshotParts {
            seq: 1,
            tick,
            verified_tick: tick,
            tick_rate,
            predicted: Arc::new(frame),
            predicted_prev: None,
            verified: None,
            stats: BridgeStats::default(),
            last_rollback: None,
            timeline,
        })
    }

    #[test]
    fn snapshot_requires_a_coherent_bounded_host_timeline() {
        let good = snapshot(60, 60, true, None, 0, 120, true);
        assert_eq!(
            snapshot_clip_time(&good, 2.0, PlaybackMode::Once).unwrap(),
            Some(1.0)
        );

        let stale_tick = snapshot(60, 60, true, Some(59), 0, 120, true);
        assert!(snapshot_clip_time(&stale_tick, 2.0, PlaybackMode::Once).is_err());
        let stale_checksum = snapshot(60, 60, true, None, 0, 120, false);
        assert!(snapshot_clip_time(&stale_checksum, 2.0, PlaybackMode::Once).is_err());
        let outside_range = snapshot(60, 60, true, None, 61, 120, true);
        assert!(snapshot_clip_time(&outside_range, 2.0, PlaybackMode::Once).is_err());
        let invalid_range = snapshot(60, 60, true, None, 121, 120, true);
        assert!(snapshot_clip_time(&invalid_range, 2.0, PlaybackMode::Once).is_err());
        let edit_with_stale_frame = snapshot_with_frame_tick(0, 1, 60, false, None, 0, 0, true);
        assert!(snapshot_clip_time(&edit_with_stale_frame, 2.0, PlaybackMode::Once).is_err());
    }

    #[test]
    fn edit_and_zero_tickrate_are_explicit() {
        let edit = snapshot(0, 60, false, None, 0, 0, true);
        assert_eq!(
            snapshot_clip_time(&edit, 2.0, PlaybackMode::Loop).unwrap(),
            None
        );
        let zero_rate_edit = snapshot(0, 0, false, None, 0, 0, true);
        assert!(snapshot_clip_time(&zero_rate_edit, 2.0, PlaybackMode::Loop).is_err());
        let zero_rate_play = snapshot(0, 0, true, None, 0, 10, true);
        assert!(snapshot_clip_time(&zero_rate_play, 2.0, PlaybackMode::Loop).is_err());
    }

    #[test]
    fn play_zero_uses_time_zero_and_rolling_first_tick_does_not_shift_phase() {
        let tick_zero = snapshot(0, 60, true, None, 0, 90, true);
        assert_eq!(
            snapshot_clip_time(&tick_zero, 3.0, PlaybackMode::Loop).unwrap(),
            Some(0.0)
        );

        let rolling = snapshot(121, 60, true, None, 100, 180, true);
        let sampled = snapshot_clip_time(&rolling, 3.0, PlaybackMode::Loop)
            .unwrap()
            .unwrap();
        assert_eq!(
            sampled.to_bits(),
            sample_clip_time(121, 60, 3.0, PlaybackMode::Loop)
                .unwrap()
                .to_bits()
        );
        assert_ne!(
            sampled,
            sample_clip_time(21, 60, 3.0, PlaybackMode::Loop).unwrap()
        );
    }

    #[test]
    fn host_speed_is_not_multiplied_again() {
        let slow_host = snapshot(60, 60, true, None, 0, 120, true);
        // Snapshot carries a deliberately non-normal speed, but the pure
        // presentation sample remains exactly tick / tick_rate (1 second).
        assert_eq!(
            snapshot_clip_time(&slow_host, 5.0, PlaybackMode::Once).unwrap(),
            Some(1.0)
        );
    }
}
