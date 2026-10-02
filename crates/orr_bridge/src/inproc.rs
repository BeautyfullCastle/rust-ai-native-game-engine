use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwapOption;
use orr_sim::{Game, PlayerSlot};

use orr_session::ControlOp;
use orr_sim::DebugCommand;

use crate::bridge::{Bridge, BridgeConfig, BridgeError, SimControl};
use crate::core::{load_snapshot, SimCore, SnapshotSlot, ToSim};
use crate::event::BridgeEvent;
use crate::host::SimHost;
use crate::snapshot::Snapshot;
use crate::{view_event_channel, ViewEventReceiver, ViewUpdate};

const NANOS: u128 = 1_000_000_000;
/// One tick in accumulator units: nanosecond x tick rate x speed (in thousandths).
const TICK_UNITS: u128 = NANOS * 1000;

/// Same-thread adapter: the caller's [`Bridge::update`] runs the sim ticks
/// that are due, then returns. For tests, tools, and platforms without
/// threads (single-threaded web, some mobile setups).
///
/// It shares all sim-side code with [`crate::Threaded`], so snapshots and
/// events are the same for the same inputs.
pub struct InProc<G: Game, H: SimHost<G>> {
    core: SimCore<G, H>,
    events: ViewEventReceiver<G::Event>,
    slot: SnapshotSlot,
    local_slot: PlayerSlot,
    tick_rate: u32,
    player_count: u8,
    /// Elapsed time in units of (nanosecond x tick rate x speed); one tick is `TICK_UNITS`.
    acc: u128,
    max_catchup: u32,
}

impl<G: Game, H: SimHost<G>> InProc<G, H> {
    pub fn new(host: H, cfg: BridgeConfig<G>) -> Self {
        let (tx, events) = view_event_channel(cfg.view_event_capacity);
        let slot: SnapshotSlot = Arc::new(ArcSwapOption::empty());
        let (local_slot, tick_rate, player_count) = (host.local_slot(), host.tick_rate(), host.player_count());
        let core = SimCore::new(host, cfg, tx, slot.clone(), Arc::new(AtomicU64::new(0)));
        Self { core, events, slot, local_slot, tick_rate, player_count, acc: 0, max_catchup: 8 }
    }

    /// The most ticks one `update` may run to catch up after a long frame
    /// (default 8); the rest of the lag is dropped.
    pub fn with_max_catchup(mut self, ticks: u32) -> Self {
        self.max_catchup = ticks.max(1);
        self
    }

    /// Runs `n` ticks now, whatever the clock says.
    pub fn step(&mut self, n: u32) {
        for _ in 0..n {
            self.core.step();
        }
    }

    /// The sim host, read-only. Not part of [`Bridge`]: for tests and tools
    /// on the sim side (for example comparing checksums of both peers).
    pub fn host(&self) -> &H {
        self.core.host()
    }
}

impl<G: Game, H: SimHost<G>> Bridge<G> for InProc<G, H> {
    fn tick_rate(&self) -> u32 {
        self.tick_rate
    }

    fn local_slot(&self) -> PlayerSlot {
        self.local_slot
    }

    fn player_count(&self) -> u8 {
        self.player_count
    }

    fn set_input(&mut self, player: PlayerSlot, input: G::Input) -> Result<(), BridgeError> {
        if player != self.local_slot {
            return Err(BridgeError::NotLocalPlayer { player, local: self.local_slot });
        }
        self.core.apply(ToSim::Input(input));
        Ok(())
    }

    fn send_command(&mut self, command: G::Command) -> Result<(), BridgeError> {
        self.core.apply(ToSim::Command(command));
        Ok(())
    }

    fn update(&mut self, elapsed: Duration) {
        if !self.core.host().wants_tick() {
            // Paused: time does not pile up, so resuming does not burst.
            self.acc = 0;
            return;
        }
        let speed = u128::from(self.core.host().speed().permille());
        self.acc += elapsed.as_nanos() * u128::from(self.tick_rate) * speed;
        let due = self.acc / TICK_UNITS;
        let run = due.min(u128::from(self.max_catchup)) as u32;
        self.acc -= u128::from(run) * TICK_UNITS;
        if due > u128::from(run) {
            // Too far behind: drop the rest instead of spiralling.
            self.acc %= TICK_UNITS;
        }
        self.step(run);
    }

    fn snapshot(&self) -> Option<Snapshot> {
        load_snapshot(&self.slot)
    }

    fn drain_events(&mut self) -> Vec<BridgeEvent<G::Event>> {
        self.events.drain_events()
    }

    fn poll_view(&mut self) -> ViewUpdate<G::Event> {
        self.events.poll()
    }

    fn is_alive(&self) -> bool {
        true
    }
}

impl<G: Game, H: SimHost<G>> SimControl<G> for InProc<G, H> {
    fn control(&mut self, op: ControlOp) -> Result<(), BridgeError> {
        self.core.apply(ToSim::Control(op));
        Ok(())
    }

    fn debug_command(&mut self, cmd: DebugCommand) -> Result<(), BridgeError> {
        self.core.apply(ToSim::Debug(cmd));
        Ok(())
    }
}
