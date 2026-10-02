//! `PlaySession`: a local, single-machine play session with a timeline.
//!
//! The editor runs a `Game` from an initial `Frame`, and controls it like a
//! video: play, pause, step, change speed, seek and branch. The session
//! records every tick's inputs and commands, and every debug command, into
//! a `.orrp` recording as it goes (see [`ReplayWriter`]). Seeking restores
//! the nearest stored frame and resimulates the recorded ticks, so the
//! result is bit-identical to straight play.
//!
//! Terms:
//!
//! - *head*: the tick the simulation is at now.
//! - *last*: the last tick that has recorded inputs. Head is at most last.
//! - *rewound*: head is before last (after a seek back).
//! - *branch*: cut the recorded future after head. Recording then goes on
//!   from head with new inputs. Any tick or edit while rewound branches
//!   first, in a `Record` session. Seeking never branches.
//! - `Viewer` mode: the session was opened from a recording. Ticks play
//!   back the recorded inputs and stop at the last tick. Edits are refused.
//!   [`ControlOp::Branch`] turns the session into a `Record` session, and
//!   the opened file is never changed.
//!
//! Speed only changes how often the host calls [`PlaySession::tick`]. The
//! simulation stays fixed-step, so speed cannot change any result.
//!
//! Frames kept for seeking: a ring with the last `ring_capacity` ticks
//! (fast scrubbing near the head), and serialized keyframes every
//! `keyframe_interval` ticks. When the keyframes use more than
//! `keyframe_budget_bytes`, every second keyframe is dropped and the
//! interval doubles. The first frame is always kept.
use std::sync::Arc;

use orr_ecs::{Frame, FrameDecodeError, FrameRing};
use orr_sim::{DebugCommand, DebugError, Game, PlayerSlot, SimEvent, Simulation, TickInputs};

use crate::replay::{ReplayError, ReplayHeader, ReplayReader, ReplayWriter};

/// How many recent `(tick, checksum)` pairs a [`Timeline`] carries.
pub const TIMELINE_CHECKSUM_WINDOW: usize = 32;

/// Wall-clock speed as a fraction of real time, in thousandths
/// (1000 = 1x). Only the pacing of ticks changes, never a tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Speed(pub u32);

impl Speed {
    pub const MIN: Speed = Speed(250);
    pub const NORMAL: Speed = Speed(1000);
    pub const MAX: Speed = Speed(4000);

    /// A speed in thousandths, clamped to 0.25x..=4x.
    pub fn from_permille(permille: u32) -> Speed {
        Speed(permille.clamp(Self::MIN.0, Self::MAX.0))
    }

    pub fn permille(self) -> u32 {
        self.0
    }
}

impl Default for Speed {
    fn default() -> Self {
        Speed::NORMAL
    }
}

/// Whether the session records new ticks or plays a recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayMode {
    Record,
    Viewer,
}

/// A timeline control, sent by the editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlOp {
    /// Runs ticks by the wall clock. While rewound, a `Record` session
    /// branches on the first tick.
    Play,
    Pause,
    /// Runs `n` ticks now, whether playing or paused.
    Step(u32),
    SetSpeed(Speed),
    /// Goes to the state of this tick (inside the recorded range). Pauses.
    Seek(u64),
    /// Cuts the recorded future after head (a `Viewer` becomes `Record`).
    Branch,
}

/// A change the view should know about (turned into `Lifecycle` events by
/// the bridge).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayNote {
    /// The head jumped from `from` to `to`. Interpolation must not smooth
    /// across it.
    Seeked { from: u64, to: u64 },
    /// The recorded future after `tick` was dropped (`dropped` ticks).
    Branched { tick: u64, dropped: u64 },
    Paused { tick: u64 },
    Resumed { tick: u64 },
    /// A debug command was refused. Nothing changed.
    DebugRejected(DebugError),
    /// `Seek` went outside the recorded range, or the recording has a gap.
    SeekRejected { target: u64 },
}

/// Why a play-session operation was refused. The session is still
/// consistent (head is where the simulation is).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayError {
    OutOfRange { target: u64, first: u64, last: u64 },
    /// A recorded tick is missing (only in a damaged foreign recording).
    MissingTick(u64),
    BadKeyframe(u64),
    /// A read-only replay viewer cannot accept new inputs or commands.
    ReadOnly,
}

impl core::fmt::Display for PlayError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PlayError::OutOfRange { target, first, last } => {
                write!(f, "tick {target} is outside the recorded range {first}..={last}")
            }
            PlayError::MissingTick(t) => write!(f, "recorded tick {t} is missing"),
            PlayError::BadKeyframe(t) => write!(f, "keyframe of tick {t} does not decode"),
            PlayError::ReadOnly => f.write_str("replay viewer is read-only; branch before adding inputs or commands"),
        }
    }
}
impl std::error::Error for PlayError {}

/// What the editor shows on its timeline bar. Cheap to clone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timeline {
    pub mode: PlayMode,
    /// The tick the simulation is at (the head).
    pub tick: u64,
    /// The newest confirmed tick. A local session has no prediction, so
    /// this is always `tick`.
    pub verified_tick: u64,
    /// The first tick you can seek to (the tick of the initial frame).
    pub first_tick: u64,
    /// The last tick with recorded inputs.
    pub last_tick: u64,
    pub playing: bool,
    pub speed: Speed,
    /// The recorded checksum of `tick` (0 if none).
    pub checksum: u64,
    /// Ticks that have a stored keyframe, ascending.
    pub keyframes: Arc<[u64]>,
    /// The newest recorded `(tick, checksum)` pairs, oldest first, up to
    /// [`TIMELINE_CHECKSUM_WINDOW`], ending at `last_tick`.
    pub recent_checksums: Arc<[(u64, u64)]>,
    /// Debug commands recorded for the next tick that are not part of any
    /// stored frame yet.
    pub pending_edits: u32,
    /// Times the recording was branched.
    pub branches: u32,
    /// Changes whenever the frame of an unchanged tick may differ (seek,
    /// edit). A view drops interpolation and smoothing when it changes.
    pub epoch: u64,
}

/// Settings of a [`PlaySession`].
#[derive(Clone, Debug)]
pub struct PlayConfig {
    pub game_id: String,
    pub player_count: u8,
    pub tick_rate: u32,
    pub seed: u64,
    pub build_id: u64,
    /// Ticks between keyframes (0 = only the first frame).
    pub keyframe_interval: u64,
    /// Frames kept in the fast ring, one per tick.
    pub ring_capacity: u32,
    /// Keyframe memory budget; above it, keyframes are thinned.
    pub keyframe_budget_bytes: usize,
    /// Start paused (the editor's usual state before the user presses play).
    pub start_paused: bool,
}

impl PlayConfig {
    pub fn new(player_count: u8, seed: u64, tick_rate: u32) -> Self {
        Self {
            game_id: String::new(),
            player_count,
            tick_rate,
            seed,
            build_id: 0,
            keyframe_interval: 60,
            ring_capacity: 128,
            keyframe_budget_bytes: 64 << 20,
            start_paused: false,
        }
    }
}

/// See the module docs.
pub struct PlaySession<G: Game> {
    sim: Simulation<G>,
    cfg: PlayConfig,
    writer: ReplayWriter<G>,
    ring: FrameRing,
    base_tick: u64,
    mode: PlayMode,
    playing: bool,
    speed: Speed,
    /// Held input per slot, used by every tick until changed.
    held: Vec<G::Input>,
    pending_commands: Vec<(PlayerSlot, G::Command)>,
    /// How many debug commands of tick `head + 1` are already applied to
    /// the frame on screen (they are applied at once so an edit shows while
    /// paused). A tick applies only the rest.
    preview_applied: usize,
    /// Changes whenever the frame at the same tick may differ (seek, edit).
    epoch: u64,
    branches: u32,
    /// Keyframe interval multiplier, doubled by thinning.
    stride: u64,
    keyframe_ticks: Arc<[u64]>,
    notes: Vec<PlayNote>,
}

impl<G: Game> PlaySession<G> {
    /// Starts from `G::setup(config)`.
    pub fn new(cfg: PlayConfig, config: G::Config) -> Self {
        let sim = Simulation::<G>::with_build_id(config, cfg.tick_rate, cfg.seed, cfg.build_id);
        Self::start(cfg, sim)
    }

    /// Starts from a frame `setup` fills (a scene bake result).
    pub fn from_setup(cfg: PlayConfig, setup: impl FnOnce(&mut Frame)) -> Self {
        let sim = Simulation::<G>::with_setup(cfg.tick_rate, cfg.seed, cfg.build_id, setup);
        Self::start(cfg, sim)
    }

    /// Starts from a copy of `initial` (see [`Simulation::from_frame`]).
    pub fn from_frame(cfg: PlayConfig, initial: &Frame) -> Result<Self, FrameDecodeError> {
        let sim = Simulation::<G>::from_frame(initial, cfg.tick_rate, cfg.build_id)?;
        Ok(Self::start(cfg, sim))
    }

    fn start(cfg: PlayConfig, sim: Simulation<G>) -> Self {
        let header = ReplayHeader {
            format_version: 3,
            game_id: cfg.game_id.clone(),
            build_hash: sim.build_hash(),
            seed: cfg.seed,
            player_count: cfg.player_count,
            tick_rate: cfg.tick_rate,
            input_size: core::mem::size_of::<G::Input>() as u32,
        };
        let mut writer = ReplayWriter::new(header);
        let base_tick = sim.tick();
        writer.record_keyframe(sim.frame());
        writer.record_checksum(base_tick, sim.checksum());
        let mut ring = FrameRing::new(cfg.ring_capacity.max(2), sim.registry().clone());
        ring.store(sim.frame());
        let mut s = Self {
            held: vec![G::Input::default(); cfg.player_count as usize],
            playing: !cfg.start_paused,
            sim,
            cfg,
            writer,
            ring,
            base_tick,
            mode: PlayMode::Record,
            speed: Speed::NORMAL,
            pending_commands: Vec::new(),
            preview_applied: 0,
            epoch: 0,
            branches: 0,
            stride: 1,
            keyframe_ticks: Arc::from(Vec::new()),
            notes: Vec::new(),
        };
        s.sync_keyframes();
        s
    }

    /// Opens a recording for viewing (read-only). `config` is used only when
    /// the file has no first-frame keyframe (a recording of `Game::setup`).
    pub fn open_replay(bytes: &[u8], config: G::Config, build_id: u64) -> Result<Self, ReplayError> {
        let reader = ReplayReader::<G>::parse(bytes)?;
        crate::replay::check_build_hash(&reader.header, build_id)?;
        let h = reader.header.clone();
        let writer = reader.to_writer();
        let first = reader.first_tick();
        // The earliest keyframe before the first input is the initial frame.
        let base_key = writer.keyframe_ticks().next().filter(|&k| k < first);
        let (sim, base_tick) = match base_key {
            Some(k) => {
                let mut sim = Simulation::<G>::with_setup(h.tick_rate, h.seed, build_id, |_| {});
                let bytes = writer.keyframe(k).ok_or(ReplayError::Truncated)?;
                let frame = Frame::from_bytes(sim.registry().clone(), bytes)
                    .map_err(|source| ReplayError::BadKeyframe { tick: k, source })?;
                if frame.tick() != k {
                    let source = FrameDecodeError::Corrupt("keyframe frame tick differs from its recorded tick");
                    return Err(ReplayError::BadKeyframe { tick: k, source });
                }
                sim.restore(&frame);
                (sim, k)
            }
            None if first <= 1 => (Simulation::<G>::with_build_id(config, h.tick_rate, h.seed, build_id), 0),
            None => return Err(ReplayError::Truncated),
        };
        let cfg = PlayConfig {
            game_id: h.game_id,
            player_count: h.player_count,
            tick_rate: h.tick_rate,
            seed: h.seed,
            build_id,
            keyframe_interval: 60,
            ring_capacity: 128,
            keyframe_budget_bytes: 64 << 20,
            start_paused: true,
        };
        let mut ring = FrameRing::new(cfg.ring_capacity, sim.registry().clone());
        ring.store(sim.frame());
        let mut s = Self {
            held: vec![G::Input::default(); cfg.player_count as usize],
            playing: false,
            sim,
            cfg,
            writer,
            ring,
            base_tick,
            mode: PlayMode::Viewer,
            speed: Speed::NORMAL,
            pending_commands: Vec::new(),
            preview_applied: 0,
            epoch: 0,
            branches: 0,
            stride: 1,
            keyframe_ticks: Arc::from(Vec::new()),
            notes: Vec::new(),
        };
        s.sync_keyframes();
        Ok(s)
    }

    // ---- reads ----

    pub fn mode(&self) -> PlayMode {
        self.mode
    }
    pub fn head_tick(&self) -> u64 {
        self.sim.tick()
    }
    pub fn first_tick(&self) -> u64 {
        self.base_tick
    }
    /// The last tick with recorded inputs (the first tick if none).
    pub fn last_tick(&self) -> u64 {
        self.writer.last_tick().max(self.base_tick)
    }
    pub fn is_playing(&self) -> bool {
        self.playing
    }
    pub fn speed(&self) -> Speed {
        self.speed
    }
    pub fn tick_rate(&self) -> u32 {
        self.cfg.tick_rate
    }
    pub fn player_count(&self) -> u8 {
        self.cfg.player_count
    }
    /// Number of gameplay commands waiting for a newly recorded tick.
    /// Commands are consumed when their tick is recorded; commands in the
    /// replay history are not pending.
    pub fn pending_command_count(&self) -> usize {
        self.pending_commands.len()
    }
    pub fn simulation(&self) -> &Simulation<G> {
        &self.sim
    }
    /// The frame at the head.
    pub fn frame(&self) -> &Frame {
        self.sim.frame()
    }
    /// The frame of `tick` if the ring still holds it (or it is the head).
    pub fn frame_at(&self, tick: u64) -> Option<&Frame> {
        if tick == self.sim.tick() {
            Some(self.sim.frame())
        } else {
            self.ring.get(tick)
        }
    }
    /// Changes whenever the frame of the same tick may have changed (seek,
    /// debug edit). The bridge uses it to drop cached frames.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    /// The recorded checksum of `tick`.
    pub fn checksum_at(&self, tick: u64) -> Option<u64> {
        self.writer.checksum_at(tick)
    }
    pub fn branch_count(&self) -> u32 {
        self.branches
    }
    /// The recording so far, as `.orrp` bytes (the session goes on).
    pub fn save_replay(&self) -> Vec<u8> {
        self.writer.to_bytes()
    }

    /// True when the host should call [`tick`](Self::tick) by the clock.
    pub fn wants_tick(&self) -> bool {
        self.playing && !(self.mode == PlayMode::Viewer && self.sim.tick() >= self.last_tick())
    }

    pub fn timeline(&self) -> Timeline {
        let head = self.sim.tick();
        let last = self.last_tick();
        let from = last.saturating_sub(TIMELINE_CHECKSUM_WINDOW as u64 - 1).max(self.base_tick);
        let recent: Vec<(u64, u64)> = self.writer.checksums_in(from, last).to_vec();
        Timeline {
            mode: self.mode,
            tick: head,
            verified_tick: head,
            first_tick: self.base_tick,
            last_tick: last,
            playing: self.playing,
            speed: self.speed,
            checksum: self.writer.checksum_at(head).unwrap_or(0),
            keyframes: self.keyframe_ticks.clone(),
            recent_checksums: Arc::from(recent),
            pending_edits: if head == last { self.writer.debug_at(head + 1).len() as u32 } else { 0 },
            branches: self.branches,
            epoch: self.epoch,
        }
    }

    /// Takes the notes gathered since the last call.
    pub fn take_notes(&mut self) -> Vec<PlayNote> {
        std::mem::take(&mut self.notes)
    }

    // ---- writes ----

    /// Sets the input `slot` uses at every following recorded tick, until
    /// changed. Out-of-range slots and writes in a read-only replay viewer
    /// are ignored. Viewer inputs are not retained across a later branch.
    pub fn set_input(&mut self, slot: PlayerSlot, input: G::Input) {
        if self.mode == PlayMode::Viewer {
            return;
        }
        if let Some(held) = self.held.get_mut(slot.0 as usize) {
            *held = input;
        }
    }

    /// Queues a game command for the next tick that is run.
    /// Returns [`PlayError::ReadOnly`] if this session is viewing a replay.
    pub fn push_command(&mut self, slot: PlayerSlot, command: G::Command) -> Result<(), PlayError> {
        if self.mode == PlayMode::Viewer {
            return Err(PlayError::ReadOnly);
        }
        self.pending_commands.push((slot, command));
        Ok(())
    }

    /// One clock tick: runs a tick when [`wants_tick`](Self::wants_tick),
    /// else nothing.
    pub fn tick(&mut self) -> Vec<SimEvent<G::Event>> {
        if !self.wants_tick() {
            return Vec::new();
        }
        self.step_now().unwrap_or_default()
    }

    /// Runs one tick now, whether playing or paused. `None` when nothing can
    /// run (a `Viewer` at its last tick). A `Viewer` that reaches its last
    /// tick pauses.
    pub fn step_now(&mut self) -> Option<Vec<SimEvent<G::Event>>> {
        let events = self.step_one()?;
        if self.mode == PlayMode::Viewer && self.sim.tick() >= self.last_tick() {
            self.pause();
        }
        Some(events)
    }

    /// Runs a timeline control. Returns the events of ticks it ran (`Step`).
    pub fn control(&mut self, op: ControlOp) -> Vec<SimEvent<G::Event>> {
        match op {
            ControlOp::Play => {
                if !self.playing {
                    self.playing = true;
                    self.notes.push(PlayNote::Resumed { tick: self.sim.tick() });
                }
                Vec::new()
            }
            ControlOp::Pause => {
                self.pause();
                Vec::new()
            }
            ControlOp::SetSpeed(speed) => {
                self.speed = Speed::from_permille(speed.0);
                Vec::new()
            }
            ControlOp::Seek(target) => {
                let _ = self.seek(target);
                Vec::new()
            }
            ControlOp::Branch => {
                self.branch_here(true);
                Vec::new()
            }
            ControlOp::Step(n) => {
                let mut events = Vec::new();
                for _ in 0..n {
                    match self.step_now() {
                        Some(mut e) => events.append(&mut e),
                        None => break,
                    }
                }
                events
            }
        }
    }

    fn pause(&mut self) {
        if self.playing {
            self.playing = false;
            self.notes.push(PlayNote::Paused { tick: self.sim.tick() });
        }
    }

    /// Applies a debug edit now (so it shows while paused) and records it
    /// for the boundary before the next tick. A `Record` session that is
    /// rewound branches first. Refused edits change nothing and are also
    /// reported as [`PlayNote::DebugRejected`].
    pub fn debug(&mut self, cmd: DebugCommand) -> Result<(), DebugError> {
        if self.mode == PlayMode::Viewer {
            self.notes.push(PlayNote::DebugRejected(DebugError::ReadOnly));
            return Err(DebugError::ReadOnly);
        }
        // Apply first: a refused command must not branch, and the head frame
        // is the frame to edit whether or not the session is rewound.
        match self.sim.apply_debug(&cmd) {
            Ok(_) => {
                if self.sim.tick() < self.last_tick() {
                    self.branch_here(false);
                }
                self.writer.record_debug(self.sim.tick() + 1, cmd);
                self.preview_applied += 1;
                self.epoch += 1;
                Ok(())
            }
            Err(e) => {
                self.notes.push(PlayNote::DebugRejected(e));
                Err(e)
            }
        }
    }

    // ---- internals ----

    /// Cuts the recorded future after head. With `force`, also turns a
    /// `Viewer` into a `Record` session at head.
    fn branch_here(&mut self, force: bool) {
        let head = self.sim.tick();
        let last = self.last_tick();
        let mut changed = false;
        if head < last {
            self.writer.truncate_after(head);
            self.sync_keyframes();
            self.notes.push(PlayNote::Branched { tick: head, dropped: last - head });
            changed = true;
        }
        if self.mode == PlayMode::Viewer && (force || changed) {
            self.mode = PlayMode::Record;
            if !changed {
                self.notes.push(PlayNote::Branched { tick: head, dropped: 0 });
            }
            changed = true;
        }
        if changed {
            self.branches += 1;
        }
    }

    /// Runs the next tick: playback of a recorded tick (`Viewer`), or a new
    /// live tick that is recorded. `None` when nothing can run.
    fn step_one(&mut self) -> Option<Vec<SimEvent<G::Event>>> {
        let head = self.sim.tick();
        let next = head + 1;
        if head < self.last_tick() {
            if self.mode == PlayMode::Viewer {
                let events = self.replay_tick(next, self.preview_applied)?;
                self.preview_applied = 0;
                self.ring.store(self.sim.frame());
                return Some(events);
            }
            self.branch_here(false);
        } else if self.mode == PlayMode::Viewer {
            return None;
        }

        let mut inputs = TickInputs::<G::Input, G::Command>::new(next, self.cfg.player_count);
        for (i, input) in self.held.iter().enumerate() {
            inputs.set_input(PlayerSlot(i as u8), *input);
        }
        let commands = std::mem::take(&mut self.pending_commands);
        self.writer.record_tick(next, &self.held, &commands);
        inputs.set_commands(commands);
        let debug = self.writer.debug_at(next);
        let skip = self.preview_applied.min(debug.len());
        let events = self.sim.step_with_debug(&inputs, &debug[skip..]);
        self.preview_applied = 0;
        self.after_step(next);
        Some(events)
    }

    fn after_step(&mut self, tick: u64) {
        self.writer.record_checksum(tick, self.sim.checksum());
        self.ring.store(self.sim.frame());
        let step = self.cfg.keyframe_interval.saturating_mul(self.stride);
        if step > 0 && tick % step == 0 {
            self.writer.record_keyframe(self.sim.frame());
            let (interval, base) = (self.cfg.keyframe_interval, self.base_tick);
            while self.writer.keyframe_bytes() > self.cfg.keyframe_budget_bytes && self.stride < (1 << 30) {
                self.stride *= 2;
                let step = interval.saturating_mul(self.stride);
                self.writer.retain_keyframes(|t| t == base || t % step == 0);
            }
            self.sync_keyframes();
        }
    }

    fn sync_keyframes(&mut self) {
        let ticks: Vec<u64> = self.writer.keyframe_ticks().collect();
        self.keyframe_ticks = Arc::from(ticks);
    }

    /// Steps the simulation with the recorded inputs, commands and debug
    /// commands of `tick` (skipping the first `skip_debug` debug commands,
    /// which are already applied). `None` if the tick was not recorded.
    fn replay_tick(&mut self, tick: u64, skip_debug: usize) -> Option<Vec<SimEvent<G::Event>>> {
        let rec = self.writer.tick_data(tick)?;
        let mut inputs = TickInputs::<G::Input, G::Command>::new(tick, self.cfg.player_count);
        for (i, input) in rec.inputs.iter().enumerate().take(self.cfg.player_count as usize) {
            inputs.set_input(PlayerSlot(i as u8), *input);
        }
        inputs.set_commands(rec.commands.clone());
        let debug = self.writer.debug_at(tick);
        let skip = skip_debug.min(debug.len());
        Some(self.sim.step_with_debug(&inputs, &debug[skip..]))
    }

    /// Goes to the state of `target`. See [`ControlOp::Seek`].
    pub fn seek(&mut self, target: u64) -> Result<(), PlayError> {
        let (first, last) = (self.base_tick, self.last_tick());
        if target < first || target > last {
            self.notes.push(PlayNote::SeekRejected { target });
            return Err(PlayError::OutOfRange { target, first, last });
        }
        self.pause();
        let from = self.sim.tick();
        if target == from {
            return Ok(());
        }

        // The best place to start: the highest stored tick at or before target.
        enum Anchor {
            Head,
            Ring(u64),
            Key(u64),
        }
        let mut best: Option<(u64, Anchor)> = None;
        if target > from && self.preview_applied == 0 {
            best = Some((from, Anchor::Head));
        }
        let lowest = target.saturating_sub(u64::from(self.ring.capacity())).max(first);
        let mut t = target;
        loop {
            if self.ring.get(t).is_some() {
                if best.as_ref().is_none_or(|(bt, _)| t > *bt) {
                    best = Some((t, Anchor::Ring(t)));
                }
                break;
            }
            if t <= lowest {
                break;
            }
            t -= 1;
        }
        if let Some(k) = self.writer.nearest_keyframe(target) {
            if best.as_ref().is_none_or(|(bt, _)| k > *bt) {
                best = Some((k, Anchor::Key(k)));
            }
        }
        let (anchor_tick, anchor) = best.ok_or(PlayError::BadKeyframe(first)).inspect_err(|_| {
            self.notes.push(PlayNote::SeekRejected { target });
        })?;

        match anchor {
            Anchor::Head => {}
            Anchor::Ring(t) => {
                if let Some(frame) = self.ring.get(t) {
                    self.sim.restore(frame);
                }
            }
            Anchor::Key(k) => {
                let decoded = self
                    .writer
                    .keyframe(k)
                    .and_then(|bytes| Frame::from_bytes(self.sim.registry().clone(), bytes).ok())
                    .filter(|f| f.tick() == k);
                match decoded {
                    Some(frame) => self.sim.restore(&frame),
                    None => {
                        self.notes.push(PlayNote::SeekRejected { target });
                        return Err(PlayError::BadKeyframe(k));
                    }
                }
            }
        }
        self.preview_applied = 0;
        self.epoch += 1;

        for tick in anchor_tick + 1..=target {
            if self.replay_tick(tick, 0).is_none() {
                self.notes.push(PlayNote::SeekRejected { target });
                self.notes.push(PlayNote::Seeked { from, to: self.sim.tick() });
                return Err(PlayError::MissingTick(tick));
            }
            self.ring.store(self.sim.frame());
        }
        self.notes.push(PlayNote::Seeked { from, to: target });
        Ok(())
    }
}
