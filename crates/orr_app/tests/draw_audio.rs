//! What the views draw (`ctx.draw()`) and play (`ctx.audio()`), against real bridges and an
//! offline audio output. No window, no renderer, no audio device.
// View layer test: sound samples are f32.
#![allow(clippy::float_arithmetic)]
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use bytemuck::{Pod, Zeroable};
use orr_app::{AudioConfig, Clip, EventState, OfflineAudio, SceneView, SoundHandle, ViewContext, Views};
use orr_bridge::{Bridge, BridgeConfig, InProc, LoopbackPair, PlayConfig, PlayHost, PlaySession, PlayerSlot};
use orr_session::SessionConfig;
use orr_sim::{ComponentRegistryBuilder, Frame, Game, SimContext, System};

const ONE_TICK: Duration = Duration::from_nanos(16_666_667);
const RATE: u32 = 48_000;
const WHITE: [f32; 4] = [1.0; 4];

// ---- a small game: holding the trigger emits a flash every tick ----

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, Pod, Zeroable)]
struct Trigger {
    held: u32,
    _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
struct Flash {
    owner: u32,
    tick: u32,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
struct Rules {
    players: u32,
    _pad: u32,
}

struct Flasher;

impl Game for Flasher {
    type Input = Trigger;
    type Command = ();
    type Event = Flash;
    type Config = u32;

    fn register(builder: &mut ComponentRegistryBuilder) {
        builder.register_singleton::<Rules>("Rules");
    }

    fn setup(frame: &mut Frame, players: &u32) {
        frame.set_singleton(Rules { players: *players, _pad: 0 });
    }

    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(Emit)]
    }
}

struct Emit;

impl System<Flasher> for Emit {
    fn name(&self) -> &'static str {
        "Emit"
    }

    fn run(&mut self, ctx: &mut SimContext<Flasher>) {
        let players = ctx.frame.singleton::<Rules>().players;
        for owner in 0..players {
            if ctx.inputs.input(PlayerSlot(owner as u8)).held != 0 {
                ctx.emit(Flash { owner, tick: ctx.tick as u32 });
            }
        }
    }
}

fn offline_bridge() -> InProc<Flasher, PlayHost<Flasher>> {
    let session = PlaySession::<Flasher>::new(PlayConfig::new(1, 42, 60), 1);
    InProc::new(PlayHost::new(session, PlayerSlot(0)), BridgeConfig::default())
}

/// Two prediction sessions on a delayed link. Player 1 holds the trigger for ticks 20..30 and the
/// local session guesses wrong at both ends of the hold, so it flashes for ticks nobody held.
fn rolling_back_bridge() -> InProc<Flasher, LoopbackPair<Flasher>> {
    let pair = LoopbackPair::new(
        || 2,
        SessionConfig::new(2, PlayerSlot(0), 42, 60),
        SessionConfig::new(2, PlayerSlot(1), 42, 60),
        4,
        0,
        777,
        |tick| (Trigger { held: u32::from((20..30).contains(&tick)), _pad: 0 }, Vec::new()),
    );
    InProc::new(pair, BridgeConfig::default())
}

fn frame<B: Bridge<Flasher>>(bridge: &mut B, views: &mut Views<Flasher>) {
    bridge.update(ONE_TICK);
    let update = bridge.poll_view();
    views.dispatch(bridge, &update, &[]);
}

// ---- sound ----

/// A 0.1 second blip of 440 Hz.
fn blip() -> Clip {
    tone(4800)
}

/// 440 Hz for `length` frames.
fn tone(length: usize) -> Clip {
    let frames = (0..length)
        .map(|i| {
            let v = 0.3 * (i as f32 * std::f32::consts::TAU * 440.0 / RATE as f32).sin();
            [v, v]
        })
        .collect();
    Clip::from_stereo(RATE, frames).unwrap()
}

type Output = Rc<RefCell<OfflineAudio>>;

fn output() -> Output {
    Rc::new(RefCell::new(OfflineAudio::new_offline(AudioConfig::default(), RATE).unwrap()))
}

/// Mixes `frames` stereo frames and returns the largest sample.
fn loudest(output: &Output, frames: usize) -> f32 {
    let mut samples = vec![0.0f32; frames * 2];
    for block in samples.chunks_mut(256) {
        output.borrow_mut().render(block).unwrap();
    }
    samples.iter().fold(0.0, |peak, v| peak.max(v.abs()))
}

#[derive(Default)]
struct Notes {
    /// For each Canceled flash: whether its sound still played right after `stop`.
    playing_after_cancel: Vec<bool>,
    started: u32,
    canceled: u32,
}

/// Plays a blip for every predicted (or verified) flash and stops it when the flash is canceled.
struct Beeper {
    clip: Clip,
    handles: Vec<(Flash, Option<SoundHandle>)>,
    notes: Rc<RefCell<Notes>>,
    /// Stops every sound in the frame whose tick is this one.
    stop_all_at: Option<u64>,
}

impl SceneView<Flasher> for Beeper {
    fn on_event(&mut self, ctx: &mut ViewContext<'_, Flasher>, event: &Flash, state: EventState) {
        match state {
            EventState::Predicted | EventState::Verified => {
                if !self.handles.iter().any(|(flash, _)| flash == event) {
                    let handle = ctx.audio().play(&self.clip);
                    self.notes.borrow_mut().started += u32::from(handle.is_some());
                    self.handles.push((*event, handle));
                }
            }
            EventState::Canceled => {
                if let Some(at) = self.handles.iter().position(|(flash, _)| flash == event) {
                    let (_, handle) = self.handles.remove(at);
                    if let Some(handle) = handle {
                        ctx.audio().stop(handle);
                        let playing = ctx.audio().is_playing(handle);
                        let mut notes = self.notes.borrow_mut();
                        notes.canceled += 1;
                        notes.playing_after_cancel.push(playing);
                    }
                }
            }
        }
    }

    fn on_update(&mut self, ctx: &mut ViewContext<'_, Flasher>) {
        if self.stop_all_at == Some(ctx.tick()) {
            for (_, handle) in self.handles.drain(..) {
                if let Some(handle) = handle {
                    ctx.audio().stop(handle);
                }
            }
        }
    }
}

#[test]
fn a_played_sound_is_heard_and_after_stop_it_is_silent() {
    let output = output();
    let notes = Rc::new(RefCell::new(Notes::default()));
    let mut views = Views::<Flasher>::new();
    views.set_sound_out(output.clone());
    // One second long: only the stop can make the output silent within the test.
    views.scene_view(Beeper { clip: tone(48_000), handles: Vec::new(), notes: notes.clone(), stop_all_at: Some(4) });
    let mut bridge = offline_bridge();

    // One flash in the first frames, then nothing.
    bridge.set_input(PlayerSlot(0), Trigger { held: 1, _pad: 0 }).unwrap();
    frame(&mut bridge, &mut views);
    bridge.set_input(PlayerSlot(0), Trigger { held: 0, _pad: 0 }).unwrap();
    assert!(notes.borrow().started >= 1, "the event started a sound");
    assert!(loudest(&output, 800) > 0.05, "the sound is heard");

    // The view stops it in the frame of tick 4; a stop is a short fade, then the output is silent.
    for _ in 0..8 {
        frame(&mut bridge, &mut views);
    }
    loudest(&output, 1600);
    assert_eq!(loudest(&output, 4800), 0.0, "after stop the output stays silent");
    assert_eq!(output.borrow().voice_count(), 0);
}

#[test]
fn a_rollback_stops_the_sounds_of_the_events_it_canceled() {
    let output = output();
    let notes = Rc::new(RefCell::new(Notes::default()));
    let mut views = Views::<Flasher>::new();
    views.set_sound_out(output.clone());
    views.scene_view(Beeper { clip: blip(), handles: Vec::new(), notes: notes.clone(), stop_all_at: None });
    let mut bridge = rolling_back_bridge();
    for _ in 0..120 {
        frame(&mut bridge, &mut views);
        loudest(&output, 800);
    }
    let notes = notes.borrow();
    assert!(notes.started >= 10, "the real flashes played");
    assert!(notes.canceled > 0, "the prediction flashed for a trigger nobody held");
    assert!(notes.playing_after_cancel.iter().all(|playing| !playing), "a canceled event's sound is stopped at once");
    // The sounds that were not canceled end by themselves.
    loudest(&output, 12_000);
    assert_eq!(output.borrow().voice_count(), 0);
}

#[test]
fn without_an_output_the_audio_calls_do_nothing() {
    let mut views = Views::<Flasher>::new();
    let notes = Rc::new(RefCell::new(Notes::default()));
    views.scene_view(Beeper { clip: blip(), handles: Vec::new(), notes: notes.clone(), stop_all_at: None });
    views.sound("blip", blip());
    let mut bridge = offline_bridge();
    bridge.set_input(PlayerSlot(0), Trigger { held: 1, _pad: 0 }).unwrap();
    for _ in 0..5 {
        frame(&mut bridge, &mut views);
    }
    assert_eq!(notes.borrow().started, 0, "no output: nothing plays and nothing breaks");
}

#[test]
fn a_named_sound_plays_the_clip_registered_under_that_name() {
    struct Named(Rc<RefCell<Vec<(bool, bool)>>>);
    impl SceneView<Flasher> for Named {
        fn on_update(&mut self, ctx: &mut ViewContext<'_, Flasher>) {
            let known = ctx.audio().play_named("blip").is_some();
            let unknown = ctx.audio().play_named("nothing").is_some();
            self.0.borrow_mut().push((known, unknown));
        }
    }
    let output = output();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut views = Views::<Flasher>::new();
    views.set_sound_out(output.clone());
    views.sound("blip", blip());
    views.scene_view(Named(seen.clone()));
    let mut bridge = offline_bridge();
    frame(&mut bridge, &mut views);
    assert_eq!(seen.borrow().as_slice(), [(true, false)]);
    assert!(loudest(&output, 800) > 0.05);
}

// ---- drawing ----

/// Draws a score text, a line and a rect in every frame, and a second text only in the even ticks.
struct Hud;

impl SceneView<Flasher> for Hud {
    fn on_update(&mut self, ctx: &mut ViewContext<'_, Flasher>) {
        let tick = ctx.tick();
        let draw = ctx.draw();
        draw.text([8.0, 8.0], 14.0, WHITE, &format!("TICK {tick}"));
        draw.line([0.0, 0.0], [1.0, 2.0], [1.0, 0.0, 0.0, 1.0]);
        draw.rect([-1.0, -1.0], [1.0, 1.0], [0.0, 1.0, 0.0, 0.5]);
        if tick % 2 == 0 {
            draw.line3([0.0, 0.0, 0.0], [0.0, 0.0, 5.0], WHITE);
        }
    }
}

#[test]
fn the_overlay_has_what_the_views_drew_and_is_cleared_for_the_next_frame() {
    let mut views = Views::<Flasher>::new();
    views.scene_view(Hud);
    let mut bridge = offline_bridge();
    assert!(views.overlay().is_empty());
    let mut ticks = Vec::new();
    for _ in 0..4 {
        frame(&mut bridge, &mut views);
        let overlay = views.overlay();
        assert_eq!(overlay.texts().len(), 1, "drawn again, not piled up");
        assert_eq!(overlay.rects().len(), 1);
        assert_eq!(overlay.texts()[0].pos, [8.0, 8.0]);
        assert_eq!(overlay.texts()[0].size, 14.0);
        let tick: u64 = overlay.texts()[0].text.strip_prefix("TICK ").unwrap().parse().unwrap();
        let even = tick % 2 == 0;
        assert_eq!(overlay.lines().len(), if even { 2 } else { 1 }, "tick {tick}");
        assert_eq!(overlay.lines()[0].a, [0.0, 0.0, 0.0]);
        assert_eq!(overlay.lines()[0].b, [1.0, 2.0, 0.0]);
        ticks.push(tick);
    }
    assert!(ticks.windows(2).all(|pair| pair[1] > pair[0]), "the text shows the tick of its own frame: {ticks:?}");
}

#[test]
fn a_frame_in_which_no_view_draws_leaves_an_empty_overlay() {
    struct EveryOther(u32);
    impl SceneView<Flasher> for EveryOther {
        fn on_update(&mut self, ctx: &mut ViewContext<'_, Flasher>) {
            self.0 += 1;
            if self.0 % 2 == 1 {
                ctx.draw().text([0.0, 0.0], 7.0, WHITE, "ON");
            }
        }
    }
    let mut views = Views::<Flasher>::new();
    views.scene_view(EveryOther(0));
    let mut bridge = offline_bridge();
    let mut counts = Vec::new();
    for _ in 0..4 {
        frame(&mut bridge, &mut views);
        counts.push(views.overlay().texts().len());
    }
    assert_eq!(counts, [1, 0, 1, 0]);
}
