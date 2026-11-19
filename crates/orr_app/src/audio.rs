//! Sound for the views: `ViewContext::audio()`.
//!
//! The app owns the audio output. A run opens the system device when the first sound plays.
//! When there is no device (or the crate was built without `audio-native`), the run goes on
//! without sound and says so once on stderr. A `--screenshot` run never opens a device.
//! Nothing here reaches the simulation: sound is presentation only.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use orr_audio::{Clip, Listener, OfflineAudio, SoundId};
#[cfg(feature = "audio-native")]
use orr_audio::AudioConfig;

/// The sound of one `play`, to stop it later (see [`Audio::stop`]).
pub type SoundHandle = SoundId;

/// An audio output that can start and stop one-shot sounds. [`OfflineAudio`] and the system
/// device implement it. [`crate::Views::set_sound_out`] takes any of them.
pub trait SoundOut {
    /// Starts `clip`. `None` when it did not start (paused or too many sounds).
    fn play(&mut self, clip: &Clip) -> Option<SoundHandle>;
    /// Fades the sound out. Does nothing when it already ended.
    fn stop(&mut self, handle: SoundHandle);
    fn is_playing(&self, handle: SoundHandle) -> bool;
    /// Sets the linear volume (0..=1) of a playing sound. Does nothing by default.
    fn set_volume(&mut self, handle: SoundHandle, volume: f32) {
        let _ = (handle, volume);
    }
    /// Sets the linear volume (0..=1) of everything this output plays. Does nothing by default.
    fn set_master_volume(&mut self, volume: f32) {
        let _ = volume;
    }
    /// Where placed clips ([`Clip::at`]) are heard from.
    fn set_listener(&mut self, listener: Listener);
    /// A device error since the last call, as text. After it the app drops this output.
    fn take_error(&mut self) -> Option<String> {
        None
    }
}

impl SoundOut for OfflineAudio {
    fn play(&mut self, clip: &Clip) -> Option<SoundHandle> {
        OfflineAudio::play(self, clip)
    }
    fn stop(&mut self, handle: SoundHandle) {
        OfflineAudio::stop(self, handle);
    }
    fn is_playing(&self, handle: SoundHandle) -> bool {
        OfflineAudio::is_playing(self, handle)
    }
    fn set_volume(&mut self, handle: SoundHandle, volume: f32) {
        OfflineAudio::set_volume(self, handle, volume);
    }
    fn set_master_volume(&mut self, volume: f32) {
        let _ = OfflineAudio::set_gain(self, volume.clamp(0.0, 1.0));
    }
    fn set_listener(&mut self, listener: Listener) {
        OfflineAudio::set_listener(self, listener);
    }
}

#[cfg(feature = "audio-native")]
impl SoundOut for orr_audio::NativeAudio {
    fn play(&mut self, clip: &Clip) -> Option<SoundHandle> {
        orr_audio::NativeAudio::play(self, clip)
    }
    fn stop(&mut self, handle: SoundHandle) {
        orr_audio::NativeAudio::stop(self, handle);
    }
    fn is_playing(&self, handle: SoundHandle) -> bool {
        orr_audio::NativeAudio::is_playing(self, handle)
    }
    fn set_volume(&mut self, handle: SoundHandle, volume: f32) {
        orr_audio::NativeAudio::set_volume(self, handle, volume);
    }
    fn set_master_volume(&mut self, volume: f32) {
        let _ = orr_audio::NativeAudio::set_gain(self, volume.clamp(0.0, 1.0));
    }
    fn set_listener(&mut self, listener: Listener) {
        orr_audio::NativeAudio::set_listener(self, listener);
    }
    fn take_error(&mut self) -> Option<String> {
        self.device_error().map(|e| e.to_string())
    }
}

/// An output that the game keeps a handle to, for example to read what an offline output rendered.
impl<S: SoundOut> SoundOut for Rc<RefCell<S>> {
    fn play(&mut self, clip: &Clip) -> Option<SoundHandle> {
        self.borrow_mut().play(clip)
    }
    fn stop(&mut self, handle: SoundHandle) {
        self.borrow_mut().stop(handle);
    }
    fn is_playing(&self, handle: SoundHandle) -> bool {
        self.borrow().is_playing(handle)
    }
    fn set_volume(&mut self, handle: SoundHandle, volume: f32) {
        self.borrow_mut().set_volume(handle, volume);
    }
    fn set_master_volume(&mut self, volume: f32) {
        self.borrow_mut().set_master_volume(volume);
    }
    fn set_listener(&mut self, listener: Listener) {
        self.borrow_mut().set_listener(listener);
    }
    fn take_error(&mut self) -> Option<String> {
        self.borrow_mut().take_error()
    }
}

enum Port {
    /// No output: every `play` does nothing.
    Silent,
    /// The system device, opened by the first `play`.
    Device,
    Out(Box<dyn SoundOut>),
}

/// What a view can do with sound. Valid for one call, like the rest of the `ViewContext`.
///
/// Play on `EventState::Predicted`, keep the handle, and stop it on `EventState::Canceled`:
/// a rollback took the event back, so its sound must not go on.
pub struct Audio {
    port: Port,
    clips: BTreeMap<String, Clip>,
    /// The listener position that 2D and 3D runs hear placed clips from.
    listener: Listener,
    /// The master volume (0..=1): applied to an output when it opens and when it changes.
    master: f32,
}

impl Default for Audio {
    fn default() -> Self {
        Self { port: Port::Silent, clips: BTreeMap::new(), listener: Listener::default(), master: 1.0 }
    }
}

impl Audio {
    /// Makes the run open the system device when the first sound plays.
    pub(crate) fn use_device(&mut self) {
        self.port = Port::Device;
    }

    /// No output at all (a screenshot run).
    pub(crate) fn mute(&mut self) {
        self.port = Port::Silent;
    }

    pub(crate) fn set_out(&mut self, mut out: Box<dyn SoundOut>) {
        if self.master < 1.0 {
            out.set_master_volume(self.master);
        }
        self.port = Port::Out(out);
    }

    pub(crate) fn add(&mut self, name: &str, clip: Clip) {
        self.clips.insert(name.to_string(), clip);
    }

    /// The output, opened now when it is the device that was not opened yet. `None` is silence.
    fn out(&mut self) -> Option<&mut dyn SoundOut> {
        if matches!(self.port, Port::Device) {
            self.port = match open_device() {
                Ok(out) => Port::Out(out),
                Err(reason) => {
                    eprintln!("no sound: {reason}");
                    Port::Silent
                }
            };
            if let Port::Out(out) = &mut self.port {
                out.set_listener(self.listener);
                if self.master < 1.0 {
                    out.set_master_volume(self.master);
                }
            }
        }
        match &mut self.port {
            Port::Out(out) => Some(out.as_mut()),
            _ => None,
        }
    }

    /// Looks at the device once per frame: after an error the output is dropped, said once.
    pub(crate) fn poll(&mut self) {
        if let Port::Out(out) = &mut self.port {
            if let Some(error) = out.take_error() {
                eprintln!("no sound from now on: {error}");
                self.port = Port::Silent;
            }
        }
    }

    /// Plays `clip` once. The handle is for [`Audio::stop`]. `None` when nothing was played: there is no
    /// audio device, or too many sounds play already.
    pub fn play(&mut self, clip: &Clip) -> Option<SoundHandle> {
        self.out()?.play(clip)
    }

    /// Plays the clip registered with [`crate::App::sound`] under `name`. `None` when there is no
    /// such clip (nothing is said) or nothing was played.
    pub fn play_named(&mut self, name: &str) -> Option<SoundHandle> {
        let clip = self.clips.get(name)?.clone();
        self.play(&clip)
    }

    /// Plays `clip` from the world position `position`. It is loud within `near` units of the
    /// listener and silent from `far` units on, and it pans toward its side
    /// (listener: [`Audio::set_listener`], by default at the origin with its right side toward `+x`).
    pub fn play_at(&mut self, clip: &Clip, position: [f32; 3], near: f32, far: f32) -> Option<SoundHandle> {
        self.play(&clip.clone().at(position, near, far))
    }

    /// Plays `clip` again and again until [`Audio::stop`] (music, an engine hum).
    pub fn play_looped(&mut self, clip: &Clip) -> Option<SoundHandle> {
        self.play(&clip.clone().looped())
    }

    /// Plays the clip registered under `name` at the linear `volume` (0..=1, 1 as recorded), in a
    /// loop when `looped`. `None` when there is no such clip or nothing was played.
    pub fn play_named_with(&mut self, name: &str, volume: f32, looped: bool) -> Option<SoundHandle> {
        let mut clip = self.clips.get(name)?.clone().volume(volume);
        if looped {
            clip = clip.looped();
        }
        self.play(&clip)
    }

    /// Sets the linear volume (0..=1) of a playing sound, over 20 ms. Does nothing when it ended.
    pub fn set_volume(&mut self, handle: SoundHandle, volume: f32) {
        if let Some(out) = self.out() {
            out.set_volume(handle, volume);
        }
    }

    /// Sets the linear volume (0..=1, default 1) of everything the run plays. It also applies to
    /// the output that opens later. A value outside 0..=1 is clamped.
    pub fn set_master_volume(&mut self, volume: f32) {
        self.master = if volume.is_finite() { volume.clamp(0.0, 1.0) } else { 0.0 };
        if let Port::Out(out) = &mut self.port {
            out.set_master_volume(self.master);
        }
    }

    /// The master volume that [`Audio::set_master_volume`] set.
    pub fn master_volume(&self) -> f32 {
        self.master
    }

    /// Fades the sound out. Use it when a rollback canceled the event that started it.
    pub fn stop(&mut self, handle: SoundHandle) {
        if let Some(out) = self.out() {
            out.stop(handle);
        }
    }

    pub fn is_playing(&mut self, handle: SoundHandle) -> bool {
        self.out().is_some_and(|out| out.is_playing(handle))
    }

    /// Moves the ear that placed clips are heard from.
    pub fn set_listener(&mut self, listener: Listener) {
        self.listener = listener;
        if let Port::Out(out) = &mut self.port {
            out.set_listener(listener);
        }
    }

    /// Whether a clip is registered under `name`.
    pub fn has_sound(&self, name: &str) -> bool {
        self.clips.contains_key(name)
    }
}

#[cfg(feature = "audio-native")]
fn open_device() -> Result<Box<dyn SoundOut>, String> {
    orr_audio::NativeAudio::new_native(AudioConfig::default()).map(|out| Box::new(out) as Box<dyn SoundOut>).map_err(|e| e.to_string())
}

#[cfg(not(feature = "audio-native"))]
fn open_device() -> Result<Box<dyn SoundOut>, String> {
    Err("built without the audio-native feature".to_string())
}
