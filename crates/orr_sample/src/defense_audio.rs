//! Optional presentation-only Stormglass soundbank owner. No simulation writes.
//! Local restart generations are monotone. Speculative restart rollback is not supported.
#![allow(clippy::float_arithmetic)]
use crate::arena_audio::AudioMode;
#[cfg(test)]
use orr_audio::OfflineAudio;
#[cfg(any(test, all(feature = "audio-native", not(target_arch = "wasm32"))))]
use orr_audio::{AudioConfig, Clip};
use orr_games::coop_defense_game::DefenseEvent;
#[cfg(any(test, all(feature = "audio-native", not(target_arch = "wasm32"))))]
use orr_games::coop_defense_game::DefenseEventKind as Kind;
use orr_package::coop_audio_admission::AdmittedBank;
use orr_session::EventBatch;
#[cfg(any(test, all(feature = "audio-native", not(target_arch = "wasm32"))))]
use orr_session::{EventKey, EventStatus};
#[cfg(any(test, all(feature = "audio-native", not(target_arch = "wasm32"))))]
use std::collections::BTreeMap;

#[cfg(any(test, all(feature = "audio-native", not(target_arch = "wasm32"))))]
pub(crate) const MAX_VOICES: usize = 12;
#[cfg(any(test, all(feature = "audio-native", not(target_arch = "wasm32"))))]
pub(crate) fn config() -> AudioConfig {
    AudioConfig {
        max_voices: MAX_VOICES,
        ..AudioConfig::default()
    }
}

/// Same event/control interface for real offline Kira and real CPAL output.
#[cfg(any(test, all(feature = "audio-native", not(target_arch = "wasm32"))))]
pub(crate) trait AudioOutput {
    fn create() -> Result<Self, String>
    where
        Self: Sized;
    fn event(&mut self, key: EventKey, event: &EventStatus<DefenseEvent>, clip: Option<Clip>);
    fn pause(&mut self, paused: bool);
    fn gain(&mut self, gain: f32) -> Result<(), String>;
    fn reset(&mut self);
    fn poll(&mut self) -> Result<(), String>;
}
#[cfg(any(test, all(feature = "audio-native", not(target_arch = "wasm32"))))]
macro_rules! output {
    ($ty:ty, $create:expr, $poll:expr) => {
        impl AudioOutput for $ty {
            fn create() -> Result<Self, String> {
                ($create)().map_err(|e| e.to_string())
            }
            fn event(
                &mut self,
                key: EventKey,
                event: &EventStatus<DefenseEvent>,
                clip: Option<Clip>,
            ) {
                self.event(key, event, |_| clip.clone());
            }
            fn pause(&mut self, paused: bool) {
                self.set_paused(paused);
            }
            fn gain(&mut self, gain: f32) -> Result<(), String> {
                self.set_gain(gain).map_err(|e| e.to_string())
            }
            fn reset(&mut self) {
                self.reset();
            }
            fn poll(&mut self) -> Result<(), String> {
                self.reap();
                ($poll)(self)
            }
        }
    };
}
#[cfg(test)]
output!(
    OfflineAudio,
    || OfflineAudio::new_offline_with_gain(config(), 48_000, 0.0),
    |_: &mut OfflineAudio| Ok(())
);
#[cfg(all(feature = "audio-native", not(target_arch = "wasm32")))]
output!(
    orr_audio::NativeAudio,
    || orr_audio::NativeAudio::new_native_with_gain(config(), 0.0),
    |a: &mut orr_audio::NativeAudio| { a.device_error().map_or(Ok(()), |e| Err(e.to_string())) }
);

#[cfg(any(test, all(feature = "audio-native", not(target_arch = "wasm32"))))]
pub(crate) struct Consumer<S: AudioOutput> {
    pub(crate) output: S,
    clips: BTreeMap<String, Clip>,
    generation: u32,
    restart: Option<EventKey>,
    baseline: u64,
    muted: bool,
    paused: bool,
    focused: bool,
    gain: f32,
    #[cfg(any(test, all(feature = "coop-defense-run-save", target_os = "linux")))]
    pub(crate) bindings: u64,
}
#[cfg(any(test, all(feature = "audio-native", not(target_arch = "wasm32"))))]
impl<S: AudioOutput> Consumer<S> {
    pub(crate) fn new(bank: &AdmittedBank) -> Result<Self, String> {
        // Native/offline factories initialize the mixer at silence, before its first block.
        let mut output = S::create()?;
        output.pause(true);
        let mut clips = BTreeMap::new();
        for record in bank.records() {
            let samples = record.pcm_bytes()[8..]
                .chunks_exact(2)
                .map(|v| {
                    let value = f32::from(i16::from_le_bytes([v[0], v[1]])) / 32768.0;
                    [value, value]
                })
                .collect();
            clips.insert(
                record.cue().to_owned(),
                Clip::from_stereo(48_000, samples).map_err(|e| e.to_string())?,
            );
        }
        Ok(Self {
            output,
            clips,
            generation: 0,
            restart: None,
            baseline: 0,
            muted: true,
            paused: true,
            focused: true,
            gain: 1.0,
            #[cfg(any(test, all(feature = "coop-defense-run-save", target_os = "linux")))]
            bindings: 0,
        })
    }
    pub(crate) fn controls(
        &mut self,
        muted: bool,
        gain: f32,
        paused: bool,
        focused: bool,
    ) -> Result<(), String> {
        if !gain.is_finite() || !(0.0..=1.0).contains(&gain) {
            return Err("audio gain must be finite and within 0..=1".into());
        }
        self.muted = muted;
        self.gain = gain;
        self.paused = paused;
        self.focused = focused;
        self.sync()
    }
    fn sync(&mut self) -> Result<(), String> {
        let suppressed = self.muted || self.paused || !self.focused;
        self.output.pause(suppressed);
        self.output.gain(if suppressed {
            0.0
        } else {
            self.gain / MAX_VOICES as f32
        })
    }
    #[cfg(any(test, all(feature = "coop-defense-run-save", target_os = "linux")))]
    pub(crate) fn replace(&mut self, tick: u64, generation: u32) -> Result<(), String> {
        self.output.reset();
        self.paused = true;
        self.generation = generation;
        self.baseline = tick;
        self.restart = None;
        self.bindings = self.bindings.saturating_add(1);
        self.sync()
    }
    pub(crate) fn consume(&mut self, batch: &EventBatch<DefenseEvent>) -> Result<(), String> {
        self.consume_items(batch.iter().cloned())
    }
    fn consume_items(
        &mut self,
        items: impl IntoIterator<Item = (EventKey, EventStatus<DefenseEvent>)>,
    ) -> Result<(), String> {
        let items: Vec<_> = items.into_iter().collect();
        // Preflight every record before changing output or epoch ownership.
        let mut generation = self.generation;
        let mut restart = self.restart;
        for (key, status) in &items {
            if key.system_index != 0 || key._pad != 0 {
                return Err("unsupported audio event producer".into());
            }
            if key.tick <= self.baseline {
                continue;
            }
            if let EventStatus::Predicted(e) | EventStatus::Verified(e) = status {
                let kind = e.event_kind().ok_or("unsupported audio event kind")?;
                if kind == Kind::Restart && Some(*key) != restart {
                    if generation.checked_add(1) != Some(e.generation) {
                        return Err("invalid audio restart generation".into());
                    }
                    generation = e.generation;
                    restart = Some(*key);
                }
                if e.generation != generation {
                    return Err("stale audio event generation".into());
                }
            } else if Some(*key) == restart {
                return Err("speculative restart rollback is unsupported by local audio".into());
            }
        }
        for (key, status) in items {
            if key.tick <= self.baseline {
                continue;
            }
            let payload = match &status {
                EventStatus::Predicted(e) | EventStatus::Verified(e) => Some(e),
                EventStatus::Canceled => None,
            };
            if let Some(e) = payload {
                if e.event_kind() == Some(Kind::Restart) && self.restart != Some(key) {
                    self.output.reset();
                    self.generation = e.generation;
                    self.restart = Some(key);
                    self.sync()?;
                }
            }
            let clip = payload
                .and_then(cue)
                .and_then(|name| self.clips.get(name))
                .cloned();
            self.output.event(key, &status, clip);
        }
        Ok(())
    }
}
#[cfg(any(test, all(feature = "audio-native", not(target_arch = "wasm32"))))]
fn cue(e: &DefenseEvent) -> Option<&'static str> {
    match e.event_kind()? {
        Kind::Summon => Some("summon"),
        Kind::Merge => Some("merge"),
        Kind::Death => Some("death"),
        Kind::Release => match e.role {
            0 => Some("attack-light"),
            1 | 2 => Some("attack-heavy"),
            3 => Some("attack-magic"),
            _ => None,
        },
        Kind::Hit => match e.role {
            0 | 3 => Some("impact-light"),
            1 | 2 => Some("impact-heavy"),
            _ => None,
        },
        _ => None,
    }
}

enum Output {
    Disabled,
    #[cfg(all(feature = "audio-native", not(target_arch = "wasm32")))]
    Native(Box<Consumer<orr_audio::NativeAudio>>),
    #[cfg(test)]
    Offline(Box<Consumer<OfflineAudio>>),
}
pub(crate) struct DefenseAudio {
    output: Output,
    mode: AudioMode,
    status: String,
    #[cfg(test)]
    pub(crate) injected_error: Option<String>,
    #[cfg(all(test, feature = "coop-defense-run-save", target_os = "linux"))]
    pub(crate) injected_replace_error: Option<String>,
}
impl Default for DefenseAudio {
    fn default() -> Self {
        Self {
            output: Output::Disabled,
            mode: AudioMode::Off,
            status: "off".into(),
            #[cfg(test)]
            injected_error: None,
            #[cfg(all(test, feature = "coop-defense-run-save", target_os = "linux"))]
            injected_replace_error: None,
        }
    }
}
macro_rules! dispatch {
    ($self:ident, $owner:ident, $body:expr) => {
        match &mut $self.output {
            Output::Disabled => Ok(()),
            #[cfg(all(feature = "audio-native", not(target_arch = "wasm32")))]
            Output::Native($owner) => $body,
            #[cfg(test)]
            Output::Offline($owner) => $body,
        }
    };
}
impl DefenseAudio {
    pub(crate) fn open(bank: Option<&AdmittedBank>, mode: AudioMode) -> Result<Self, String> {
        let mut result = Self {
            mode,
            ..Self::default()
        };
        if mode == AudioMode::Off {
            return Ok(result);
        }
        let Some(bank) = bank else {
            if mode == AudioMode::Required {
                return Err("required audio needs --audio-bank".into());
            }
            result.status = "no bank".into();
            return Ok(result);
        };
        #[cfg(all(feature = "audio-native", not(target_arch = "wasm32")))]
        let native = Consumer::<orr_audio::NativeAudio>::new(bank)
            .map(Box::new)
            .map(Output::Native);
        #[cfg(not(all(feature = "audio-native", not(target_arch = "wasm32"))))]
        let native: Result<Output, String> = {
            let _ = bank;
            Err("native audio unavailable: build with audio-native".into())
        };
        match native {
            Ok(output) => {
                result.output = output;
                result.status = "native stream open".into();
            }
            Err(error) => result.failure(error)?,
        }
        Ok(result)
    }
    fn failure(&mut self, error: String) -> Result<(), String> {
        self.output = Output::Disabled;
        self.status = error
            .chars()
            .take(160)
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        if self.mode == AudioMode::Required {
            Err(self.status.clone())
        } else {
            Ok(())
        }
    }
    pub(crate) fn enabled(&self) -> bool {
        !matches!(self.output, Output::Disabled)
    }
    pub(crate) fn status(&self) -> &str {
        &self.status
    }
    pub(crate) fn poll(&mut self) -> Result<(), String> {
        #[cfg(test)]
        if let Some(error) = self.injected_error.take() {
            return self.failure(error);
        }
        let result: Result<(), String> = dispatch!(self, owner, owner.output.poll());
        result.or_else(|e| self.failure(e))
    }
    pub(crate) fn controls(
        &mut self,
        muted: bool,
        gain: f32,
        paused: bool,
        focused: bool,
    ) -> Result<(), String> {
        // Arguments stay used when both native and test backends are compiled out.
        let _ = (muted, gain, paused, focused);
        dispatch!(self, owner, owner.controls(muted, gain, paused, focused))
    }
    pub(crate) fn consume(&mut self, batch: &EventBatch<DefenseEvent>) -> Result<(), String> {
        let _ = batch;
        dispatch!(self, owner, owner.consume(batch))
    }
    #[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
    pub(crate) fn replace(&mut self, tick: u64, generation: u32) -> Result<(), String> {
        #[cfg(test)]
        if let Some(error) = self.injected_replace_error.take() {
            return self.failure(error);
        }
        let _ = (tick, generation);
        let result: Result<(), String> = dispatch!(self, owner, owner.replace(tick, generation));
        result.or_else(|e| self.failure(e))
    }
    #[cfg(test)]
    pub(crate) fn offline(bank: &AdmittedBank, mode: AudioMode) -> Self {
        Self {
            output: Output::Offline(Box::new(Consumer::new(bank).unwrap())),
            mode,
            status: "offline test output".into(),
            injected_error: None,
            #[cfg(all(test, feature = "coop-defense-run-save", target_os = "linux"))]
            injected_replace_error: None,
        }
    }
    #[cfg(test)]
    pub(crate) fn offline_owner(&mut self) -> &mut Consumer<OfflineAudio> {
        match &mut self.output {
            Output::Offline(owner) => owner,
            _ => panic!("offline output required"),
        }
    }
}
#[cfg(test)]
#[path = "defense_audio_tests.rs"]
pub(crate) mod tests;
