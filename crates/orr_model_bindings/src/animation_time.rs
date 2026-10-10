//! GPU-free, presentation-only time from an absolute simulation tick and rate.
//!
//! The host accounts for simulation playback speed by advancing ticks. Optional
//! authored clip rates scale those absolute ticks, without a second clock.
#![allow(clippy::float_arithmetic)]

use crate::model_bindings::PlaybackMode;

/// Closed exact powers-of-two clip rates. This is presentation data only; it
/// never changes the simulation tick rate or introduces accumulated time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub enum PlaybackRate {
    #[serde(rename = "1/4x")]
    Quarter,
    #[serde(rename = "1/2x")]
    Half,
    #[default]
    #[serde(rename = "1x")]
    Normal,
    #[serde(rename = "2x")]
    Double,
    #[serde(rename = "4x")]
    Quadruple,
}

impl<'de> serde::Deserialize<'de> for PlaybackRate {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Rate;
        impl serde::de::Visitor<'_> for Rate {
            type Value = PlaybackRate;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("one of the strings 1/4x, 1/2x, 1x, 2x, 4x")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                PlaybackRate::ALL
                    .into_iter()
                    .find(|rate| rate.label() == value)
                    .ok_or_else(|| E::custom("unsupported animation playback rate"))
            }
        }
        deserializer.deserialize_str(Rate)
    }
}

impl PlaybackRate {
    pub const ALL: [Self; 5] = [
        Self::Quarter,
        Self::Half,
        Self::Normal,
        Self::Double,
        Self::Quadruple,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Quarter => "1/4x",
            Self::Half => "1/2x",
            Self::Normal => "1x",
            Self::Double => "2x",
            Self::Quadruple => "4x",
        }
    }

    const fn exponent(self) -> i32 {
        match self {
            Self::Quarter => -2,
            Self::Half => -1,
            Self::Normal => 0,
            Self::Double => 1,
            Self::Quadruple => 2,
        }
    }
}

/// Samples one clip at an absolute simulation tick, at fixed 1x presentation
/// speed. Once clamps to its final time; Loop wraps at the clip duration.
pub fn sample_clip_time(
    tick: u64,
    tick_rate: u32,
    duration_seconds: f32,
    playback: PlaybackMode,
) -> Result<f32, String> {
    sample_clip_time_at_rate(
        tick,
        tick_rate,
        duration_seconds,
        playback,
        PlaybackRate::Normal,
    )
}

/// Sample a closed rational rate directly from the absolute tick. Reduction is
/// exact before the final float conversion, including ticks above 2^53. The
/// integer modulus is at most 56 bits and its products at most 112 bits.
pub fn sample_clip_time_at_rate(
    tick: u64,
    tick_rate: u32,
    duration_seconds: f32,
    playback: PlaybackMode,
    speed: PlaybackRate,
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
    // Divide the period by the exact rate instead of multiplying u64 ticks.
    // Scaling the reduced remainder back cannot overflow f64 for f32 bounds.
    let exponent = exponent - speed.exponent();
    let scale = 2.0_f64.powi(speed.exponent());
    let sampled = match playback {
        PlaybackMode::Once if once_finished(tick, tick_rate, mantissa, exponent) => duration,
        PlaybackMode::Once => tick as f64 / f64::from(tick_rate) * scale,
        PlaybackMode::Loop => loop_remainder(tick, tick_rate, mantissa, exponent) * scale,
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
        1_u128
            .checked_shl(exponent as u32)
            .and_then(|scale| u128::from(significand).checked_mul(scale))
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
        let Some(duration_ticks) = 1_u128
            .checked_shl(exponent as u32)
            .and_then(|scale| u128::from(significand).checked_mul(scale))
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
    fn every_closed_rate_has_exact_rational_phase_at_large_ticks() {
        for (speed, numerator, denominator) in [
            (PlaybackRate::Quarter, 1_u128, 4_u128),
            (PlaybackRate::Half, 1, 2),
            (PlaybackRate::Normal, 1, 1),
            (PlaybackRate::Double, 2, 1),
            (PlaybackRate::Quadruple, 4, 1),
        ] {
            for tick in [
                0,
                1,
                22,
                30,
                45,
                60,
                90,
                u64::MAX - 2,
                u64::MAX - 1,
                u64::MAX,
            ] {
                // duration 3/8 at 60 Hz: phase = ((tick * n * 2) %
                // (45 * d)) / (120 * d), with no floating tick conversion.
                let expected = ((u128::from(tick) * numerator * 2) % (45 * denominator)) as f32
                    / (120 * denominator) as f32;
                let actual =
                    sample_clip_time_at_rate(tick, 60, 0.375, PlaybackMode::Loop, speed).unwrap();
                assert_eq!(actual, expected, "{speed:?} tick {tick}");
                assert_eq!(
                    actual.to_bits(),
                    sample_clip_time_at_rate(tick, 60, 0.375, PlaybackMode::Loop, speed)
                        .unwrap()
                        .to_bits()
                );
                let expected_once = ((tick as f64 * numerator as f64) / (60.0 * denominator as f64))
                    .min(0.375) as f32;
                assert_eq!(
                    sample_clip_time_at_rate(tick, 60, 0.375, PlaybackMode::Once, speed).unwrap(),
                    expected_once
                );
            }
        }
    }

    #[test]
    fn scaled_extreme_durations_rates_and_invalid_input_are_bounded() {
        for speed in PlaybackRate::ALL {
            for duration in [
                0.0,
                f32::from_bits(1),
                f32::MIN_POSITIVE,
                0.125,
                16777216.0,
                f32::MAX,
            ] {
                for tick_rate in [1, 60, u32::MAX] {
                    for tick in [0, 1, u64::MAX] {
                        for mode in [PlaybackMode::Loop, PlaybackMode::Once] {
                            let time =
                                sample_clip_time_at_rate(tick, tick_rate, duration, mode, speed)
                                    .unwrap();
                            assert!(time.is_finite() && time >= 0.0 && time <= duration);
                            if mode == PlaybackMode::Loop && duration != 0.0 {
                                assert!(time < duration);
                            }
                            if speed == PlaybackRate::Normal {
                                assert_eq!(
                                    time.to_bits(),
                                    sample_clip_time(tick, tick_rate, duration, mode)
                                        .unwrap()
                                        .to_bits()
                                );
                            }
                        }
                    }
                }
            }
            assert!(sample_clip_time_at_rate(1, 0, 0.0, PlaybackMode::Loop, speed).is_err());
            for duration in [-1.0, f32::INFINITY, f32::NEG_INFINITY, f32::NAN] {
                assert!(
                    sample_clip_time_at_rate(1, 60, duration, PlaybackMode::Loop, speed).is_err()
                );
            }
        }
        for invalid in [
            "null",
            "0",
            "1.0",
            "\"0x\"",
            "\"3x\"",
            "\"-1x\"",
            "\"NaN\"",
            "{}",
            "{\"1x\":null}",
            "[]",
        ] {
            assert!(
                serde_json::from_str::<PlaybackRate>(invalid).is_err(),
                "{invalid}"
            );
        }
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
