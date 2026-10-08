//! Presentation-only ownership of one admitted Collect cue. No filesystem reads.
pub use crate::arena_audio::AudioMode;
use crate::collect_audio::{Document, PreparedAudio};
use orr_audio::{AudioConfig, AudioStats, OfflineAudio};
use orr_bridge::ViewUpdate;

pub struct Playback {
    prepared: PreparedAudio,
    output: Output,
    status: String,
    mode: AudioMode,
}
enum Output {
    Silent,
    Offline(Box<OfflineAudio>),
    #[cfg(all(feature = "collect-audio-native", not(target_arch = "wasm32")))]
    Native(Box<orr_audio::NativeAudio>),
}
impl Playback {
    /// Asset admission has already completed, including when output is Off.
    pub fn open(prepared: PreparedAudio, mode: AudioMode) -> Result<Self, String> {
        let mut owner = Self {
            prepared,
            output: Output::Silent,
            status: "off (no device)".into(),
            mode,
        };
        if mode == AudioMode::Off {
            return Ok(owner);
        }
        #[cfg(all(feature = "collect-audio-native", not(target_arch = "wasm32")))]
        {
            match orr_audio::NativeAudio::new_native(AudioConfig::default()) {
                Ok(mixer) => {
                    owner.output = Output::Native(Box::new(mixer));
                    owner.status = "native stream opened; physical hearing unverified".into();
                    return Ok(owner);
                }
                Err(error) => owner.status = format!("unavailable: {error}"),
            }
        }
        #[cfg(not(all(feature = "collect-audio-native", not(target_arch = "wasm32"))))]
        {
            owner.status =
                "unavailable: native output not built (collect-audio-native required)".into();
        }
        if mode == AudioMode::Required {
            Err(owner.status)
        } else {
            Ok(owner)
        }
    }
    /// Explicit real Kira offline output for acceptance; never opens a device.
    pub fn offline(prepared: PreparedAudio) -> Result<Self, String> {
        Ok(Self {
            prepared,
            output: Output::Offline(Box::new(
                OfflineAudio::new_offline(AudioConfig::default(), 48_000)
                    .map_err(|e| e.to_string())?,
            )),
            status: "offline Kira renderer (no device)".into(),
            mode: AudioMode::Off,
        })
    }
    pub fn status(&self) -> &str {
        &self.status
    }
    pub fn document(&self) -> &Document {
        &self.prepared.document
    }
    pub fn set_document(&mut self, document: Document) -> Result<(), String> {
        let next = self.prepared.with_document(document)?;
        // Existing tails cannot retain the old gain/mute/assignment.
        self.stop();
        self.prepared = next;
        Ok(())
    }
    pub fn stop(&mut self) {
        match &mut self.output {
            Output::Silent => {}
            Output::Offline(mixer) => mixer.stop_transients(),
            #[cfg(all(feature = "collect-audio-native", not(target_arch = "wasm32")))]
            Output::Native(mixer) => mixer.stop_transients(),
        }
    }
    pub fn reset(&mut self) {
        match &mut self.output {
            Output::Silent => {}
            Output::Offline(mixer) => mixer.reset(),
            #[cfg(all(feature = "collect-audio-native", not(target_arch = "wasm32")))]
            Output::Native(mixer) => mixer.reset(),
        }
    }
    pub fn stats(&self) -> AudioStats {
        match &self.output {
            Output::Silent => AudioStats::default(),
            Output::Offline(mixer) => mixer.stats(),
            #[cfg(all(feature = "collect-audio-native", not(target_arch = "wasm32")))]
            Output::Native(mixer) => mixer.stats(),
        }
    }
    /// Consume the caller's complete atomic update once, before it moves snapshot.
    /// Mute maps to no clip but still lets EventAudio retain every event identity.
    pub fn update<E>(
        &mut self,
        source: u64,
        update: &ViewUpdate<E>,
        pickup: impl Fn(&E) -> bool,
    ) -> Result<(), String> {
        #[cfg(all(feature = "collect-audio-native", not(target_arch = "wasm32")))]
        if let Output::Native(mixer) = &mut self.output {
            if let Some(error) = mixer.device_error() {
                self.output = Output::Silent;
                self.status = format!("unavailable: {error}");
                eprintln!("Collect audio: {}", self.status);
                if self.mode == AudioMode::Required {
                    return Err(self.status.clone());
                }
            }
        }
        let clip = if self.prepared.document.mute {
            None
        } else {
            Some(self.prepared.pickup_clip()?)
        };
        let map = |event: &E| if pickup(event) { clip.clone() } else { None };
        match &mut self.output {
            Output::Silent => {}
            Output::Offline(mixer) => mixer.update(source, update, map),
            #[cfg(all(feature = "collect-audio-native", not(target_arch = "wasm32")))]
            Output::Native(mixer) => mixer.update(source, update, map),
        }
        // Keep explicit policy available without native feature unification.
        let _ = self.mode;
        Ok(())
    }
    pub fn render(&mut self, stereo: &mut [f32]) -> Result<(), String> {
        match &mut self.output {
            Output::Offline(mixer) => mixer.render(stereo).map_err(|e| e.to_string()),
            Output::Silent => {
                stereo.fill(0.0);
                Ok(())
            }
            #[cfg(all(feature = "collect-audio-native", not(target_arch = "wasm32")))]
            Output::Native(_) => Err("native output cannot be read as offline PCM".into()),
        }
    }
}

#[cfg(test)]
#[path = "collect_audio_output_tests.rs"]
mod tests;
