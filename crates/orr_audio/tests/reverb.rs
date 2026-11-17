#![allow(clippy::float_arithmetic)]

use orr_audio::{AudioConfig, Clip, Listener, OfflineAudio, ReverbParams, ReverbZone};
use orr_bridge::{EventKey, EventStatus, ViewUpdate};

const RATE: u32 = 48_000;

fn mixer() -> OfflineAudio {
    OfflineAudio::new_offline(AudioConfig::default(), RATE).unwrap()
}
fn tone(seconds: f32) -> Clip {
    let frames = (RATE as f32 * seconds) as usize;
    Clip::from_stereo(
        RATE,
        (0..frames)
            .map(|i| {
                let v = 0.2 * (i as f32 * std::f32::consts::TAU * 330.0 / RATE as f32).sin();
                [v, v]
            })
            .collect(),
    )
    .unwrap()
}
fn click() -> Clip {
    Clip::from_stereo(RATE, vec![[0.5, 0.5]]).unwrap()
}
fn play(audio: &mut OfflineAudio, tick: u64, clip: &Clip) {
    audio.event(EventKey::new(tick, 0, 0), &EventStatus::Verified(()), |_| Some(clip.clone()));
}
fn render(audio: &mut OfflineAudio, frames: usize) -> Vec<f32> {
    let mut samples = vec![0.0; frames * 2];
    for block in samples.chunks_mut(256) {
        audio.render(block).unwrap();
    }
    samples
}
/// Energy of `samples` in windows of `window` frames.
fn energies(samples: &[f32], window: usize) -> Vec<f32> {
    samples.chunks(window * 2).map(|w| w.iter().map(|v| v * v).sum()).collect()
}
fn room(room_size: f32) -> ReverbParams {
    ReverbParams { room_size, damping: 0.3, pre_delay: 0.0, wet: 0.0 }
}

/// Renders a click sent fully into a reverb of `room_size`.
fn tail(room_size: f32) -> Vec<f32> {
    let mut audio = mixer();
    audio.set_ambient_reverb(room(room_size)).unwrap();
    play(&mut audio, 1, &click().reverb(1.0));
    render(&mut audio, RATE as usize * 3)
}

#[test]
fn an_impulse_leaves_a_decaying_tail_that_is_longer_in_a_larger_room() {
    let small = energies(&tail(0.2), 4800);
    let large = energies(&tail(0.95), 4800);
    for (name, tail) in [("small", &small), ("large", &large)] {
        assert!(tail[1] > 0.0, "{name} room has a tail");
        // Past the first echoes the energy only falls, window after window.
        for pair in tail[3..].windows(2) {
            assert!(pair[1] <= pair[0] * 1.02, "{name} room tail must decay: {pair:?}");
        }
        assert!(tail[tail.len() - 1] < tail[3] * 0.5, "{name} room tail falls over the render");
    }
    // Time for the tail to fall 20 dB below its level at 100 ms.
    let decay = |tail: &[f32]| tail[2..].iter().position(|e| *e < tail[2] * 0.01).unwrap_or(tail.len());
    assert!(decay(&large) > decay(&small) * 2, "large room decays in {} windows, small in {}", decay(&large), decay(&small));
    assert!(large[20] > small[20] * 20.0, "late in the tail the large room is far louder");
}

#[test]
fn pre_delay_holds_the_first_echoes_back() {
    let first_echo = |pre_delay: f32| {
        let mut audio = mixer();
        audio.set_ambient_reverb(ReverbParams { pre_delay, ..room(0.5) }).unwrap();
        play(&mut audio, 1, &click().reverb(1.0));
        let samples = render(&mut audio, 9600);
        // Skip the click itself at frame 0.
        samples.chunks(2).skip(1).position(|f| f[0].abs() > 1e-5).unwrap()
    };
    let none = first_echo(0.0);
    let delayed = first_echo(0.05);
    assert!(delayed >= none + 2300, "a 50 ms pre-delay moves the first echo from {none} to {delayed} frames");
}

#[test]
fn without_a_send_or_a_covering_zone_the_output_is_untouched() {
    let spatial = tone(0.5).at([2.0, 0.0, -3.0], 1.0, 50.0);
    let reference = {
        let mut audio = mixer();
        play(&mut audio, 1, &spatial);
        play(&mut audio, 2, &tone(0.5));
        render(&mut audio, 24_000)
    };
    // A zone the listener is nowhere near, an outdoor zone around it, and an explicit send of zero.
    for setup in 0..3 {
        let mut audio = mixer();
        match setup {
            0 => drop(audio.add_reverb_zone(ReverbZone::cuboid([500.0, 0.0, 0.0], [5.0; 3], ReverbParams::CAVE).fade(0.0)).unwrap()),
            1 => drop(audio.add_reverb_zone(ReverbZone::sphere([0.0; 3], 10.0, ReverbParams::OUTDOOR)).unwrap()),
            _ => {}
        }
        let (spatial, plain) = if setup == 2 { (spatial.clone().reverb(0.0), tone(0.5).reverb(0.0)) } else { (spatial.clone(), tone(0.5)) };
        play(&mut audio, 1, &spatial);
        play(&mut audio, 2, &plain);
        let out = render(&mut audio, 24_000);
        assert!(out.iter().zip(&reference).all(|(a, b)| a.to_bits() == b.to_bits()), "setup {setup} changed the output");
    }
}

#[test]
fn a_zone_reverberates_placed_sounds_only() {
    let wet = |clip: Clip| {
        let mut audio = mixer();
        audio.add_reverb_zone(ReverbZone::sphere([0.0; 3], 10.0, ReverbParams::HALL)).unwrap();
        play(&mut audio, 1, &clip);
        let mut dry = mixer();
        play(&mut dry, 1, &clip);
        let (with, without) = (render(&mut audio, 24_000), render(&mut dry, 24_000));
        with.iter().zip(&without).map(|(a, b)| (a - b) * (a - b)).sum::<f32>()
    };
    assert!(wet(tone(0.3).at([0.0, 0.0, -2.0], 100.0, 200.0)) > 0.01, "a placed sound in the hall has reverb");
    assert_eq!(wet(tone(0.3)), 0.0, "a sound that is not placed has none");
}

#[test]
fn a_clip_send_reverberates_a_sound_that_is_not_placed() {
    let level = |send: f32| {
        let mut audio = mixer();
        audio.set_ambient_reverb(room(0.6)).unwrap();
        play(&mut audio, 1, &click().reverb(send));
        energies(&render(&mut audio, 24_000), 24_000)[0]
    };
    // Energy of the click itself, without a send.
    let dry = level(0.0);
    let (half, full) = (level(0.5) - dry, level(1.0) - dry);
    assert!(half > 0.0);
    assert!((full / half - 4.0).abs() < 0.2, "the send scales the amplitude: {half} at 0.5, {full} at 1");
}

/// A tone at the origin, heard by a listener walking away along z through a hall,
/// with the same walk without the hall: their difference is the reverb alone.
fn walk(with_zone: bool) -> Vec<f32> {
    let mut audio = mixer();
    if with_zone {
        audio.add_reverb_zone(ReverbZone::sphere([0.0; 3], 5.0, ReverbParams::HALL).fade(5.0)).unwrap();
    }
    play(&mut audio, 1, &tone(4.0).at([0.0; 3], 100.0, 200.0));
    let mut out = Vec::new();
    for block in 0..720 {
        audio.set_listener(Listener { position: [0.0, 0.0, 0.05 * block as f32], ..Listener::default() });
        out.extend(render(&mut audio, 256));
    }
    out
}

#[test]
fn walking_out_of_a_zone_fades_its_reverb_without_jumps() {
    let (with, without) = (walk(true), walk(false));
    let wet: Vec<f32> = with.iter().zip(&without).map(|(a, b)| a - b).collect();
    let windows = energies(&wet, 9600);
    eprintln!("{windows:?}");
    let inside = windows[1];
    assert!(inside > 0.01, "reverb is heard inside the zone: {inside}");
    // The listener leaves the 5 m sphere at 0.53 s and is 10 m away at 1.06 s (windows of 0.2 s).
    for pair in windows[3..].windows(2) {
        assert!(pair[1] <= pair[0] * 1.1 + 1e-6, "the reverb only fades: {pair:?}");
    }
    assert!(*windows.last().unwrap() < inside * 0.001, "far outside the zone the reverb is gone");
    let peak = wet.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let step = wet.chunks(2).zip(wet.chunks(2).skip(1)).fold(0.0f32, |m, (a, b)| m.max((b[0] - a[0]).abs()));
    assert!(step < peak * 0.12, "no sample jumps: largest step {step} against peak {peak}");
}

#[test]
fn zones_blend_by_weight_and_reject_invalid_input() {
    let mut audio = mixer();
    let hall = audio.add_reverb_zone(ReverbZone::sphere([0.0; 3], 10.0, ReverbParams::HALL).fade(10.0)).unwrap();
    assert_eq!(audio.reverb_at([0.0; 3]), ReverbParams::HALL);
    assert_eq!(audio.reverb_at([30.0, 0.0, 0.0]), ReverbParams::OUTDOOR);
    let edge = audio.reverb_at([15.0, 0.0, 0.0]);
    assert!((edge.wet - ReverbParams::HALL.wet * 0.5).abs() < 1e-6, "halfway through the fade: {edge:?}");
    let cave = audio.add_reverb_zone(ReverbZone::cuboid([0.0; 3], [10.0; 3], ReverbParams::CAVE)).unwrap();
    let both = audio.reverb_at([0.0; 3]);
    assert!((both.wet - 0.425).abs() < 1e-6 && both.room_size > 0.8 && both.room_size < 0.92, "two covering zones average: {both:?}");
    assert!(audio.remove_reverb_zone(cave) && !audio.remove_reverb_zone(cave));
    assert!(audio.remove_reverb_zone(hall));
    assert_eq!(audio.reverb_at([0.0; 3]), ReverbParams::OUTDOOR);
    assert!(audio.add_reverb_zone(ReverbZone::sphere([0.0; 3], 0.0, ReverbParams::HALL)).is_err());
    assert!(audio.add_reverb_zone(ReverbZone::sphere([f32::NAN; 3], 1.0, ReverbParams::HALL)).is_err());
    assert!(audio.add_reverb_zone(ReverbZone::sphere([0.0; 3], 1.0, ReverbParams { wet: f32::NAN, ..ReverbParams::HALL })).is_err());
    assert!(audio.add_reverb_zone(ReverbZone::sphere([0.0; 3], 1.0, ReverbParams::HALL).fade(-1.0)).is_err());
    for i in 0..orr_audio::MAX_REVERB_ZONES {
        audio.add_reverb_zone(ReverbZone::sphere([i as f32; 3], 1.0, ReverbParams::HALL)).unwrap();
    }
    assert!(audio.add_reverb_zone(ReverbZone::sphere([0.0; 3], 1.0, ReverbParams::HALL)).is_err());
    audio.clear_reverb_zones();
    assert!(audio.set_ambient_reverb(ReverbParams { damping: f32::INFINITY, ..ReverbParams::HALL }).is_err());
}

/// How a session ends: each way stops the old voices, so each must also silence the bus.
#[derive(Clone, Copy, Debug)]
enum Boundary {
    Reset,
    Pause,
    SourceSwap,
}

/// A click sent fully into a long-tail room, 100 ms of it rendered, then the boundary (or
/// nothing), then one second more. Returns the energy of that second after a 100 ms fade window.
fn energy_after(boundary: Option<Boundary>) -> f32 {
    let mut audio = mixer();
    audio.set_ambient_reverb(ReverbParams { wet: 1.0, ..ReverbParams::CAVE }).unwrap();
    let empty = ViewUpdate::<()> { snapshot: None, events: Vec::new(), resync: None };
    audio.update(1, &empty, |_| None);
    play(&mut audio, 1, &click().reverb(1.0));
    let before = render(&mut audio, 4800);
    assert!(energies(&before, 4800)[0] > 0.0, "the reverb is already ringing at the boundary");
    match boundary {
        Some(Boundary::Reset) => audio.reset(),
        Some(Boundary::Pause) => audio.set_paused(true),
        Some(Boundary::SourceSwap) => audio.update(2, &empty, |_| None),
        None => {}
    }
    let after = render(&mut audio, RATE as usize);
    energies(&after[4800 * 2..], 43_200)[0]
}

#[test]
fn a_reverb_tail_rings_on_without_a_boundary_so_the_clear_below_proves_something() {
    assert!(energy_after(None) > 1e-6, "the cave tail is audible one second later");
}

#[test]
fn reset_pause_and_a_source_swap_silence_the_reverb_tail() {
    let ringing = energy_after(None);
    for boundary in [Boundary::Reset, Boundary::Pause, Boundary::SourceSwap] {
        let cleared = energy_after(Some(boundary));
        assert!(cleared < ringing * 1e-6, "{boundary:?}: energy {cleared} after the fade against {ringing} without a boundary");
    }
}

#[test]
fn sounds_after_a_reset_still_get_their_reverb_and_leaving_a_zone_keeps_the_natural_tail() {
    let mut audio = mixer();
    audio.set_ambient_reverb(ReverbParams { wet: 1.0, ..ReverbParams::CAVE }).unwrap();
    play(&mut audio, 1, &click().reverb(1.0));
    render(&mut audio, 4800);
    audio.reset();
    render(&mut audio, 9600);
    play(&mut audio, 2, &click().reverb(1.0));
    let again = render(&mut audio, 24_000);
    assert!(energies(&again, 4800)[1] > 0.0, "a click after the reset has a tail again");

    // Leaving the zone: the ambient reverb turns off, the sound that already went in keeps ringing.
    let mut audio = mixer();
    audio.set_ambient_reverb(ReverbParams { wet: 1.0, ..ReverbParams::CAVE }).unwrap();
    play(&mut audio, 1, &click().reverb(1.0));
    render(&mut audio, 4800);
    audio.set_ambient_reverb(ReverbParams::OUTDOOR).unwrap();
    let after = render(&mut audio, RATE as usize);
    assert!(energies(&after[4800 * 2..], 43_200)[0] > 1e-6, "the natural tail survives leaving the zone");
}
