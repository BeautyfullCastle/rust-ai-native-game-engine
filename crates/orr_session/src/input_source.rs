use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;

use orr_fp::FrameRng;
use orr_sim::{Game, PlayerSlot, TickInputs};

/// One remote player's confirmed input (and any commands they submitted)
/// for one tick, as delivered by an [`InputSource`].
pub struct RemoteInput<G: Game> {
    pub tick: u64,
    pub slot: PlayerSlot,
    pub input: G::Input,
    pub commands: Vec<G::Command>,
    /// The server confirmed this tick with nobody present in the slot (relay
    /// `FLAG_ABSENT`): the game sees `PlayerFlags::disconnected`. Always
    /// `false` outside relay sessions.
    pub disconnected: bool,
}

/// Borrowed evidence for a tick that just passed the local Session verification
/// gate. Historical `predicted` flags are not evidence of receipt. Consumers
/// requiring exact payload agreement must independently compare all slots and
/// ordered commands: the gameplay gate does not reconcile arbitrary conflicting
/// duplicates for slots that were already confirmed when simulated.
///
/// This view is synchronous and never allocates, clones, or encodes. Missing
/// command-map entries mean an empty command list for that confirmed slot.
/// This is local verification, not distributed consensus or packet provenance.
pub struct LocallyVerifiedTick<'a, G: Game> {
    pub simulated: &'a TickInputs<G::Input, G::Command>,
    /// Bytes recorded when the tick was simulated, in simulation order.
    pub simulated_commands: &'a [(PlayerSlot, Vec<u8>)],
    pub confirmed_inputs: &'a BTreeMap<PlayerSlot, G::Input>,
    pub confirmed_commands: Option<&'a BTreeMap<PlayerSlot, Vec<G::Command>>>,
    pub confirmed_absent: &'a BTreeSet<(u64, u8)>,
}

/// Where a [`crate::Session`] gets other players' confirmed inputs from.
///
/// The local player's own input is *not* routed through this trait — the
/// session already knows it with certainty the instant it's produced, so
/// `Session::advance` records it directly. `InputSource` only needs to
/// deliver everyone else's, whenever they arrive (immediately for
/// [`LocalInputSource`]'s single-player case, after simulated latency for
/// [`LoopbackNetwork`], or from a recorded file for a replay reader).
pub trait InputSource<G: Game> {
    /// Called once per [`crate::Session::advance`] with the local player's
    /// input for `tick` (already offset by the session's input delay), so
    /// a network implementation can transmit it to peers.
    fn send_local(&mut self, tick: u64, slot: PlayerSlot, input: G::Input, commands: Vec<G::Command>);

    /// Drains every remote input that has newly become available (i.e.
    /// "arrived") since the last call. Order is not significant — the
    /// session sorts by tick internally.
    fn poll_remote(&mut self) -> Vec<RemoteInput<G>>;

    /// Once per newly locally verified tick, after the existing verification
    /// gate and before its simulated/confirmed bookkeeping is pruned. Not
    /// called for rollback execution or late packets for old verified ticks.
    /// Default sources pay no additional encoding, cloning, or allocation cost.
    /// Wrappers must forward this to their inner source. An observer belongs to
    /// one Session lifetime: replace its generation or explicitly invalidate it
    /// before restoring/replacing that Session, including forward restores.
    fn on_locally_verified(&mut self, _tick: LocallyVerifiedTick<'_, G>) {}

}

/// A no-op transport for single-player sessions: there are no remote
/// players, so nothing is ever delivered.
#[derive(Default)]
pub struct LocalInputSource;

impl<G: Game> InputSource<G> for LocalInputSource {
    fn send_local(&mut self, _tick: u64, _slot: PlayerSlot, _input: G::Input, _commands: Vec<G::Command>) {}
    fn poll_remote(&mut self) -> Vec<RemoteInput<G>> {
        Vec::new()
    }
}

struct Envelope<G: Game> {
    deliver_at: u64,
    tick: u64,
    slot: PlayerSlot,
    input: G::Input,
    commands: Vec<G::Command>,
}

struct Link<G: Game> {
    latency: u64,
    jitter: u64,
    rng: FrameRng,
    queue: VecDeque<Envelope<G>>,
}

/// Drives the shared "time in flight" clock for a [`LoopbackNetwork`]. Call
/// [`LoopbackClock::tick`] once per round (e.g. right before both peers
/// call `Session::advance`) so packets take the configured number of ticks
/// to arrive, regardless of which peer's `advance` runs first.
#[derive(Clone)]
pub struct LoopbackClock(Rc<RefCell<u64>>);

impl LoopbackClock {
    pub fn tick(&self) {
        *self.0.borrow_mut() += 1;
    }

    pub fn now(&self) -> u64 {
        *self.0.borrow()
    }
}

/// A local, two-peer, in-process fake network: exercises `Session`'s
/// prediction/rollback path without real sockets. Latency and jitter are
/// expressed in ticks.
pub struct LoopbackNetwork;

impl LoopbackNetwork {
    /// Builds a connected pair of ends plus the [`LoopbackClock`] driving
    /// both. `latency_ticks` is the one-way delivery delay; `jitter_ticks`
    /// adds a deterministic `[0, jitter]` wobble (seeded, so runs are
    /// reproducible) on top of it.
    #[allow(clippy::new_ret_no_self)]
    pub fn new<G: Game>(
        latency_ticks: u64,
        jitter_ticks: u64,
        seed: u64,
    ) -> (LoopbackEnd<G>, LoopbackEnd<G>, LoopbackClock) {
        let clock = LoopbackClock(Rc::new(RefCell::new(0u64)));
        let (a, b) = Self::with_clock(&clock, latency_ticks, jitter_ticks, seed);
        (a, b, clock)
    }

    /// Like [`LoopbackNetwork::new`], but the pair shares an existing
    /// `clock`, so a link added later (e.g. for a late joiner) runs on the
    /// same time as the others.
    pub fn with_clock<G: Game>(
        clock: &LoopbackClock,
        latency_ticks: u64,
        jitter_ticks: u64,
        seed: u64,
    ) -> (LoopbackEnd<G>, LoopbackEnd<G>) {
        // `a_to_b` carries envelopes A sends that B receives, and vice
        // versa; both ends share both links (one to send into, one to
        // read from) plus one shared clock driving delivery times for
        // both directions.
        let a_to_b = Rc::new(RefCell::new(Link {
            latency: latency_ticks,
            jitter: jitter_ticks,
            rng: FrameRng::new(seed),
            queue: VecDeque::new(),
        }));
        let b_to_a = Rc::new(RefCell::new(Link {
            latency: latency_ticks,
            jitter: jitter_ticks,
            rng: FrameRng::new(seed ^ 0x9E37_79B9_7F4A_7C15),
            queue: VecDeque::new(),
        }));
        (
            LoopbackEnd { send: a_to_b.clone(), recv: b_to_a.clone(), clock: clock.0.clone() },
            LoopbackEnd { send: b_to_a, recv: a_to_b, clock: clock.0.clone() },
        )
    }
}

/// One side of a [`LoopbackNetwork`].
pub struct LoopbackEnd<G: Game> {
    send: Rc<RefCell<Link<G>>>,
    recv: Rc<RefCell<Link<G>>>,
    clock: Rc<RefCell<u64>>,
}

impl<G: Game> LoopbackEnd<G> {
    fn now(&self) -> u64 {
        *self.clock.borrow()
    }
}

impl<G: Game> InputSource<G> for LoopbackEnd<G> {
    fn send_local(&mut self, tick: u64, slot: PlayerSlot, input: G::Input, commands: Vec<G::Command>) {
        let now = self.now();
        let mut link = self.send.borrow_mut();
        let jitter = if link.jitter > 0 { link.rng.next_u32() as u64 % (link.jitter + 1) } else { 0 };
        let deliver_at = now + link.latency + jitter;
        link.queue.push_back(Envelope { deliver_at, tick, slot, input, commands });
    }

    fn poll_remote(&mut self) -> Vec<RemoteInput<G>> {
        let now = self.now();
        let mut link = self.recv.borrow_mut();
        let mut out = Vec::new();
        // `queue` is FIFO by send order, not by `deliver_at` (jitter can
        // reorder), so scan-and-remove rather than peek-front.
        let mut i = 0;
        while i < link.queue.len() {
            if link.queue[i].deliver_at <= now {
                let env = link.queue.remove(i).unwrap();
                out.push(RemoteInput { tick: env.tick, slot: env.slot, input: env.input, commands: env.commands, disconnected: false });
            } else {
                i += 1;
            }
        }
        out
    }
}
