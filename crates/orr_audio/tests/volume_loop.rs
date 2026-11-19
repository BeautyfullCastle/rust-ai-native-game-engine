//! Volume and loop of a played clip (`Clip::volume`, `Clip::looped`, `EventAudio::set_volume`).
#![allow(clippy::float_arithmetic)]

use orr_audio::{AudioConfig, Clip, OfflineAudio};

const RATE: u32 = 48_000;

/// A constant 0.5 level for `frames` frames: its peak is its level, so a gain shows directly.
fn flat(frames: usize) -> Clip {
    Clip::from_stereo(RATE, vec![[0.5, 0.5]; frames]).unwrap()
}

fn mixer() -> OfflineAudio {
    OfflineAudio::new_offline(AudioConfig::default(), RATE).unwrap()
}

fn render(audio: &mut OfflineAudio, frames: usize) -> Vec<f32> {
    let mut samples = vec![0.0; frames * 2];
    for block in samples.chunks_mut(256) {
        audio.render(block).unwrap();
    }
    samples
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |peak, v| peak.max(v.abs()))
}

#[test]
fn a_clip_volume_scales_what_is_heard_and_a_full_volume_changes_nothing() {
    let mut loud = mixer();
    loud.play(&flat(4800)).unwrap();
    let loud = peak(&render(&mut loud, 2400));
    let mut half = mixer();
    half.play(&flat(4800).volume(0.5)).unwrap();
    let half = peak(&render(&mut half, 2400));
    let mut muted = mixer();
    muted.play(&flat(4800).volume(0.0)).unwrap();
    let muted = peak(&render(&mut muted, 2400));
    assert!((loud - 0.5).abs() < 0.01, "as recorded: {loud}");
    assert!((half - 0.25).abs() < 0.01, "half the amplitude: {half}");
    assert!(muted < 0.001, "volume 0 is silent: {muted}");
}

#[test]
fn a_volume_that_is_not_a_number_or_too_large_is_bounded() {
    let mut audio = mixer();
    audio.play(&flat(4800).volume(f32::NAN)).unwrap();
    assert!(peak(&render(&mut audio, 2400)) < 0.001);
    let mut audio = mixer();
    audio.play(&flat(4800).volume(7.0)).unwrap();
    let level = peak(&render(&mut audio, 2400));
    assert!((level - 0.5).abs() < 0.01, "clamped to full volume: {level}");
}

#[test]
fn a_looped_clip_plays_until_it_is_stopped_and_a_plain_one_ends() {
    let mut audio = mixer();
    let plain = audio.play(&flat(2400)).unwrap();
    let looped = audio.play(&flat(2400).looped()).unwrap();
    // 0.25 seconds: the plain clip (50 ms) is over, the loop goes on.
    let heard = render(&mut audio, 12_000);
    assert!(!audio.is_playing(plain));
    assert!(audio.is_playing(looped));
    assert!(peak(&heard[heard.len() - 2000..]) > 0.4, "the loop is still audible at the end");
    audio.stop(looped);
    let after = render(&mut audio, 4800);
    assert!(!audio.is_playing(looped), "stop ends a loop");
    assert!(peak(&after[after.len() - 2000..]) < 0.001);
    assert_eq!(audio.voice_count(), 0);
}

#[test]
fn set_volume_changes_a_playing_sound_and_ignores_a_finished_one() {
    let mut audio = mixer();
    let id = audio.play(&flat(2400).looped()).unwrap();
    let before = render(&mut audio, 4800);
    assert!((peak(&before[before.len() - 1000..]) - 0.5).abs() < 0.01);
    audio.set_volume(id, 0.2);
    let after = render(&mut audio, 4800);
    let level = peak(&after[after.len() - 1000..]);
    assert!((level - 0.1).abs() < 0.01, "0.5 x 0.2 after the 20 ms change: {level}");
    audio.stop(id);
    render(&mut audio, 4800);
    audio.set_volume(id, 1.0); // nothing plays any more: nothing happens, nothing panics
    assert_eq!(audio.voice_count(), 0);
}

#[test]
fn a_placed_clip_keeps_its_volume_when_the_listener_moves() {
    use orr_audio::Listener;
    let mut audio = mixer();
    audio.play(&flat(48_000).at([0.0, 0.0, 0.0], 5.0, 50.0).volume(0.5).looped()).unwrap();
    let still = render(&mut audio, 4800);
    let level = peak(&still[still.len() - 1000..]);
    assert!((level - 0.25).abs() < 0.02, "full distance volume times the clip volume: {level}");
    audio.set_listener(Listener { position: [0.0, 0.0, 0.5], ..Listener::default() });
    let moved = render(&mut audio, 4800);
    let level = peak(&moved[moved.len() - 1000..]);
    assert!((level - 0.25).abs() < 0.02, "the follow keeps the clip volume: {level}");
}
