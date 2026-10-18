use std::path::PathBuf;

use crate::{editor::Editor, game::EditorGame, Mode};

const MAX_SEEK_TICKS_PER_FRAME: u64 = 120;

/// UI-clock multipliers only; these never change the host's simulation speed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum PlaybackSpeed {
    Quarter,
    Half,
    #[default]
    Normal,
    Double,
    Quadruple,
}

impl PlaybackSpeed {
    pub(crate) const ALL: [Self; 5] = [
        Self::Quarter,
        Self::Half,
        Self::Normal,
        Self::Double,
        Self::Quadruple,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Quarter => "0.25×",
            Self::Half => "0.5×",
            Self::Normal => "1×",
            Self::Double => "2×",
            Self::Quadruple => "4×",
        }
    }

    fn factor(self) -> f64 {
        match self {
            Self::Quarter => 0.25,
            Self::Half => 0.5,
            Self::Normal => 1.0,
            Self::Double => 2.0,
            Self::Quadruple => 4.0,
        }
    }
}

#[derive(Clone, Debug)]
struct PlaybackContext {
    scene_path: Option<PathBuf>,
    scene_epoch: u64,
    first_tick: u64,
    last_tick: u64,
    tick_rate: u32,
    expected_epoch: u64,
}

#[derive(Default)]
pub(crate) struct State {
    context: Option<PlaybackContext>,
    running: bool,
    speed: PlaybackSpeed,
    anchor_tick: u64,
    anchor_time: f64,
    last_time: f64,
    last_requested_tick: u64,
    error: Option<String>,
}

impl State {
    pub(crate) fn active(&self) -> bool {
        self.context.is_some()
    }

    pub(crate) fn running(&self) -> bool {
        self.active() && self.running
    }

    pub(crate) fn speed(&self) -> PlaybackSpeed {
        self.speed
    }

    /// Reanchor at the displayed recorded tick so changing speed cannot jump.
    pub(crate) fn set_speed(&mut self, editor: &Editor, speed: PlaybackSpeed, now: f64) -> bool {
        if !valid_clock(if self.active() { self.last_time } else { now }, now) {
            self.fail("Recorded playback UI clock moved backwards or became non-finite");
            return false;
        }
        if let Some(error) = self.context_error(editor) {
            self.fail(error);
            return false;
        }
        self.speed = speed;
        if self.active() {
            self.anchor_tick = editor.sim().head_tick;
            self.anchor_time = now;
            self.last_time = now;
        }
        true
    }

    /// Scrub only the existing local recording, leaving the host paused.
    /// No step, branch or live-play command is issued by this path.
    pub(crate) fn seek_recorded(&mut self, editor: &mut Editor, tick: u64, now: f64) -> bool {
        if !valid_clock(if self.active() { self.last_time } else { now }, now) {
            self.fail("Recorded playback UI clock moved backwards or became non-finite");
            return false;
        }
        if let Some(error) = self.context_error(editor) {
            self.fail(error);
            return false;
        }
        if !Self::eligible(editor) || editor.is_viewer() || editor.sim().playing {
            self.fail(
                "Pause the local MeshNavigation session before seeking its recorded timeline",
            );
            return false;
        }
        let sim = editor.sim();
        if tick < sim.first_tick || tick > sim.last_tick {
            self.fail("Requested tick is outside the recorded MeshNavigation timeline");
            return false;
        }
        if !self.active() && !self.begin(editor, now, false) {
            return false;
        }
        editor.seek(tick);
        if editor.sim().head_tick != tick {
            self.fail("Recorded timeline seek was not accepted by the MeshNavigation host");
            return false;
        }
        self.running = false;
        self.anchor_tick = tick;
        self.last_requested_tick = tick;
        self.anchor_time = now;
        self.last_time = now;
        self.error = None;
        if let Some(context) = self.context.as_mut() {
            context.expected_epoch = editor.sim().epoch;
        }
        if let Some(error) = self.context_error(editor) {
            self.fail(error);
            return false;
        }
        true
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(crate) fn restart(&mut self, editor: &mut Editor, now: f64) -> bool {
        self.clear();
        self.begin(editor, now, true)
    }

    fn begin(&mut self, editor: &mut Editor, now: f64, from_first_tick: bool) -> bool {
        if !valid_clock(now, now) {
            self.error = Some("Recorded playback requires a finite UI clock".into());
            return false;
        }
        if !Self::eligible(editor) {
            self.error =
                Some("Recorded playback requires a local MeshNavigation Play session".into());
            return false;
        }

        if from_first_tick {
            editor.pause();
        }
        let sim = editor.sim();
        if sim.mode != Mode::Play || sim.playing || editor.is_viewer() {
            self.error = Some(
                "Pause the local MeshNavigation session before replaying its recorded timeline"
                    .into(),
            );
            return false;
        }
        let (first_tick, last_tick, tick_rate) = (sim.first_tick, sim.last_tick, sim.tick_rate);
        if first_tick > last_tick
            || tick_rate == 0
            || sim.head_tick < first_tick
            || sim.head_tick > last_tick
        {
            self.error = Some(
                "The recorded MeshNavigation timeline has an invalid range or tick rate".into(),
            );
            return false;
        }

        let start_tick = if from_first_tick {
            first_tick
        } else {
            sim.head_tick
        };
        self.context = Some(PlaybackContext {
            scene_path: editor.path(),
            scene_epoch: editor.scene_load_epoch(),
            first_tick,
            last_tick,
            tick_rate,
            expected_epoch: sim.epoch,
        });
        self.running = start_tick < last_tick;
        self.anchor_tick = start_tick;
        self.anchor_time = now;
        self.last_time = now;
        self.last_requested_tick = start_tick;
        if from_first_tick {
            editor.seek(start_tick);
            if editor.sim().head_tick != start_tick {
                self.fail("Could not seek to the first recorded MeshNavigation tick");
                return false;
            }
            if let Some(context) = self.context.as_mut() {
                context.expected_epoch = editor.sim().epoch;
            }
        }
        if let Some(error) = self.context_error(editor) {
            self.fail(error);
            return false;
        }
        true
    }

    pub(crate) fn resume(&mut self, editor: &mut Editor, now: f64) -> bool {
        if !self.active() {
            if editor.mode() == Mode::Play && editor.sim().head_tick >= editor.sim().last_tick {
                return false;
            }
            self.clear();
            return self.begin(editor, now, false);
        }
        if !valid_clock(self.last_time, now) {
            self.fail("Recorded playback UI clock moved backwards or became non-finite");
            return true;
        }
        if let Some(error) = self.context_error(editor) {
            self.fail(error);
            return true;
        }
        if editor.sim().head_tick >= self.context.as_ref().unwrap().last_tick {
            self.clear();
            return false;
        }
        self.anchor_tick = editor.sim().head_tick;
        self.last_requested_tick = editor.sim().head_tick;
        self.anchor_time = now;
        self.last_time = now;
        self.running = true;
        true
    }

    pub(crate) fn pause(&mut self, editor: &mut Editor) {
        self.clear();
        if editor.mode() == Mode::Play {
            editor.pause();
        }
    }

    pub(crate) fn pump(&mut self, editor: &mut Editor, now: f64) {
        if !self.active() {
            return;
        }
        if !valid_clock(self.last_time, now) {
            self.fail("Recorded playback UI clock moved backwards or became non-finite");
            return;
        }
        self.last_time = now;
        if let Some(error) = self.context_error(editor) {
            self.fail(error);
            return;
        }
        if !self.snapshot_matches(editor) {
            return;
        }
        if !self.running {
            if editor.sim().head_tick >= self.context.as_ref().unwrap().last_tick {
                self.clear();
            }
            return;
        }

        let context = self.context.as_ref().unwrap();
        let last_tick = context.last_tick;
        let target = match seek_target(
            self.anchor_tick,
            last_tick,
            context.tick_rate,
            self.anchor_time,
            now,
            self.last_requested_tick,
            self.speed,
        ) {
            Ok(target) => target,
            Err(error) => {
                self.fail(error);
                return;
            }
        };
        if target <= self.last_requested_tick {
            return;
        }
        editor.seek(target);
        if editor.sim().head_tick != target {
            self.fail("Recorded timeline seek was not accepted by the MeshNavigation host");
            return;
        }
        self.last_requested_tick = target;
        if let Some(context) = self.context.as_mut() {
            context.expected_epoch = editor.sim().epoch;
        }
        if target == last_tick {
            self.running = false;
        }
    }

    fn snapshot_matches(&self, editor: &Editor) -> bool {
        let Some(context) = self.context.as_ref() else {
            return false;
        };
        let Some(snapshot) = editor.snapshot() else {
            return false;
        };
        let Some(timeline) = snapshot.timeline() else {
            return false;
        };
        snapshot.tick() == editor.sim().head_tick
            && timeline.tick == editor.sim().head_tick
            && timeline.epoch == context.expected_epoch
            && timeline.last_tick == context.last_tick
            && !timeline.playing
    }

    fn context_error(&self, editor: &Editor) -> Option<&'static str> {
        let context = self.context.as_ref()?;
        let sim = editor.sim();
        if !Self::eligible(editor) || editor.is_viewer() {
            return Some(
                "Recorded playback stopped because the local MeshNavigation Play session ended",
            );
        }
        if editor.path() != context.scene_path || editor.scene_load_epoch() != context.scene_epoch {
            return Some("Recorded playback stopped because the scene changed");
        }
        if sim.playing {
            return Some("Recorded playback stopped because live timeline playback started");
        }
        if sim.first_tick != context.first_tick
            || sim.last_tick != context.last_tick
            || sim.tick_rate != context.tick_rate
            || sim.epoch != context.expected_epoch
            || sim.head_tick < context.first_tick
            || sim.head_tick > context.last_tick
            || sim.head_tick != self.last_requested_tick
        {
            return Some("Recorded playback stopped because the timeline changed");
        }
        None
    }

    fn eligible(editor: &Editor) -> bool {
        editor.game() == EditorGame::MeshNavigation
            && editor.spec().is_local()
            && editor.can_mutate()
            && editor.mode() == Mode::Play
    }

    fn fail(&mut self, error: &str) {
        self.clear();
        self.error = Some(error.into());
    }

    fn clear(&mut self) {
        self.context = None;
        self.running = false;
        self.anchor_tick = 0;
        self.anchor_time = 0.0;
        self.last_time = 0.0;
        self.last_requested_tick = 0;
        self.error = None;
    }
}

fn valid_clock(previous: f64, now: f64) -> bool {
    previous.is_finite() && now.is_finite() && now >= previous
}

fn seek_target(
    anchor_tick: u64,
    last_tick: u64,
    tick_rate: u32,
    anchor_time: f64,
    now: f64,
    last_requested_tick: u64,
    speed: PlaybackSpeed,
) -> Result<u64, &'static str> {
    if !valid_clock(anchor_time, now) {
        return Err("Recorded playback UI clock moved backwards or became non-finite");
    }
    if tick_rate == 0
        || anchor_tick > last_tick
        || last_requested_tick < anchor_tick
        || last_requested_tick > last_tick
    {
        return Err("Recorded playback range or tick rate is invalid");
    }
    let elapsed_ticks = (now - anchor_time) * f64::from(tick_rate) * speed.factor();
    if !elapsed_ticks.is_finite() {
        return Err("Recorded playback UI clock exceeded its bounded range");
    }
    let available = last_tick - anchor_tick;
    let elapsed_offset = if elapsed_ticks <= 0.0 {
        0
    } else if elapsed_ticks >= available as f64 {
        available
    } else {
        elapsed_ticks.floor() as u64
    };
    let desired = anchor_tick + elapsed_offset;
    let frame_limit = last_requested_tick
        .saturating_add(MAX_SEEK_TICKS_PER_FRAME)
        .min(last_tick);
    Ok(desired.max(last_requested_tick).min(frame_limit))
}

#[cfg(test)]
#[path = "mesh_recorded_playback/tests.rs"]
mod tests;
