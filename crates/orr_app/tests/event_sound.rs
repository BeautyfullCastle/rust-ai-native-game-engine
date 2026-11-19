//! Event sounds (`Views::on_event_sound`), the audio controls of `ctx.audio()` and WAV clips, against the
//! real mixer. The rollback test runs two prediction sessions on a delayed loopback link, as
//! `tests/views.rs` does, so the bridge itself delivers `Predicted`, `Verified` and `Canceled`.
#![allow(clippy::float_arithmetic)]

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::time::Duration;

use bytemuck::{Pod, Zeroable};
use orr_app::{clip_from_wav, AudioConfig, Clip, EventState, Listener, OfflineAudio, SceneView, SoundHandle, SoundOut, ViewContext, Views};
use orr_bridge::{
    Bridge, BridgeConfig, BridgeEvent, EventKey, EventStatus, InProc, LoopbackPair, PlayConfig, PlayHost, PlaySession, PlayerSlot, ViewResync,
};
use orr_session::SessionConfig;
use orr_sim::{ComponentRegistryBuilder, Frame, Game, SimContext, System};

const ONE_TICK: Duration = Duration::from_nanos(16_666_667);
const RATE: u32 = 44_100;

// ---- a game in which holding the trigger spawns a spark and emits a flash ----

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, Pod, Zeroable)]
struct Trigger {
    held: u32,
    _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
struct Spark {
    owner: u32,
    born: u32,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
struct Flash {
    owner: u32,
    tick: u32,
}

struct Burst;

impl Game for Burst {
    type Input = Trigger;
    type Command = ();
    type Event = Flash;
    type Config = ();

    fn register(builder: &mut ComponentRegistryBuilder) {
        builder.register_component::<Spark>("Spark");
    }

    fn setup(_frame: &mut Frame, _config: &()) {}

    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(Sparks)]
    }
}

struct Sparks;

impl System<Burst> for Sparks {
    fn name(&self) -> &'static str {
        "Sparks"
    }

    fn run(&mut self, ctx: &mut SimContext<Burst>) {
        for owner in 0..2u32 {
            if ctx.inputs.input(PlayerSlot(owner as u8)).held != 0 {
                let spark = ctx.frame.spawn();
                ctx.frame.add(spark, Spark { owner, born: ctx.tick as u32 });
                ctx.emit(Flash { owner, tick: ctx.tick as u32 });
            }
        }
    }
}

// ---- an output that remembers what the app asked of it, and plays it for real ----

#[derive(Default)]
struct Recorder {
    plays: Vec<SoundHandle>,
    stops: Vec<SoundHandle>,
    mixer: Option<OfflineAudio>,
}

impl Recorder {
    fn new() -> Rc<RefCell<Recorder>> {
        Rc::new(RefCell::new(Recorder { mixer: Some(OfflineAudio::new_offline(AudioConfig::default(), RATE).unwrap()), ..Default::default() }))
    }

    fn mixer(&mut self) -> &mut OfflineAudio {
        self.mixer.as_mut().unwrap()
    }
}

impl SoundOut for Recorder {
    fn play(&mut self, clip: &Clip) -> Option<SoundHandle> {
        let handle = self.mixer().play(clip)?;
        self.plays.push(handle);
        Some(handle)
    }
    fn stop(&mut self, handle: SoundHandle) {
        self.stops.push(handle);
        self.mixer().stop(handle);
    }
    fn is_playing(&self, handle: SoundHandle) -> bool {
        self.mixer.as_ref().unwrap().is_playing(handle)
    }
    fn set_volume(&mut self, handle: SoundHandle, volume: f32) {
        self.mixer().set_volume(handle, volume);
    }
    fn set_master_volume(&mut self, volume: f32) {
        self.mixer().set_gain(volume).unwrap();
    }
    fn set_listener(&mut self, listener: Listener) {
        self.mixer().set_listener(listener);
    }
}

/// A long sound (9 seconds), so only a stop can end it inside a test.
fn long_clip() -> Clip {
    Clip::from_stereo(22_050, vec![[0.3, 0.3]; 22_050 * 9]).unwrap()
}

fn render(recorder: &Rc<RefCell<Recorder>>, frames: usize) -> Vec<f32> {
    let mut samples = vec![0.0; frames * 2];
    for block in samples.chunks_mut(256) {
        recorder.borrow_mut().mixer().render(block).unwrap();
    }
    samples
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |peak, v| peak.max(v.abs()))
}

fn views_with_sound(recorder: &Rc<RefCell<Recorder>>) -> Views<Burst> {
    let mut views = Views::<Burst>::new();
    views.set_sound_out(recorder.clone());
    views.sound("flash", long_clip());
    views.on_event_sound(|_flash: &Flash| Some("flash"));
    views
}

// ---- real rollbacks ----

#[derive(Default)]
struct Log {
    flashes: RefCell<Vec<(Flash, EventState)>>,
}

struct Watch {
    log: Rc<Log>,
}

impl SceneView<Burst> for Watch {
    fn on_event(&mut self, _ctx: &mut ViewContext<'_, Burst>, event: &Flash, state: EventState) {
        self.log.flashes.borrow_mut().push((*event, state));
    }
}

/// Two prediction sessions on a delayed link. Player 1 (remote) holds the trigger for ticks 20..30. The
/// local session repeats the last input it knows of player 1, so it guesses wrong at the end of the hold.
fn rolling_back_bridge() -> InProc<Burst, LoopbackPair<Burst>> {
    let pair = LoopbackPair::new(
        || (),
        SessionConfig::new(2, PlayerSlot(0), 42, 60),
        SessionConfig::new(2, PlayerSlot(1), 42, 60),
        4,
        0,
        777,
        |tick| (Trigger { held: u32::from((20..30).contains(&tick)), _pad: 0 }, Vec::new()),
    );
    InProc::new(pair, BridgeConfig::default())
}

#[test]
fn across_real_rollbacks_every_event_sounds_once_and_every_canceled_one_stops() {
    let recorder = Recorder::new();
    let (mut views, log) = (views_with_sound(&recorder), Rc::new(Log::default()));
    views.scene_view(Watch { log: log.clone() });
    let mut bridge = rolling_back_bridge();
    for _ in 0..120 {
        bridge.update(ONE_TICK);
        let update = bridge.poll_view();
        views.dispatch(&mut bridge, &update, &[]);
        render(&recorder, 735); // one tick of sound time
    }
    let snapshot = bridge.snapshot().unwrap();
    assert!(snapshot.stats().rollbacks >= 2, "the guess was wrong twice (start and end of the hold)");

    let mut states: BTreeMap<(u32, u32), Vec<EventState>> = BTreeMap::new();
    for (flash, state) in log.flashes.borrow().iter() {
        states.entry((flash.owner, flash.tick)).or_default().push(*state);
    }
    let canceled: BTreeSet<(u32, u32)> = states.iter().filter(|(_, seen)| seen.contains(&EventState::Canceled)).map(|(key, _)| *key).collect();
    let surviving = states.len() - canceled.len();
    assert!(!canceled.is_empty(), "the prediction flashed for a trigger nobody held");
    assert_eq!(surviving, 10, "the ten real flashes survive");

    let recorder = recorder.borrow();
    assert_eq!(recorder.plays.len(), states.len(), "one sound for every event, real or guessed, and no second one for a kept event");
    assert_eq!(recorder.stops.len(), canceled.len(), "a stop for every canceled event and for nothing else");
    let stopped: BTreeSet<SoundHandle> = recorder.stops.iter().copied().collect();
    assert_eq!(stopped.len(), recorder.stops.len(), "no sound is stopped twice");
    assert!(stopped.iter().all(|handle| recorder.plays.contains(handle)));
    for handle in &recorder.plays {
        // The clip lasts 9 seconds and the test 2: only a stop can have ended a sound.
        assert_eq!(recorder.mixer.as_ref().unwrap().is_playing(*handle), !stopped.contains(handle), "{handle:?}");
    }
    assert_eq!(recorder.mixer.as_ref().unwrap().voice_count(), surviving);
}

// ---- scripted updates: duplicates, replacement, resync ----

fn offline_bridge() -> InProc<Burst, PlayHost<Burst>> {
    let session = PlaySession::<Burst>::new(PlayConfig::new(2, 42, 60), ());
    let mut bridge = InProc::new(PlayHost::new(session, PlayerSlot(0)), BridgeConfig::default());
    bridge.step(1);
    bridge
}

fn flash(owner: u32) -> Flash {
    Flash { owner, tick: 5 }
}

fn key(seq: u32) -> EventKey {
    EventKey::new(5, 0, seq)
}

fn send(views: &mut Views<Burst>, bridge: &mut impl Bridge<Burst>, events: Vec<(EventKey, EventStatus<Flash>)>) {
    let mut update = bridge.poll_view();
    update.events = events.into_iter().map(|(key, status)| BridgeEvent::Sim { key, status }).collect();
    views.dispatch(bridge, &update, &[]);
}

#[test]
fn an_event_key_starts_one_sound_whatever_the_order_and_repeats_of_its_notices() {
    let recorder = Recorder::new();
    let mut views = views_with_sound(&recorder);
    let mut bridge = offline_bridge();
    let counts = |recorder: &Rc<RefCell<Recorder>>| (recorder.borrow().plays.len(), recorder.borrow().stops.len());

    // Predicted, then Verified: one sound, which goes on.
    send(&mut views, &mut bridge, vec![(key(0), EventStatus::Predicted(flash(0)))]);
    send(&mut views, &mut bridge, vec![(key(0), EventStatus::Verified(flash(0)))]);
    assert_eq!(counts(&recorder), (1, 0));
    // A repeated Predicted or Verified, even after a Canceled that arrives late, changes nothing.
    send(&mut views, &mut bridge, vec![(key(0), EventStatus::Predicted(flash(0))), (key(0), EventStatus::Verified(flash(0))), (key(0), EventStatus::Canceled)]);
    assert_eq!(counts(&recorder), (1, 0));

    // Offline, an event is Verified at once: one sound, and a repeat does not start another.
    send(&mut views, &mut bridge, vec![(key(1), EventStatus::Verified(flash(1))), (key(1), EventStatus::Verified(flash(1)))]);
    assert_eq!(counts(&recorder), (2, 0));

    // Predicted twice: one sound. Canceled: one stop, and a second Canceled stops nothing.
    send(&mut views, &mut bridge, vec![(key(2), EventStatus::Predicted(flash(2))), (key(2), EventStatus::Predicted(flash(2)))]);
    assert_eq!(counts(&recorder), (3, 0));
    send(&mut views, &mut bridge, vec![(key(2), EventStatus::Canceled), (key(2), EventStatus::Canceled)]);
    assert_eq!(counts(&recorder), (3, 1));
    // A Verified that comes after the Cancel is stale: no sound. A new Predicted with the key is a replacement and sounds.
    send(&mut views, &mut bridge, vec![(key(2), EventStatus::Verified(flash(2)))]);
    assert_eq!(counts(&recorder), (3, 1));
    send(&mut views, &mut bridge, vec![(key(2), EventStatus::Predicted(flash(2)))]);
    assert_eq!(counts(&recorder), (4, 1));
    send(&mut views, &mut bridge, vec![(key(2), EventStatus::Verified(flash(2)))]);
    assert_eq!(counts(&recorder), (4, 1), "the replacement is confirmed and keeps its sound");

    // Canceled for an event whose sound was never started (never announced): nothing to stop.
    send(&mut views, &mut bridge, vec![(key(9), EventStatus::Canceled)]);
    assert_eq!(counts(&recorder), (4, 1));
    render(&recorder, 4410);
    assert_eq!(recorder.borrow().mixer.as_ref().unwrap().voice_count(), 3, "sounds of 0, 1 and the replacement of 2 play; the canceled one faded");
}

#[test]
fn a_resync_stops_the_unconfirmed_sounds_and_keeps_the_confirmed_ones() {
    let recorder = Recorder::new();
    let mut views = views_with_sound(&recorder);
    let mut bridge = offline_bridge();
    send(&mut views, &mut bridge, vec![(key(0), EventStatus::Verified(flash(0))), (key(1), EventStatus::Predicted(flash(1)))]);
    assert_eq!((recorder.borrow().plays.len(), recorder.borrow().stops.len()), (2, 0));

    let mut update = bridge.poll_view();
    update.resync = Some(ViewResync { generation: 1, discarded_events: 1, head_tick: 1, verified_tick: 1, disconnected: false, last_desync: None, lifecycle: Vec::new() });
    views.dispatch(&mut bridge, &update, &[]);
    assert_eq!(recorder.borrow().stops.len(), 1, "only the guess stops: no Canceled will come for it");
    render(&recorder, 4410);
    assert_eq!(recorder.borrow().mixer.as_ref().unwrap().voice_count(), 1, "the verified sound goes on");

    // The Canceled of a forgotten guess finds nothing to stop.
    send(&mut views, &mut bridge, vec![(key(7), EventStatus::Canceled)]);
    assert_eq!((recorder.borrow().plays.len(), recorder.borrow().stops.len()), (2, 1));
    // A Verified with the key of the stopped guess starts a fresh, whole sound: its first one was cut.
    send(&mut views, &mut bridge, vec![(key(1), EventStatus::Verified(flash(1)))]);
    assert_eq!((recorder.borrow().plays.len(), recorder.borrow().stops.len()), (3, 1));
}

#[test]
fn a_replaced_session_stops_every_sound_and_forgets_the_keys() {
    let recorder = Recorder::new();
    let mut views = views_with_sound(&recorder);
    let mut bridge = offline_bridge();
    send(&mut views, &mut bridge, vec![(key(0), EventStatus::Verified(flash(0))), (key(1), EventStatus::Predicted(flash(1)))]);
    views.session_replaced();
    let mut bridge = offline_bridge();
    send(&mut views, &mut bridge, vec![]);
    render(&recorder, 4410);
    assert_eq!(recorder.borrow().mixer.as_ref().unwrap().voice_count(), 1, "the unconfirmed sound stopped; a verified one is not stopped by a new session");
    send(&mut views, &mut bridge, vec![(key(0), EventStatus::Verified(flash(0)))]);
    assert_eq!(recorder.borrow().plays.len(), 3, "the key of the old session sounds again in the new one");
}

#[test]
fn a_binding_that_names_no_sound_or_an_unknown_one_plays_nothing_and_a_run_without_output_is_silent() {
    let recorder = Recorder::new();
    let mut views = Views::<Burst>::new();
    views.set_sound_out(recorder.clone());
    views.sound("flash", long_clip());
    views.on_event_sound(|flash: &Flash| (flash.owner == 1).then_some("flash"));
    views.on_event_sound(|_flash: &Flash| Some("not-registered"));
    let mut bridge = offline_bridge();
    send(&mut views, &mut bridge, vec![(key(0), EventStatus::Verified(flash(0))), (key(1), EventStatus::Verified(flash(1)))]);
    assert_eq!(recorder.borrow().plays.len(), 1, "owner 1 only; the unknown name is skipped");

    let mut silent = Views::<Burst>::new();
    silent.sound("flash", long_clip());
    silent.on_event_sound(|_flash: &Flash| Some("flash"));
    send(&mut silent, &mut bridge, vec![(key(0), EventStatus::Predicted(flash(0))), (key(0), EventStatus::Canceled)]);
}

// ---- the controls of ctx.audio() ----

struct Probe {
    run: Box<dyn FnMut(&mut ViewContext<'_, Burst>)>,
}

impl SceneView<Burst> for Probe {
    fn on_update(&mut self, ctx: &mut ViewContext<'_, Burst>) {
        (self.run)(ctx);
    }
}

fn flat_clip(frames: usize) -> Clip {
    Clip::from_stereo(RATE, vec![[0.5, 0.5]; frames]).unwrap()
}

#[test]
fn the_audio_of_a_view_plays_loops_changes_volume_and_has_a_master_volume() {
    let recorder = Recorder::new();
    let mut views = Views::<Burst>::new();
    views.set_sound_out(recorder.clone());
    views.sound("hum", flat_clip(2205));
    let handles = Rc::new(RefCell::new(Vec::new()));
    let for_probe = handles.clone();
    views.scene_view(Probe {
        run: Box::new(move |ctx| {
            if !for_probe.borrow().is_empty() {
                return;
            }
            assert!(ctx.audio().has_sound("hum") && !ctx.audio().has_sound("nope"));
            let looped = ctx.audio().play_named_with("hum", 1.0, true).unwrap();
            let quiet = ctx.audio().play_named_with("hum", 0.2, false).unwrap();
            let plain = ctx.audio().play_named("hum").unwrap();
            for_probe.borrow_mut().extend([looped, quiet, plain]);
        }),
    });
    let mut bridge = offline_bridge();
    send(&mut views, &mut bridge, vec![]);
    let [looped, quiet, plain] = <[SoundHandle; 3]>::try_from(handles.borrow().clone()).unwrap();
    render(&recorder, 22_050); // half a second: the 50 ms clips ended, the loop did not
    assert!(!recorder.borrow().is_playing(plain) && !recorder.borrow().is_playing(quiet));
    assert!(recorder.borrow().is_playing(looped));

    // The loop is the only voice; its volume follows set_volume, and the master volume scales it.
    let level = |recorder: &Rc<RefCell<Recorder>>| {
        let samples = render(recorder, 4410);
        peak(&samples[samples.len() - 2000..])
    };
    assert!((level(&recorder) - 0.5).abs() < 0.01);
    let mut probe_views = views;
    probe_views.scene_view(Probe {
        run: Box::new(move |ctx| {
            ctx.audio().set_volume(looped, 0.5);
            ctx.audio().set_master_volume(0.5);
            assert_eq!(ctx.audio().master_volume(), 0.5);
        }),
    });
    send(&mut probe_views, &mut bridge, vec![]);
    assert!((level(&recorder) - 0.125).abs() < 0.01, "0.5 x 0.5 x 0.5");
}

// ---- WAV files as clips ----

fn wav(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
    fmt.extend_from_slice(&(channels * 2).to_le_bytes());
    fmt.extend_from_slice(&16u16.to_le_bytes());
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&((4 + 8 + fmt.len() + 8 + data.len()) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    out.extend_from_slice(&fmt);
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

#[test]
fn a_wav_clip_keeps_its_length_in_seconds_on_an_output_of_another_rate() {
    // One second of a constant 0.5 level at 22050 Hz, mono, played on a 44100 Hz mixer.
    let clip = clip_from_wav(&wav(22_050, 1, &vec![16_384i16; 22_050])).unwrap();
    let mut mixer = OfflineAudio::new_offline(AudioConfig::default(), RATE).unwrap();
    let id = mixer.play(&clip).unwrap();
    fn render(mixer: &mut OfflineAudio, frames: usize) -> Vec<f32> {
        let mut samples = vec![0.0; frames * 2];
        for block in samples.chunks_mut(256) {
            mixer.render(block).unwrap();
        }
        samples
    }
    let first = render(&mut mixer, 41_000); // 0.93 s
    assert!((peak(&first[first.len() - 2000..]) - 0.5).abs() < 0.02, "16384 / 32768 is 0.5, on both sides");
    assert!(mixer.is_playing(id), "still playing at 0.93 s");
    let rest = render(&mut mixer, 8_000); // to 1.11 s
    assert!(peak(&rest[rest.len() - 2000..]) < 0.001 && !mixer.is_playing(id), "over at 1.11 s: resampled, not sped up");
}

#[test]
fn a_stereo_wav_keeps_its_two_sides() {
    // Left 0.5, right -0.25.
    let samples: Vec<i16> = (0..4410).flat_map(|_| [16_384i16, -8_192i16]).collect();
    let clip = clip_from_wav(&wav(RATE, 2, &samples)).unwrap();
    let mut mixer = OfflineAudio::new_offline(AudioConfig::default(), RATE).unwrap();
    mixer.play(&clip).unwrap();
    let mut out = vec![0.0; 2 * 2000];
    for block in out.chunks_mut(256) {
        mixer.render(block).unwrap();
    }
    let (left, right) = (out[2 * 1500], out[2 * 1500 + 1]);
    assert!((left - 0.5).abs() < 0.01 && (right + 0.25).abs() < 0.01, "left {left} right {right}");
}
