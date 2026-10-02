use std::time::Duration;

use orr_session::{ControlOp, Timeline};
use orr_sim::{DebugCommand, Game, PlayerSlot};

use crate::event::BridgeEvent;
use crate::snapshot::Snapshot;
use crate::{ViewUpdate, DEFAULT_VIEW_EVENT_CAPACITY};

/// Why a view-to-sim call was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgeError {
    /// This bridge carries input for the local player only.
    NotLocalPlayer { player: PlayerSlot, local: PlayerSlot },
    /// The sim thread is gone (it panicked or was shut down).
    Disconnected,
}

impl core::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BridgeError::NotLocalPlayer { player, local } => {
                write!(f, "input for player {} refused: this bridge only carries slot {}", player.0, local.0)
            }
            BridgeError::Disconnected => write!(f, "the sim side of the bridge is gone"),
        }
    }
}
impl std::error::Error for BridgeError {}

/// Turns one tick's input into commands (see [`BridgeConfig::commands_from_input`]).
pub type CommandsFromInput<G> = Box<dyn Fn(&<G as Game>::Input) -> Vec<<G as Game>::Command> + Send>;

/// Sim-side wall time of one [`Bridge`] step, given to a step observer
/// (see [`BridgeConfig::with_step_observer`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StepTiming {
    /// Time inside the sim host's `advance` (simulation, prediction, rollback resimulation).
    pub host_advance: Duration,
    /// Time to copy and publish the snapshot for the view (zero if nothing changed).
    pub publish: Duration,
    /// The host did not simulate a new tick because the prediction limit was reached.
    pub stalled: bool,
}

/// Called on the sim side after every step. Must be quick and must not block.
pub type StepObserver = Box<dyn FnMut(StepTiming) + Send>;

/// Settings shared by all adapters.
pub struct BridgeConfig<G: Game> {
    /// Maximum queued presentation notifications; overflow resets the view.
    /// This does not bound input/command queues or durable event consumers.
    pub view_event_capacity: usize,
    /// Called with the timing of every sim step (for benchmarks and debug
    /// overlays). Costs two clock reads per step; `None` costs nothing.
    pub step_observer: Option<StepObserver>,
    /// Turns the input of one tick into commands for that tick, on the sim
    /// side. `orr_testgame`'s arena, for one, spawns a bullet command when the
    /// fire bit is set. Called once per simulated tick, in the sim's order,
    /// so replays stay exact. `None` means only [`Bridge::send_command`]
    /// commands are sent.
    pub commands_from_input: Option<CommandsFromInput<G>>,
}

impl<G: Game> Default for BridgeConfig<G> {
    fn default() -> Self {
        Self { commands_from_input: None, step_observer: None, view_event_capacity: DEFAULT_VIEW_EVENT_CAPACITY }
    }
}

impl<G: Game> BridgeConfig<G> {
    pub fn with_view_event_capacity(mut self, capacity: usize) -> Self {
        self.view_event_capacity = capacity.max(1);
        self
    }

    /// Sets the step observer (see [`StepTiming`]).
    pub fn with_step_observer(mut self, f: impl FnMut(StepTiming) + Send + 'static) -> Self {
        self.step_observer = Some(Box::new(f));
        self
    }

    pub fn with_commands_from_input(mut self, f: impl Fn(&G::Input) -> Vec<G::Command> + Send + 'static) -> Self {
        self.commands_from_input = Some(Box::new(f));
        self
    }
}

/// The view's whole interface to the simulation. Same API for every
/// adapter ([`crate::InProc`], [`crate::Threaded`]); view code does not know
/// which one it holds.
///
/// Writes go through [`set_input`](Self::set_input) and
/// [`send_command`](Self::send_command) only. Reads are the immutable
/// [`Snapshot`] and the [`BridgeEvent`] stream.
pub trait Bridge<G: Game> {
    fn tick_rate(&self) -> u32;
    fn local_slot(&self) -> PlayerSlot;
    fn player_count(&self) -> u8;

    /// Sets the input of `player` that the sim samples at its next ticks
    /// (held until changed). Only the local slot is accepted.
    fn set_input(&mut self, player: PlayerSlot, input: G::Input) -> Result<(), BridgeError>;

    /// Queues a one-off command for the next tick.
    fn send_command(&mut self, command: G::Command) -> Result<(), BridgeError>;

    /// Call once per render frame with the real time since the last call.
    /// [`InProc`](crate::InProc) runs the sim ticks that are due here, on the
    /// calling thread. [`Threaded`](crate::Threaded) in real time mode ignores
    /// `elapsed`: its thread has its own clock.
    fn update(&mut self, elapsed: Duration);

    /// The newest published snapshot: a pointer copy, never blocks.
    /// `None` until the sim has published its first one.
    fn snapshot(&self) -> Option<Snapshot>;

    /// Takes presentation events in sim order. On overflow returns an explicit
    /// `BridgeEvent::ViewResynced`; reset effects and read a fresh snapshot.
    /// This best-effort channel is not a durable verified-event consumer.
    fn drain_events(&mut self) -> Vec<BridgeEvent<G::Event>>;

    /// Reads presentation output. InProc/Threaded provide bounded, coherent
    /// recovery; adapters must document their guarantees. The default keeps
    /// legacy split-read behavior and does not itself add overflow recovery.
    fn poll_view(&mut self) -> ViewUpdate<G::Event> {
        let events = self.drain_events();
        ViewUpdate { snapshot: self.snapshot(), events, resync: None }
    }

    /// `false` once the sim side has stopped (Threaded: the thread ended).
    fn is_alive(&self) -> bool;
}

/// The editor's timeline controls for a play session
/// ([`orr_session::PlaySession`] behind a [`crate::PlayHost`]). Implemented
/// by every adapter with the same behavior. Calls never block the caller
/// (except [`crate::Threaded`] with manual pacing, which waits so tests are
/// exact). The effects show up in the next [`Bridge::snapshot`] (its
/// [`Snapshot::timeline`]) and in [`crate::Lifecycle`] events.
///
/// A host without a play session ignores controls and refuses debug commands
/// (a `DebugRejected` lifecycle event).
pub trait SimControl<G: Game>: Bridge<G> {
    /// Play, pause, step, set speed, seek or branch.
    fn control(&mut self, op: ControlOp) -> Result<(), BridgeError>;

    /// Sends a byte-level edit. It is applied at a tick boundary inside the
    /// sim and recorded in the replay. It is not part of the game's
    /// `Command` type. See [`orr_sim::DebugCommand`].
    fn debug_command(&mut self, cmd: DebugCommand) -> Result<(), BridgeError>;

    /// The newest published timeline.
    fn timeline(&self) -> Option<Timeline> {
        self.snapshot().and_then(|s| s.timeline().cloned())
    }
}
