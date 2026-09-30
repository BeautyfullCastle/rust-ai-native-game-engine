use orr_ecs::Frame;

use crate::event::Lifecycle;
use orr_session::{AdvanceResult, InputSource, LoopbackClock, LoopbackEnd, LoopbackNetwork, RollbackInfo, Session, SessionConfig};
use orr_sim::{Game, PlayerSlot};

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
