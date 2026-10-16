//! Absolute, presentation-only animation time for the Yard3D host timeline.
//!
//! The host has already accounted for its playback speed by advancing ticks.
//! Therefore a clip is sampled from the snapshot's absolute tick and tick rate
//! at fixed 1x. No egui clock, incremental delta, or rolling-history origin is
//! involved. `None` from [`snapshot_clip_time`] means Edit/Stop and requests a
//! rest pose; tick zero in Play is the clip's time-zero pose.

use crate::model_bindings::PlaybackMode;
use orr_bridge::Snapshot;

/// Samples one clip at an absolute simulation tick, at fixed 1x presentation
/// speed. Once clamps to its final time; Loop wraps at the clip duration.
pub fn sample_clip_time(
    tick: u64,
    tick_rate: u32,
    duration_seconds: f32,
    playback: PlaybackMode,
) -> Result<f32, String> {
    if tick_rate == 0 {
        return Err("Yard animation cannot sample a zero tick rate".into());
    }
    if !duration_seconds.is_finite() || duration_seconds < 0.0 {
        return Err("Yard animation clip duration must be finite and nonnegative".into());
    }
    if duration_seconds == 0.0 {
        // Clips whose keys all occur at time zero are valid and sample that
        // rest-derived pose at time zero for either playback policy.
        return Ok(0.0);
    }

    let duration = f64::from(duration_seconds);
    let (mantissa, exponent) = f32_rational(duration_seconds);
    let sampled = match playback {
        PlaybackMode::Once if once_finished(tick, tick_rate, mantissa, exponent) => duration,
        PlaybackMode::Once => tick as f64 / f64::from(tick_rate),
        PlaybackMode::Loop => loop_remainder(tick, tick_rate, mantissa, exponent),
    } as f32;
    let sampled = if playback == PlaybackMode::Loop && sampled >= duration_seconds {
        // Narrowing to f32 can round a value just below the endpoint up to it;
        // Loop must keep the model's wrap-at-duration behavior.
        0.0
    } else {
        sampled
    };
    if !sampled.is_finite() {
        return Err("Yard animation sample time is not finite".into());
    }
    Ok(sampled)
}

/// Exact `f32` significand and power-of-two scale: `value = significand *
/// 2^exponent`. This makes the rational tick/clip modular arithmetic exact
/// until its final conversion to the model's presentation `f32` time.
fn f32_rational(value: f32) -> (u32, i32) {
    let bits = value.to_bits();
    let exponent_bits = ((bits >> 23) & 0xff) as i32;
    let fraction = bits & 0x7f_ffff;
    if exponent_bits == 0 {
        (fraction, -149)
    } else {
        ((1 << 23) | fraction, exponent_bits - 127 - 23)
    }
}

fn once_finished(tick: u64, tick_rate: u32, significand: u32, exponent: i32) -> bool {
    let rate = u128::from(tick_rate);
    let duration_ticks = if exponent >= 0 {
        u128::from(significand)
            .checked_shl(exponent as u32)
            .and_then(|seconds| seconds.checked_mul(rate))
    } else {
        let numerator = u128::from(significand) * rate;
        let shift = exponent.unsigned_abs();
        let ceiling = if shift >= 128 {
            1
        } else {
            let denominator = 1_u128 << shift;
            numerator.saturating_add(denominator - 1) / denominator
        };
        return u128::from(tick) >= ceiling.max(1);
    };
    duration_ticks.is_some_and(|duration_ticks| u128::from(tick) >= duration_ticks)
}

fn loop_remainder(tick: u64, tick_rate: u32, significand: u32, exponent: i32) -> f64 {
    let rate = u128::from(tick_rate);
    if exponent >= 0 {
        let Some(duration_ticks) = u128::from(significand)
            .checked_shl(exponent as u32)
            .and_then(|seconds| seconds.checked_mul(rate))
        else {
            // The loop period is larger than any representable absolute tick.
            return tick as f64 / f64::from(tick_rate);
        };
        return (u128::from(tick) % duration_ticks) as f64 / f64::from(tick_rate);
    }

    // With duration = significand / 2^shift, the tick period's integer
    // modulus is `tick_rate * significand`. Reduce `2^shift` modulo that
    // modulus before multiplying, keeping all intermediates exact and bounded.
    let shift = exponent.unsigned_abs();
    let modulus = rate * u128::from(significand);
    let mut power = 1_u128;
    let mut base = 2_u128 % modulus;
    let mut remaining = shift;
    while remaining != 0 {
        if remaining & 1 == 1 {
            power = (power * base) % modulus;
        }
        remaining >>= 1;
        if remaining != 0 {
            base = (base * base) % modulus;
        }
    }
    let numerator = (u128::from(tick) % modulus) * power % modulus;
    (numerator as f64 / f64::from(tick_rate)) * 2.0_f64.powi(-(shift as i32))
}

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
    fn once_clamps_at_both_clip_endpoints() {
        assert_eq!(
            sample_clip_time(0, 60, 2.0, PlaybackMode::Once).unwrap(),
            0.0
        );
        assert_eq!(
            sample_clip_time(60, 60, 2.0, PlaybackMode::Once).unwrap(),
            1.0
        );
        assert_eq!(
            sample_clip_time(120, 60, 2.0, PlaybackMode::Once).unwrap(),
            2.0
        );
        assert_eq!(
            sample_clip_time(180, 60, 2.0, PlaybackMode::Once).unwrap(),
            2.0
        );
    }

    #[test]
    fn loop_wraps_at_exact_duration() {
        assert_eq!(
            sample_clip_time(0, 60, 2.0, PlaybackMode::Loop).unwrap(),
            0.0
        );
        assert_eq!(
            sample_clip_time(60, 60, 2.0, PlaybackMode::Loop).unwrap(),
            1.0
        );
        assert_eq!(
            sample_clip_time(120, 60, 2.0, PlaybackMode::Loop).unwrap(),
            0.0
        );
        assert_eq!(
            sample_clip_time(150, 60, 2.0, PlaybackMode::Loop).unwrap(),
            0.5
        );
    }

    #[test]
    fn absolute_tick_sampling_is_stable_across_pause_seek_and_restart() {
        let at_tick = sample_clip_time(37, 60, 4.0, PlaybackMode::Loop).unwrap();
        // Repeated sampling of the same paused head is byte-identical.
        assert_eq!(
            at_tick.to_bits(),
            sample_clip_time(37, 60, 4.0, PlaybackMode::Loop)
                .unwrap()
                .to_bits()
        );
        // Seeking backwards and restarting are direct absolute samples, not a
        // subtraction from the previously observed tick.
        assert_eq!(
            sample_clip_time(12, 60, 4.0, PlaybackMode::Loop).unwrap(),
            0.2
        );
        assert_eq!(
            sample_clip_time(0, 60, 4.0, PlaybackMode::Loop).unwrap(),
            0.0
        );
    }

    #[test]
    fn rejects_zero_rate_and_invalid_clip_bounds() {
        assert!(sample_clip_time(1, 0, 1.0, PlaybackMode::Once).is_err());
        assert_eq!(
            sample_clip_time(1, 60, 0.0, PlaybackMode::Once).unwrap(),
            0.0
        );
        assert!(sample_clip_time(1, 60, f32::NAN, PlaybackMode::Loop).is_err());
    }

    #[test]
    fn large_ticks_stay_finite_and_within_loop_bounds() {
        let sample = sample_clip_time(u64::MAX, 60, 2.0, PlaybackMode::Loop).unwrap();
        assert!(sample.is_finite());
        assert!((0.0..2.0).contains(&sample));
        // u64::MAX % (2 seconds * 60 ticks/second) == 15 ticks.
        assert!((sample - 0.25).abs() < 1.0e-6);
        assert_eq!(
            sample.to_bits(),
            sample_clip_time(u64::MAX, 60, 2.0, PlaybackMode::Loop)
                .unwrap()
                .to_bits()
        );
        assert_eq!(
            sample_clip_time(u64::MAX, 60, 3.25, PlaybackMode::Once).unwrap(),
            3.25
        );
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
