//! Arena's owned procedural hit cue and explicit optional-device policy.
use orr_bridge::ViewUpdate;
use orr_testgame::Hit;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioMode {
    Off,
    #[default]
    Auto,
    Required,
}
impl FromStr for AudioMode {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "off" => Ok(Self::Off),
            "auto" => Ok(Self::Auto),
            "required" => Ok(Self::Required),
            _ => Err("--audio needs off|auto|required".into()),
        }
    }
}

/// Original synthesized impact, authored in this source. No external asset license,
/// loading, random state or codec is needed. Quiet attack/release avoids hard edges.
#[cfg(feature = "audio")]
pub fn hit_clip() -> orr_audio::Clip {
    let rate = 48_000;
    let count = 7_200;
    let frames = (0..count)
        .map(|i| {
            let t = i as f32 / rate as f32;
            let attack = (t / 0.002).min(1.0);
            let release = (1.0 - i as f32 / count as f32).powi(3);
            let phase = std::f32::consts::TAU * (260.0 * t - 500.0 * t * t);
            let sample = 0.22 * attack * release * (phase.sin() + 0.25 * (phase * 2.7).sin());
            [sample, sample]
        })
        .collect();
    orr_audio::Clip::from_stereo(rate, frames).expect("authored hit PCM is valid")
}

pub struct ArenaAudio {
    status: String,
    #[cfg(all(feature = "audio-native", not(target_arch = "wasm32")))]
    native: Option<orr_audio::NativeAudio>,
    #[cfg(all(feature = "audio-native", not(target_arch = "wasm32")))]
    clip: orr_audio::Clip,
    #[cfg(all(feature = "audio-native", not(target_arch = "wasm32")))]
    mode: AudioMode,
    started: u64,
}
impl ArenaAudio {
    /// Off never opens a device. Auto reports and tolerates unavailability;
    /// Required fails rather than silently using a mock or muted backend.
    pub fn open(mode: AudioMode) -> Result<Self, String> {
        #[cfg(all(feature = "audio-native", not(target_arch = "wasm32")))]
        {
            let (native, status) = if mode == AudioMode::Off {
                (None, "off".into())
            } else {
                match orr_audio::NativeAudio::new_native(orr_audio::AudioConfig::default()) {
                    Ok(audio) => (Some(audio), "native stream opened (CPAL)".into()),
                    Err(e) if mode == AudioMode::Required => return Err(e.to_string()),
                    Err(e) => (None, format!("unavailable: {e}")),
                }
            };
            Ok(Self {
                native,
                status,
                mode,
                clip: hit_clip(),
                started: 0,
            })
        }
        #[cfg(not(all(feature = "audio-native", not(target_arch = "wasm32"))))]
        {
            let reason = "native output not built; use --features audio-native on a desktop target";
            if mode == AudioMode::Required {
                return Err(reason.into());
            }
            Ok(Self {
                status: if mode == AudioMode::Off {
                    "off".into()
                } else {
                    format!("unavailable: {reason}")
                },
                started: 0,
            })
        }
    }
    pub fn status(&self) -> &str {
        &self.status
    }
    pub fn started(&self) -> u64 {
        self.started
    }
    pub fn update(&mut self, update: &ViewUpdate<Hit>) -> Result<(), String> {
        #[cfg(all(feature = "audio-native", not(target_arch = "wasm32")))]
        if let Some(audio) = &mut self.native {
            if let Some(error) = audio.device_error() {
                self.status = error.to_string();
                self.native = None;
                if self.mode == AudioMode::Required {
                    return Err(self.status.clone());
                }
                eprintln!("audio: {}", self.status);
                return Ok(());
            }
            audio.update(0, update, |_| Some(self.clip.clone()));
            self.started = audio.stats().started;
        }
        #[cfg(not(all(feature = "audio-native", not(target_arch = "wasm32"))))]
        let _ = update;
        Ok(())
    }
}

#[cfg(test)]
#[path = "arena_audio_tests.rs"]
mod tests;
