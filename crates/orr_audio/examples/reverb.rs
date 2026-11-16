//! Renders a click and tone sequence dry and in a hall to two WAV files.
//! Run `cargo run --release -p orr_audio --example reverb -- <folder>`.
#![allow(clippy::float_arithmetic)]

use std::path::Path;

use orr_audio::{AudioConfig, Clip, OfflineAudio, ReverbParams, ReverbZone};
use orr_bridge::{EventKey, EventStatus};

const RATE: u32 = 48_000;

fn click() -> Clip {
    // A short decaying noise burst from a fixed linear congruential sequence.
    let mut state = 12_345u32;
    let frames = (0..960)
        .map(|i| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let v = ((state >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0) * 0.6 * (-(i as f32) / 120.0).exp();
            [v, v]
        })
        .collect();
    Clip::from_stereo(RATE, frames).expect("valid clip")
}

fn blip(hz: f32) -> Clip {
    let frames = (0..RATE as usize / 5)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let v = 0.35 * (t * std::f32::consts::TAU * hz).sin() * (-t * 18.0).exp();
            [v, v]
        })
        .collect();
    Clip::from_stereo(RATE, frames).expect("valid clip")
}

fn render(hall: bool) -> Vec<f32> {
    let mut audio = OfflineAudio::new_offline(AudioConfig::default(), RATE).expect("offline audio");
    if hall {
        audio.add_reverb_zone(ReverbZone::sphere([0.0; 3], 20.0, ReverbParams::HALL)).expect("zone");
    }
    let mut out = Vec::new();
    // Sounds three meters ahead, two seconds apart, alternating sides.
    let sounds = [(click(), -1.0), (blip(440.0), 1.0), (click(), 1.0), (blip(660.0), -1.0), (blip(330.0), 0.0)];
    for (tick, (clip, x)) in sounds.into_iter().enumerate() {
        let clip = clip.at([x, 0.0, -3.0], 2.0, 60.0);
        audio.event(EventKey::new(tick as u64, 0, 0), &EventStatus::Verified(()), |_| Some(clip.clone()));
        let mut block = vec![0.0; RATE as usize * 2 * if tick == 4 { 6 } else { 2 }];
        for chunk in block.chunks_mut(512) {
            audio.render(chunk).expect("render");
        }
        out.extend(block);
    }
    out
}

fn write_wav(path: &Path, samples: &[f32]) -> std::io::Result<()> {
    let data: Vec<u8> = samples.iter().flat_map(|v| ((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).collect();
    let mut file = Vec::with_capacity(44 + data.len());
    file.extend(b"RIFF");
    file.extend((36 + data.len() as u32).to_le_bytes());
    file.extend(b"WAVEfmt ");
    file.extend(16u32.to_le_bytes());
    file.extend(1u16.to_le_bytes()); // PCM
    file.extend(2u16.to_le_bytes()); // stereo
    file.extend(RATE.to_le_bytes());
    file.extend((RATE * 4).to_le_bytes());
    file.extend(4u16.to_le_bytes());
    file.extend(16u16.to_le_bytes());
    file.extend(b"data");
    file.extend((data.len() as u32).to_le_bytes());
    file.extend(data);
    std::fs::write(path, file)
}

fn main() -> std::io::Result<()> {
    let folder = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    for (name, hall) in [("reverb_dry.wav", false), ("reverb_hall.wav", true)] {
        let path = Path::new(&folder).join(name);
        write_wav(&path, &render(hall))?;
        println!("wrote {}", path.display());
    }
    Ok(())
}
