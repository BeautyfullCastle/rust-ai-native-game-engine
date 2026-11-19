//! The "ding" of a picked star, made in code, for steps 3 to 5.
//!
//! It is made from numbers when the program starts, so these steps need no sound file. Step 2 plays
//! the same ding from a WAV file instead (`assets/pickup.wav`, made by `assets/make_pickup_wav.py`)
//! and only borrows the name `PICKUP` from here. A real game loads recorded sound data from a file
//! (`App::sound_file`) or makes the `Clip` from samples (`Clip::from_stereo`).

use orr_app::Clip;

/// The name of the sound for `App::sound` and `ctx.audio().play_named`.
pub const PICKUP: &str = "pickup";

const RATE: u32 = 44_100;
/// Length of the sound in seconds, and the moment when the pitch steps up.
const LENGTH: f32 = 0.18;
const STEP_AT: f32 = 0.07;

/// Two sine tones one after the other (A5, then E6), fading out. About 0.18 seconds.
#[allow(clippy::float_arithmetic)]
pub fn pickup() -> Clip {
    let count = (RATE as f32 * LENGTH) as usize;
    let frames = (0..count)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let (hz, since) = if t < STEP_AT { (880.0, 0.0) } else { (1318.5, STEP_AT) };
            // A very short rise at the start of each tone (no click), and a fade that ends at zero.
            let rise = ((t - since) / 0.004).min(1.0);
            let fade = (1.0 - t / LENGTH).max(0.0);
            let v = 0.25 * rise * fade * fade * ((t - since) * std::f32::consts::TAU * hz).sin();
            [v, v]
        })
        .collect();
    Clip::from_stereo(RATE, frames).expect("the samples are finite and within -1..1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_app::{AudioConfig, OfflineAudio};

    #[test]
    #[allow(clippy::float_arithmetic)]
    fn the_pickup_sound_is_audible_and_ends_by_itself() {
        let mut audio = OfflineAudio::new_offline(AudioConfig::default(), RATE).unwrap();
        assert!(audio.play(&pickup()).is_some());
        let mut samples = vec![0.0f32; 2 * RATE as usize]; // one second of stereo
        for block in samples.chunks_mut(256) {
            audio.render(block).unwrap();
        }
        let loud = |range: std::ops::Range<usize>| samples[range].iter().fold(0.0f32, |peak, v| peak.max(v.abs()));
        assert!(loud(0..2000) > 0.05, "the ding is heard at the start");
        assert!(loud(2 * 11_000..2 * RATE as usize) < 1e-6, "and it is over after 0.25 s");
        assert_eq!(audio.voice_count(), 0);
    }
}
