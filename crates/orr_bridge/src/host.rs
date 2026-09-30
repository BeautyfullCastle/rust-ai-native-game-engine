use orr_ecs::Frame;

use crate::event::Lifecycle;
use orr_session::{
    AdvanceResult, ControlOp, EventBatch, InputSource, LoopbackClock, LoopbackEnd, LoopbackNetwork, PlayNote, PlaySession,
    RollbackInfo, Session, SessionConfig, Speed, Timeline,
};
use orr_sim::{DebugCommand, DebugError, Game, PlayerSlot};

/// What a control or debug call on a [`SimHost`] produced: sim events and
/// lifecycle notes, sent to the view in that order.
pub struct HostOutcome<G: Game> {
    pub events: EventBatch<G::Event>,
    pub lifecycle: Vec<Lifecycle>,
}

impl<G: Game> Default for HostOutcome<G> {
    fn default() -> Self {
        Self { events: EventBatch::empty(), lifecycle: Vec::new() }
    }
}

/// The sim side of a bridge: whatever owns the simulation and steps it one
/// tick at a time with the local player's input. `orr_session::Session` is
/// one (a real peer); [`LoopbackPair`] is another (two peers in one process,
/// for demos and tests). The bridge adapters drive a host and publish what
/// it holds; the view never touches it.
pub trait SimHost<G: Game> {
    fn tick_rate(&self) -> u32;
    fn local_slot(&self) -> PlayerSlot;
    fn player_count(&self) -> u8;

    /// One fixed step with the local player's input and commands.
    fn advance(&mut self, input: G::Input, commands: Vec<G::Command>) -> AdvanceResult<G>;

    fn head_tick(&self) -> u64;
    fn verified_tick(&self) -> u64;
    fn predicted_frame(&self) -> &Frame;
    fn verified_frame(&self) -> Option<&Frame>;
    /// A stored frame of `tick`, while it is still in the snapshot ring.
    fn frame_at(&self, tick: u64) -> Option<&Frame>;
    fn rollback_count(&self) -> u64;
    fn last_rollback(&self) -> Option<RollbackInfo>;

    /// Notifications of the host itself (network play: desync, disconnect),
    /// taken after every `advance`. Default: none.
    fn take_lifecycle(&mut self) -> Vec<Lifecycle> {
        Vec::new()
    }

    /// Changes whenever a stored frame of an unchanged tick may differ
    /// (a seek or an edit). Cached frames of older epochs are dropped.
    /// Default: the rollback count.
    fn epoch(&self) -> u64 {
        self.rollback_count()
    }

    /// Whether the clock should run a tick now. A paused play session says
    /// `false`. Default: always.
    fn wants_tick(&self) -> bool {
        true
    }

    /// Pacing of the clock relative to real time. Default: 1x.
    fn speed(&self) -> Speed {
        Speed::NORMAL
    }

    /// The timeline of a play session. Default: none.
    fn timeline(&self) -> Option<Timeline> {
        None
    }

    /// A timeline control other than `Step` (the bridge runs `Step` itself
    /// with `advance`). Default: ignored.
    fn control(&mut self, _op: ControlOp) -> HostOutcome<G> {
        HostOutcome::default()
    }

    /// A debug command. Default: refused as unsupported.
    fn debug_command(&mut self, _cmd: DebugCommand) -> HostOutcome<G> {
        HostOutcome { events: EventBatch::empty(), lifecycle: vec![Lifecycle::DebugRejected(DebugError::Unsupported)] }
    }
}

impl<G: Game, S: InputSource<G>> SimHost<G> for Session<G, S> {
    fn tick_rate(&self) -> u32 {
        self.config().tick_rate
    }
    fn local_slot(&self) -> PlayerSlot {
        self.config().local_slot
    }
    fn player_count(&self) -> u8 {
        self.config().player_count
    }
    fn advance(&mut self, input: G::Input, commands: Vec<G::Command>) -> AdvanceResult<G> {
        Session::advance(self, input, commands)
    }
    fn head_tick(&self) -> u64 {
        Session::head_tick(self)
    }
    fn verified_tick(&self) -> u64 {
        Session::verified_tick(self)
    }
    fn predicted_frame(&self) -> &Frame {
        Session::predicted_frame(self)
    }
    fn verified_frame(&self) -> Option<&Frame> {
        Session::verified_frame(self)
    }
    fn frame_at(&self, tick: u64) -> Option<&Frame> {
        Session::frame_at(self, tick)
    }
    fn rollback_count(&self) -> u64 {
        Session::rollback_count(self)
    }
    fn last_rollback(&self) -> Option<RollbackInfo> {
        Session::last_rollback(self)
    }
}

/// Produces the bot peer's input (and commands) for the tick it is stamped for.
type Bot<G> = Box<dyn FnMut(u64) -> (<G as Game>::Input, Vec<<G as Game>::Command>)>;

/// Two peers of one session in one process, joined by `orr_session`'s
/// loopback network with simulated latency, so a local demo has real
/// rollbacks. Peer A (slot of `cfg_a`) is the one the view watches and
/// plays; peer B is driven by a `bot` closure. Every [`advance`](SimHost::advance)
/// steps both peers, so the network clock is logical (ticks), not wall time:
/// the run is reproducible whatever adapter drives it.
///
/// Holds `Rc`s inside (the loopback network), so it is not `Send`: build it
/// inside the sim thread with [`crate::Threaded::spawn`]'s factory closure.
pub struct LoopbackPair<G: Game> {
    a: Session<G, LoopbackEnd<G>>,
    b: Session<G, LoopbackEnd<G>>,
    clock: LoopbackClock,
    bot: Bot<G>,
}

impl<G: Game> LoopbackPair<G> {
    /// `make_config` is called twice (once per peer). `latency_ticks` and
    /// `jitter_ticks` are the one-way delivery delay of the loopback link.
    /// `bot(tick)` gets the tick B's input is stamped for.
    pub fn new(
        make_config: impl Fn() -> G::Config,
        cfg_a: SessionConfig,
        cfg_b: SessionConfig,
        latency_ticks: u64,
        jitter_ticks: u64,
        net_seed: u64,
        bot: impl FnMut(u64) -> (G::Input, Vec<G::Command>) + 'static,
    ) -> Self {
        let (end_a, end_b, clock) = LoopbackNetwork::new::<G>(latency_ticks, jitter_ticks, net_seed);
        let a = Session::<G, _>::new(make_config(), cfg_a, end_a);
        let b = Session::<G, _>::new(make_config(), cfg_b, end_b);
        Self { a, b, clock, bot: Box::new(bot) }
    }

    /// The bot peer, read-only (checksum comparison in tests).
    pub fn peer_b(&self) -> &Session<G, LoopbackEnd<G>> {
        &self.b
    }

    pub fn peer_a(&self) -> &Session<G, LoopbackEnd<G>> {
        &self.a
    }
}

impl<G: Game> SimHost<G> for LoopbackPair<G> {
    fn tick_rate(&self) -> u32 {
        self.a.config().tick_rate
    }
    fn local_slot(&self) -> PlayerSlot {
        self.a.config().local_slot
    }
    fn player_count(&self) -> u8 {
        self.a.config().player_count
    }
    fn advance(&mut self, input: G::Input, commands: Vec<G::Command>) -> AdvanceResult<G> {
        self.clock.tick();
        let result = self.a.advance(input, commands);
        let (bot_input, bot_commands) = (self.bot)(self.b.next_send_tick());
        let _ = self.b.advance(bot_input, bot_commands);
        result
    }
    fn head_tick(&self) -> u64 {
        self.a.head_tick()
    }
    fn verified_tick(&self) -> u64 {
        self.a.verified_tick()
    }
    fn predicted_frame(&self) -> &Frame {
        self.a.predicted_frame()
    }
    fn verified_frame(&self) -> Option<&Frame> {
        self.a.verified_frame()
    }
    fn frame_at(&self, tick: u64) -> Option<&Frame> {
        self.a.frame_at(tick)
    }
    fn rollback_count(&self) -> u64 {
        self.a.rollback_count()
    }
    fn last_rollback(&self) -> Option<RollbackInfo> {
        self.a.last_rollback()
    }
}

/// Produces the input of a slot other than the local one, for the tick it
/// is stamped for.
type SlotBot<G> = Box<dyn FnMut(PlayerSlot, u64) -> <G as Game>::Input>;

/// A local play session as a bridge host: the sim side of the editor's
/// play mode. See [`orr_session::PlaySession`] for the timeline semantics.
///
/// The view's input goes to the local slot; other slots are default input
/// unless [`PlayHost::with_bot`] is set.
pub struct PlayHost<G: Game> {
    session: PlaySession<G>,
    local_slot: PlayerSlot,
    bot: Option<SlotBot<G>>,
    lifecycle: Vec<Lifecycle>,
}

impl<G: Game> PlayHost<G> {
    pub fn new(session: PlaySession<G>, local_slot: PlayerSlot) -> Self {
        Self { session, local_slot, bot: None, lifecycle: Vec::new() }
    }

    /// Sets `bot(slot, tick)`, called for every non-local slot before each
    /// tick. Its input is ignored while a recording plays back.
    pub fn with_bot(mut self, bot: impl FnMut(PlayerSlot, u64) -> G::Input + 'static) -> Self {
        self.bot = Some(Box::new(bot));
        self
    }

    pub fn session(&self) -> &PlaySession<G> {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut PlaySession<G> {
        &mut self.session
    }

    fn collect_notes(&mut self) {
        for note in self.session.take_notes() {
            self.lifecycle.push(match note {
                PlayNote::Seeked { from, to } => Lifecycle::Seeked { from, to },
                PlayNote::Branched { tick, dropped } => Lifecycle::Branched { tick, dropped },
                PlayNote::Paused { tick } => Lifecycle::Paused { tick },
                PlayNote::Resumed { tick } => Lifecycle::Resumed { tick },
                PlayNote::DebugRejected(e) => Lifecycle::DebugRejected(e),
                PlayNote::SeekRejected { target } => Lifecycle::SeekRejected { target },
            });
        }
    }
}

impl<G: Game> SimHost<G> for PlayHost<G> {
    fn tick_rate(&self) -> u32 {
        self.session.tick_rate()
    }
    fn local_slot(&self) -> PlayerSlot {
        self.local_slot
    }
    fn player_count(&self) -> u8 {
        self.session.player_count()
    }
    fn advance(&mut self, input: G::Input, commands: Vec<G::Command>) -> AdvanceResult<G> {
        self.session.set_input(self.local_slot, input);
        for command in commands {
            self.session.push_command(self.local_slot, command);
        }
        if let Some(bot) = self.bot.as_mut() {
            let tick = self.session.head_tick() + 1;
            for slot in 0..self.session.player_count() {
                if PlayerSlot(slot) != self.local_slot {
                    let input = bot(PlayerSlot(slot), tick);
                    self.session.set_input(PlayerSlot(slot), input);
                }
            }
        }
        let result = match self.session.step_now() {
            Some(events) => AdvanceResult::Advanced {
                tick: self.session.head_tick(),
                events: EventBatch::verified(events),
                rollback: None,
            },
            None => AdvanceResult::Stalled { events: EventBatch::empty() },
        };
        self.collect_notes();
        result
    }
    fn head_tick(&self) -> u64 {
        self.session.head_tick()
    }
    fn verified_tick(&self) -> u64 {
        self.session.head_tick()
    }
    fn predicted_frame(&self) -> &Frame {
        self.session.frame()
    }
    fn verified_frame(&self) -> Option<&Frame> {
        Some(self.session.frame())
    }
    fn frame_at(&self, tick: u64) -> Option<&Frame> {
        self.session.frame_at(tick)
    }
    fn rollback_count(&self) -> u64 {
        0
    }
    fn last_rollback(&self) -> Option<RollbackInfo> {
        None
    }
    fn take_lifecycle(&mut self) -> Vec<Lifecycle> {
        std::mem::take(&mut self.lifecycle)
    }
    fn epoch(&self) -> u64 {
        self.session.epoch()
    }
    fn wants_tick(&self) -> bool {
        self.session.wants_tick()
    }
    fn speed(&self) -> Speed {
        self.session.speed()
    }
    fn timeline(&self) -> Option<Timeline> {
        Some(self.session.timeline())
    }
    fn control(&mut self, op: ControlOp) -> HostOutcome<G> {
        let events = self.session.control(op);
        self.collect_notes();
        HostOutcome { events: EventBatch::verified(events), lifecycle: std::mem::take(&mut self.lifecycle) }
    }
    fn debug_command(&mut self, cmd: DebugCommand) -> HostOutcome<G> {
        let _ = self.session.debug(cmd);
        self.collect_notes();
        HostOutcome { events: EventBatch::empty(), lifecycle: std::mem::take(&mut self.lifecycle) }
    }
}
