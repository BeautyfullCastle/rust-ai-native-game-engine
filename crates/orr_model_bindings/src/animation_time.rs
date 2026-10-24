//! GPU-free, presentation-only time from an absolute simulation tick and rate.
//!
//! The host already accounts for playback speed by advancing ticks. This clock
//! uses fixed 1x with no wall time, accumulated delta, or history/transition origin.
#![allow(clippy::float_arithmetic)]

use crate::model_bindings::PlaybackMode;

/// Samples one clip at an absolute simulation tick, at fixed 1x presentation
/// speed. Once clamps to its final time; Loop wraps at the clip duration.
pub fn sample_clip_time(
    tick: u64,
    tick_rate: u32,
    duration_seconds: f32,
    playback: PlaybackMode,
) -> Result<f32, String> {
    if tick_rate == 0 {
        return Err("animation cannot sample a zero tick rate".into());
    }
    if !duration_seconds.is_finite() || duration_seconds < 0.0 {
        return Err("animation clip duration must be finite and nonnegative".into());
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
        return Err("animation sample time is not finite".into());
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn exact_fractional_period_and_extreme_f32_durations_are_bounded() {
        // 3/8 second at 60Hz is 22.5 ticks: retaining integer modular
        // arithmetic distinguishes adjacent ticks even at the u64 boundary.
        for tick in [u64::MAX - 2, u64::MAX - 1, u64::MAX] {
            let expected = ((u128::from(tick) * 2) % 45) as f32 / 120.0;
            assert_eq!(
                sample_clip_time(tick, 60, 0.375, PlaybackMode::Loop).unwrap(),
                expected
            );
        }
        for duration in [
            f32::from_bits(1),
            f32::MIN_POSITIVE,
            0.125,
            16777216.0,
            f32::MAX,
        ] {
            for rate in [1, 60, u32::MAX] {
                for tick in [0, 1, u64::MAX] {
                    let time = sample_clip_time(tick, rate, duration, PlaybackMode::Loop).unwrap();
                    assert!(time.is_finite() && (0.0..duration).contains(&time));
                    let time = sample_clip_time(tick, rate, duration, PlaybackMode::Once).unwrap();
                    assert!(time.is_finite() && (0.0..=duration).contains(&time));
                }
            }
        }
        for duration in [-1.0, f32::INFINITY, f32::NEG_INFINITY, f32::NAN] {
            assert!(sample_clip_time(1, 60, duration, PlaybackMode::Loop).is_err());
        }
    }
}
