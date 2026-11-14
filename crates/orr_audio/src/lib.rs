//! Presentation-only rollback-aware one-shot audio, using Kira's real mixer.
//! A clip placed with [`Clip::at`] is heard from [`EventAudio::set_listener`]:
//! its volume falls with distance and it pans toward its side.
//!
//! The default build has no device or codec dependencies. [`OfflineAudio`] renders
//! PCM into a caller-owned buffer; `native` enables CPAL output. Nothing feeds back
//! into simulation, checksums or replay files. Feed each ordered [`ViewUpdate`]
//! once, and use a different source ID when replacing its bridge/session owner.
#![allow(clippy::float_arithmetic)]
#![allow(clippy::disallowed_types)]

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use kira::backend::Backend;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle, StaticSoundSettings};
use kira::sound::PlaybackState;
use kira::track::MainTrackBuilder;
use kira::{AudioManager, AudioManagerSettings, Capacities, Decibels, Frame, Panning, Tween};
use orr_bridge::{BridgeEvent, EventKey, EventStatus, Lifecycle, ViewResync, ViewUpdate};

mod offline;
pub use offline::OfflineBackend;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioError(pub String);
impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for AudioError {}

/// Where a sound comes from in the world, and how far it carries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Emitter {
    pub position: [f32; 3],
    /// Within this many meters the sound plays at full volume.
    pub near: f32,
    /// At this many meters and beyond the sound is silent.
    pub far: f32,
}

/// The ear that spatial sounds are heard from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listener {
    pub position: [f32; 3],
    /// The listener's right, toward the right speaker. It need not be a unit vector.
    pub right: [f32; 3],
}
impl Default for Listener {
    fn default() -> Self {
        Self { position: [0.0; 3], right: [1.0, 0.0, 0.0] }
    }
}

impl Emitter {
    /// Volume in decibels (0 is full, -60 silent) and left-right placement (-1 left,
    /// 1 right) of this emitter for `listener`: full volume within `near`, falling
    /// with distance after it and reaching silence at `far`.
    pub fn heard_by(&self, listener: &Listener) -> (f32, f32) {
        let offset: [f32; 3] = std::array::from_fn(|i| self.position[i] - listener.position[i]);
        let distance = offset.iter().map(|v| v * v).sum::<f32>().sqrt();
        let near = self.near.max(1e-3);
        let far = self.far.max(near + 1e-3);
        // Inverse distance past `near`, faded to zero over the last fifth before `far`.
        let fade = ((far - distance) / (0.2 * (far - near))).clamp(0.0, 1.0);
        let amplitude = (near / distance.max(near)) * fade;
        let volume = if amplitude <= 0.001 { Decibels::SILENCE.0 } else { (20.0 * amplitude.log10()).max(Decibels::SILENCE.0) };
        let right_length = listener.right.iter().map(|v| v * v).sum::<f32>().sqrt();
        let pan = if distance > 1e-4 && right_length > 1e-6 {
            offset.iter().zip(&listener.right).map(|(o, r)| o * r).sum::<f32>() / (distance * right_length)
        } else {
            0.0
        };
        (volume, pan.clamp(-1.0, 1.0))
    }
}

/// Validated, owned PCM shared cheaply by every instance. No decoder or asset IO.
#[derive(Clone)]
pub struct Clip {
    data: StaticSoundData,
    emitter: Option<Emitter>,
}
impl Clip {
    /// This clip played from `position`, heard through [`EventAudio::set_listener`]:
    /// full volume within `near` meters, silent from `far` meters on.
    pub fn at(mut self, position: [f32; 3], near: f32, far: f32) -> Self {
        self.emitter = Some(Emitter { position, near, far });
        self
    }

    pub fn emitter(&self) -> Option<Emitter> {
        self.emitter
    }

    pub fn from_stereo(sample_rate: u32, frames: Vec<[f32; 2]>) -> Result<Self, AudioError> {
        if !(8_000..=192_000).contains(&sample_rate) || frames.is_empty() {
            return Err(AudioError(
                "clip needs nonempty PCM at 8000..=192000 Hz".into(),
            ));
        }
        if frames.len() > sample_rate as usize * 10 {
            return Err(AudioError(
                "one-shot clips are limited to 10 seconds".into(),
            ));
        }
        if frames
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 1.0)
        {
            return Err(AudioError(
                "PCM samples must be finite and within [-1, 1]".into(),
            ));
        }
        let frames: Arc<[Frame]> = frames
            .into_iter()
            .map(|[left, right]| Frame::new(left, right))
            .collect();
        Ok(Self {
            data: StaticSoundData {
                sample_rate,
                frames,
                settings: StaticSoundSettings::default(),
                slice: None,
            },
            emitter: None,
        })
    }
}

/// Explicit hard bounds. Fading/reset voices count against the voice limit too.
#[derive(Clone, Copy, Debug)]
pub struct AudioConfig {
    pub max_voices: usize,
    pub history_capacity: usize,
    pub cancel_fade: Duration,
}
impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            max_voices: 32,
            history_capacity: 4096,
            cancel_fade: Duration::from_millis(15),
        }
    }
}
impl AudioConfig {
    fn validate(self) -> Result<Self, AudioError> {
        if !(1..=1024).contains(&self.max_voices) || !(1..=65536).contains(&self.history_capacity) {
            return Err(AudioError(
                "audio bounds must be 1..=1024 voices and 1..=65536 history entries".into(),
            ));
        }
        if self.cancel_fade > Duration::from_secs(1) {
            return Err(AudioError("cancel fade must be at most one second".into()));
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AudioStats {
    pub started: u64,
    pub confirmed: u64,
    pub canceled: u64,
    pub completed: u64,
    pub duplicates: u64,
    pub dropped: u64,
    pub resets: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Seen {
    Predicted,
    Verified,
    Canceled,
}
struct Voice {
    // A detached fading voice must not be confirmed/canceled as a replacement.
    key: Option<EventKey>,
    handle: StaticSoundHandle,
    emitter: Option<Emitter>,
}

/// One owner of mixer, voice handles and bounded completion tombstones.
///
/// A Predicted→Verified pair starts once even if playback finished before
/// verification. Canceled→Predicted may start a replacement with the same key.
/// At capacity, newest voices are dropped (and tombstoned) rather than replayed
/// on confirmation. History pressure retires entire oldest ticks; their late
/// notifications stay stale forever, until a source/seek reset. This can omit
/// sounds under overload but cannot resurrect an evicted sound.
pub struct EventAudio<B: Backend> {
    manager: AudioManager<B>,
    config: AudioConfig,
    voices: Vec<Voice>,
    history: BTreeMap<EventKey, Seen>,
    retired_through: Option<u64>,
    source: Option<u64>,
    paused: bool,
    disconnected: bool,
    stats: AudioStats,
    listener: Listener,
}
pub type OfflineAudio = EventAudio<OfflineBackend>;
#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
pub type NativeAudio = EventAudio<kira::backend::cpal::CpalBackend>;

fn settings<B: Backend>(
    config: AudioConfig,
    backend_settings: B::Settings,
    initial_gain: kira::Decibels,
) -> AudioManagerSettings<B> {
    AudioManagerSettings {
        // This vertical slice uses only the main track. No hidden unbounded resources.
        capacities: Capacities {
            sub_track_capacity: 0,
            send_track_capacity: 0,
            clock_capacity: 0,
            modulator_capacity: 0,
            listener_capacity: 0,
        },
        main_track_builder: MainTrackBuilder::new()
            .volume(initial_gain)
            .sound_capacity(config.max_voices),
        internal_buffer_size: 128,
        backend_settings,
    }
}

fn linear_gain(gain: f32) -> Result<kira::Decibels, AudioError> {
    if !gain.is_finite() || !(0.0..=1.0).contains(&gain) {
        return Err(AudioError("gain must be finite and within 0..=1".into()));
    }
    Ok(if gain == 0.0 {
        kira::Decibels::SILENCE
    } else {
        kira::Decibels(20.0 * gain.log10())
    })
}

impl OfflineAudio {
    pub fn new_offline(config: AudioConfig, sample_rate: u32) -> Result<Self, AudioError> {
        Self::new_offline_with_gain(config, sample_rate, 1.0)
    }

    /// Initialize the track at a bounded linear gain before its first PCM block.
    pub fn new_offline_with_gain(
        config: AudioConfig,
        sample_rate: u32,
        gain: f32,
    ) -> Result<Self, AudioError> {
        let gain = linear_gain(gain)?;
        let config = config.validate()?;
        let manager = AudioManager::new(settings(config, sample_rate, gain))?;
        Ok(Self::from_manager(manager, config))
    }

    /// Mix into interleaved stereo at the configured output rate. No device IO.
    pub fn render(&mut self, stereo: &mut [f32]) -> Result<(), AudioError> {
        self.manager.backend_mut().render(stereo)?;
        self.reap();
        Ok(())
    }
}

#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
impl NativeAudio {
    /// Success means CPAL opened a stream, not that a human heard sound.
    pub fn new_native(config: AudioConfig) -> Result<Self, AudioError> {
        Self::new_native_with_gain(config, 1.0)
    }

    /// Validate gain before opening output, and initialize it before the first block.
    pub fn new_native_with_gain(config: AudioConfig, gain: f32) -> Result<Self, AudioError> {
        let gain = linear_gain(gain)?;
        let config = config.validate()?;
        let manager = AudioManager::new(settings(config, Default::default(), gain))
            .map_err(|e| AudioError(format!("native audio unavailable: {e}")))?;
        Ok(Self::from_manager(manager, config))
    }

    /// Poll every presentation frame for stream errors forwarded by Kira.
    /// The owner should drop this output and report an error, or deliberately reopen.
    /// This is not a health guarantee: Kira's background device-reopen worker can
    /// panic without forwarding an error (see the pinned backend's upstream limits).
    pub fn device_error(&mut self) -> Option<AudioError> {
        self.manager
            .backend_mut()
            .pop_error()
            .map(|e| AudioError(format!("native audio stream: {e}")))
    }
}

impl<B: Backend> EventAudio<B> {
    fn from_manager(manager: AudioManager<B>, config: AudioConfig) -> Self {
        Self {
            manager,
            config,
            voices: Vec::new(),
            listener: Listener::default(),
            history: BTreeMap::new(),
            retired_through: None,
            source: None,
            paused: false,
            disconnected: false,
            stats: AudioStats::default(),
        }
    }
    pub fn stats(&self) -> AudioStats {
        self.stats
    }
    pub fn voice_count(&self) -> usize {
        self.voices.len()
    }
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Poll completion independently of incoming events. Keep tombstones so late
    /// confirmation cannot replay a one-shot that already finished.
    pub fn reap(&mut self) {
        let before = self.voices.len();
        self.voices
            .retain(|v| v.handle.state() != PlaybackState::Stopped);
        self.stats.completed = self
            .stats
            .completed
            .saturating_add((before - self.voices.len()) as u64);
    }

    /// Presentation suppression stops transients but retains their tombstones.
    pub fn set_paused(&mut self, paused: bool) {
        if paused && !self.paused {
            self.stop_voices();
        }
        self.paused = paused;
    }

    /// Set bounded linear master amplitude. Kira accepts decibels, not amplitude.
    /// Kira interpolates the old/new values over one internal block even with a
    /// zero-duration tween. Use the gain-aware constructor for a first-block bound.
    pub fn set_gain(&mut self, gain: f32) -> Result<(), AudioError> {
        let volume = linear_gain(gain)?;
        self.manager.main_track().set_volume(
            volume,
            Tween {
                duration: Duration::ZERO,
                ..Tween::default()
            },
        );
        Ok(())
    }

    /// Stop all owned voices with the short cancel fade, forget identity history.
    /// The manager keeps fading voices alive and bounded until they finish.
    pub fn reset(&mut self) {
        self.stop_voices();
        self.history.clear();
        self.retired_through = None;
        self.source = None;
        self.paused = false;
        self.disconnected = false;
        self.stats.resets = self.stats.resets.saturating_add(1);
        self.reap();
    }

    /// Consume the bridge's atomic view update in order. Map only audible game
    /// events to clips. Source replacement, seek, branch, disconnect and resync
    /// stop old voices instead of reconstructing missed one-shots. Pause stops
    /// transients but retains tombstones, so resume cannot replay them.
    ///
    /// Lifecycle events, not the newest snapshot epoch, delimit lifetimes: a
    /// normal snapshot can be newer than this batch's event tail.
    pub fn update<E>(
        &mut self,
        source: u64,
        update: &ViewUpdate<E>,
        clip: impl Fn(&E) -> Option<Clip>,
    ) {
        self.reap();
        let mut adopt_baseline = self.source != Some(source);
        if self.source.is_some_and(|old| old != source) {
            self.reset();
        }
        self.source = Some(source);
        if let Some(resync) = &update.resync {
            self.resync(source, resync, adopt_baseline);
            adopt_baseline = false;
        }
        for event in &update.events {
            match event {
                BridgeEvent::Sim { key, status } => self.event(*key, status, &clip),
                BridgeEvent::ViewResynced(resync) => self.resync(source, resync, adopt_baseline),
                BridgeEvent::Lifecycle(note) => match note {
                    Lifecycle::Seeked { to, .. } => self.reset_at(source, *to),
                    Lifecycle::Branched { tick, .. } => self.reset_at(source, *tick),
                    Lifecycle::SessionStarted { .. } => {
                        self.reset();
                        self.source = Some(source);
                    }
                    Lifecycle::Disconnected => {
                        self.disconnected = true;
                        self.reset_at(source, u64::MAX);
                    }
                    Lifecycle::Paused { .. } => {
                        self.set_paused(true);
                    }
                    Lifecycle::Resumed { .. } => self.set_paused(false),
                    _ => {}
                },
            }
            adopt_baseline = false;
        }
    }

    fn resync(&mut self, source: u64, resync: &ViewResync, adopt_baseline: bool) {
        self.reset_at(source, resync.head_tick);
        // `disconnected` is historical sticky diagnostics. Seed an unseen owner
        // only; otherwise ordered incremental recovery determines current state.
        if adopt_baseline {
            self.disconnected = resync.disconnected;
        }
        for recovery in &resync.lifecycle {
            match recovery.last {
                Lifecycle::SessionStarted { .. } => {
                    self.paused = false;
                    self.disconnected = false;
                }
                Lifecycle::Disconnected => self.disconnected = true,
                Lifecycle::Paused { .. } => self.paused = true,
                Lifecycle::Resumed { .. } => self.set_paused(false),
                _ => {}
            }
        }
    }

    fn reset_at(&mut self, source: u64, tick: u64) {
        let paused = self.paused;
        let disconnected = self.disconnected;
        self.reset();
        // Same-source discontinuities do not imply a resume. Recovery summaries
        // are incremental and may omit a pause already observed in an older poll.
        self.paused = paused;
        self.disconnected = disconnected;
        self.source = Some(source);
        self.retired_through = Some(tick);
    }

    fn stop_voices(&mut self) {
        let tween = self.fade();
        for voice in &mut self.voices {
            if voice.key.take().is_some() {
                voice.handle.stop(tween);
            }
        }
    }

    fn fade(&self) -> Tween {
        Tween {
            duration: self.config.cancel_fade,
            ..Tween::default()
        }
    }

    /// Moves the ear of spatial sounds. Playing spatial sounds follow it over
    /// a few milliseconds, so a moving listener does not click.
    pub fn set_listener(&mut self, listener: Listener) {
        if !listener.position.iter().chain(&listener.right).all(|v| v.is_finite()) {
            return;
        }
        self.listener = listener;
        let tween = Tween { duration: Duration::from_millis(20), ..Tween::default() };
        for voice in self.voices.iter_mut().filter(|v| v.key.is_some()) {
            if let Some(emitter) = voice.emitter {
                let (volume, pan) = emitter.heard_by(&listener);
                voice.handle.set_volume(Decibels(volume), tween);
                voice.handle.set_panning(Panning(pan), tween);
            }
        }
    }

    pub fn listener(&self) -> Listener {
        self.listener
    }

    /// Low-level ordered event input, for adapters without a ViewUpdate. Call
    /// reset on any source/epoch change; never feed both this and update for one batch.
    pub fn event<E>(
        &mut self,
        key: EventKey,
        status: &EventStatus<E>,
        clip: impl Fn(&E) -> Option<Clip>,
    ) {
        self.reap();
        if self.disconnected || self.retired_through.is_some_and(|tick| key.tick <= tick) {
            self.stats.dropped = self.stats.dropped.saturating_add(1);
            return;
        }
        let seen = self.history.get(&key).copied();
        match status {
            EventStatus::Canceled => {
                if seen == Some(Seen::Verified) || seen == Some(Seen::Canceled) {
                    self.stats.duplicates = self.stats.duplicates.saturating_add(1);
                    return;
                }
                let tween = self.fade();
                for voice in self.voices.iter_mut().filter(|v| v.key == Some(key)) {
                    voice.handle.stop(tween);
                    voice.key = None;
                }
                self.remember(key, Seen::Canceled);
                self.stats.canceled = self.stats.canceled.saturating_add(1);
            }
            EventStatus::Predicted(payload) | EventStatus::Verified(payload) => {
                let verified = matches!(status, EventStatus::Verified(_));
                if matches!(seen, Some(Seen::Predicted | Seen::Verified)) {
                    if verified && seen == Some(Seen::Predicted) {
                        self.history.insert(key, Seen::Verified);
                        self.stats.confirmed = self.stats.confirmed.saturating_add(1);
                    } else {
                        self.stats.duplicates = self.stats.duplicates.saturating_add(1);
                    }
                    return;
                }
                if verified && seen == Some(Seen::Canceled) {
                    self.stats.duplicates = self.stats.duplicates.saturating_add(1);
                    return;
                }
                // Canceled→Predicted is a replacement occurrence; preserve ordered delivery.
                if !self.remember(
                    key,
                    if verified {
                        Seen::Verified
                    } else {
                        Seen::Predicted
                    },
                ) {
                    return;
                }
                let Some(clip) = clip(payload) else {
                    return;
                };
                if self.paused || self.voices.len() >= self.config.max_voices {
                    self.stats.dropped = self.stats.dropped.saturating_add(1);
                    return;
                }
                let mut data = clip.data;
                if let Some(emitter) = clip.emitter {
                    let (volume, pan) = emitter.heard_by(&self.listener);
                    data = data.volume(Decibels(volume)).panning(Panning(pan));
                }
                match self.manager.play(data) {
                    Ok(handle) => {
                        self.voices.push(Voice {
                            key: Some(key),
                            handle,
                            emitter: clip.emitter,
                        });
                        self.stats.started = self.stats.started.saturating_add(1);
                    }
                    Err(_) => self.stats.dropped = self.stats.dropped.saturating_add(1),
                }
            }
        }
    }

    fn remember(&mut self, key: EventKey, state: Seen) -> bool {
        if !self.history.contains_key(&key) && self.history.len() >= self.config.history_capacity {
            // Retire a whole tick, not an individual key whose late notification
            // would otherwise look new. The monotone floor costs constant memory.
            let oldest = self
                .history
                .first_key_value()
                .expect("nonempty history")
                .0
                .tick;
            let floor = oldest.max(self.retired_through.unwrap_or(0));
            self.retired_through = Some(floor);
            self.history.retain(|key, _| key.tick > floor);
            let tween = self.fade();
            for voice in self
                .voices
                .iter_mut()
                .filter(|v| v.key.is_some_and(|k| k.tick <= floor))
            {
                voice.handle.stop(tween);
                voice.key = None;
            }
            if key.tick <= floor {
                self.stats.dropped = self.stats.dropped.saturating_add(1);
                return false;
            }
        }
        self.history.insert(key, state);
        true
    }
}

impl<B: Backend> Drop for EventAudio<B> {
    fn drop(&mut self) {
        // Do not leave transients running while a native backend shuts down its
        // device thread. Delivery is asynchronous, not a synchronous device fence.
        let stop = Tween {
            duration: Duration::ZERO,
            ..Tween::default()
        };
        for voice in &mut self.voices {
            voice.handle.stop(stop);
        }
    }
}
