use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use arc_swap::ArcSwapOption;
use orr_sim::{Game, PlayerSlot};

use orr_session::{ControlOp, Speed};
use orr_sim::DebugCommand;

use crate::bridge::{Bridge, BridgeConfig, BridgeError, SimControl};
use crate::core::{load_snapshot, SimCore, SnapshotSlot};
use crate::event::BridgeEvent;
use crate::host::SimHost;
use crate::ingress::Ingress;
use crate::snapshot::Snapshot;
use crate::{view_event_channel, ViewEventReceiver, ViewUpdate};

const NANOS: u128 = 1_000_000_000;

/// How the sim thread decides when to tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Pacing {
    /// The thread ticks by the wall clock at the host's tick rate (default).
    /// [`Bridge::update`] does nothing.
    #[default]
    Realtime,
    /// The thread ticks only when the view asks. [`Bridge::update`] then
    /// counts the elapsed time like [`InProc`](crate::InProc) and **waits**
    /// until the sim finished those ticks. For tests: with the same inputs
    /// the run matches `InProc` tick for tick. Not for a render loop.
    Manual,
}

#[derive(Clone, Copy, Debug)]
pub struct ThreadedConfig {
    pub pacing: Pacing,
    /// Realtime: the most ticks run back to back after a stall of the
    /// thread (default 8); the rest of the lag is dropped.
    pub max_catchup: u32,
}

impl Default for ThreadedConfig {
    fn default() -> Self {
        Self { pacing: Pacing::Realtime, max_catchup: 8 }
    }
}

/// Sim-on-its-own-thread adapter (design doc 5.3, "Threaded").
///
/// The sim thread owns the host. After each tick that changes what the view
/// can see it publishes an immutable [`Snapshot`] into a lock-free slot
/// (`arc_swap`). The view reads the newest one at any time without waiting.
/// Presentation events use a bounded, best-effort mailbox with explicit
/// snapshot recovery. Input updates coalesce into one held sample. Reliable
/// messages share a bounded FIFO with separate data/control quotas; explicit
/// command credits remain charged through paused/core/host staging. A running
/// host call is additional to the queued-message quotas and never holds the
/// mailbox lock. A call cannot be interrupted; shutdown is checked between calls.
pub struct Threaded<G: Game> {
    ingress: Arc<Ingress<G>>,
    events: ViewEventReceiver<G::Event>,
    slot: SnapshotSlot,
    steps_done: Arc<AtomicU64>,
    steps_sent: u64,
    handle: Option<JoinHandle<()>>,
    local_slot: PlayerSlot,
    tick_rate: u32,
    player_count: u8,
    pacing: Pacing,
    acc: u128,
}

impl<G: Game> Threaded<G> {
    /// Starts the sim thread. `make_host` runs **on that thread**, so the
    /// host need not be `Send` (a [`crate::LoopbackPair`] holds `Rc`s).
    /// Returns once the host exists and the first snapshot is published.
    pub fn spawn<H: SimHost<G> + 'static>(
        make_host: impl FnOnce() -> H + Send + 'static,
        cfg: BridgeConfig<G>,
        threaded: ThreadedConfig,
    ) -> Result<Self, BridgeError> {
        Self::try_spawn(move || Ok(make_host()), cfg, threaded).map_err(|_| BridgeError::Disconnected)
    }

    /// Like [`spawn`](Self::spawn), for a host that can fail to start (a
    /// network host that cannot reach or join the server). The error text
    /// of `make_host` is returned as is.
    pub fn try_spawn<H: SimHost<G> + 'static>(
        make_host: impl FnOnce() -> Result<H, String> + Send + 'static,
        cfg: BridgeConfig<G>,
        threaded: ThreadedConfig,
    ) -> Result<Self, String> {
        let ingress = Arc::new(Ingress::new(cfg.command_capacity, cfg.control_capacity));
        let (event_tx, events) = view_event_channel(cfg.view_event_capacity);
        let (ready_tx, ready_rx) = channel::<Result<(PlayerSlot, u32, u8), String>>();
        let slot: SnapshotSlot = Arc::new(ArcSwapOption::empty());
        let steps_done = Arc::new(AtomicU64::new(0));

        let (thread_slot, thread_steps) = (slot.clone(), steps_done.clone());
        let thread_ingress = ingress.clone();
        let handle = thread::Builder::new()
            .name("orr-sim".to_string())
            .spawn(move || {
                // Closing on unwind wakes manual callers too.
                let _close = CloseIngress(thread_ingress.clone());
                let host = match make_host() {
                    Ok(host) => host,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let info = (host.local_slot(), host.tick_rate(), host.player_count());
                let core = SimCore::new(host, cfg, event_tx, thread_slot, thread_steps, thread_ingress.clone());
                let _ = ready_tx.send(Ok(info));
                match threaded.pacing {
                    Pacing::Manual => run_manual(thread_ingress, core),
                    Pacing::Realtime => run_realtime(thread_ingress, core, info.1, threaded.max_catchup.max(1)),
                }
            })
            .map_err(|e| format!("start sim thread: {e}"))?;

        // If the factory panicked, `ready_tx` is dropped and this errors.
        let (local_slot, tick_rate, player_count) = match ready_rx.recv() {
            Ok(Ok(info)) => info,
            Ok(Err(e)) => {
                let _ = handle.join();
                return Err(e);
            }
            Err(_) => return Err(BridgeError::Disconnected.to_string()),
        };
        Ok(Self {
            ingress,
            events,
            slot,
            steps_done,
            steps_sent: 0,
            handle: Some(handle),
            local_slot,
            tick_rate,
            player_count,
            pacing: threaded.pacing,
            acc: 0,
        })
    }

    /// Counts only a successfully admitted request. Manual callers wait for
    /// its exact completion; realtime controls never wait for the sim.
    fn wait_counted(&mut self, count: u64) -> Result<(), BridgeError> {
        self.steps_sent += count;
        if self.pacing == Pacing::Manual {
            while self.steps_done.load(Ordering::Acquire) < self.steps_sent {
                if !self.is_alive() {
                    return Err(BridgeError::Disconnected);
                }
                thread::sleep(Duration::from_micros(50));
            }
            // A multi-step control may terminate early on disconnect while
            // still completing its request acknowledgement.
            if !self.is_alive() {
                return Err(BridgeError::Disconnected);
            }
        }
        Ok(())
    }

    /// Manual pacing: runs `n` ticks and waits for them to finish. Realtime
    /// and zero-count calls do nothing. Use [`Self::try_step`] to observe a
    /// terminal disconnection. The single mutable manual producer waits for
    /// every control, so its independent control quota cannot be saturated.
    pub fn step(&mut self, n: u32) {
        let _ = self.try_step(n);
    }

    /// Fallible [`Self::step`]. Input is captured at admission, so later held
    /// input changes cannot alter any tick in this explicit batch.
    pub fn try_step(&mut self, n: u32) -> Result<(), BridgeError> {
        if self.pacing != Pacing::Manual || n == 0 {
            return Ok(());
        }
        self.ingress.step(n)?;
        self.wait_counted(u64::from(n))
    }

}

impl<G: Game> Drop for Threaded<G> {
    fn drop(&mut self) {
        self.ingress.close();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl<G: Game> Bridge<G> for Threaded<G> {
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
        self.ingress.set_input(input)
    }

    fn send_command(&mut self, command: G::Command) -> Result<(), BridgeError> {
        self.ingress.command(command)
    }

    fn update(&mut self, elapsed: Duration) {
        if self.pacing == Pacing::Manual {
            let timeline = load_snapshot(&self.slot).and_then(|s| s.timeline().cloned());
            if timeline.as_ref().is_some_and(|t| !t.playing) {
                self.acc = 0;
                return;
            }
            let speed = u128::from(timeline.map_or(Speed::NORMAL, |t| t.speed).permille());
            self.acc += elapsed.as_nanos() * u128::from(self.tick_rate) * speed;
            let due = (self.acc / (NANOS * 1000)) as u32;
            self.acc -= u128::from(due) * NANOS * 1000;
            self.step(due);
        }
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
        self.ingress.is_open() && self.handle.as_ref().is_some_and(|h| !h.is_finished())
    }
}

impl<G: Game> SimControl<G> for Threaded<G> {
    fn control(&mut self, op: ControlOp) -> Result<(), BridgeError> {
        self.ingress.control(op)?;
        self.wait_counted(1)
    }

    fn debug_command(&mut self, cmd: DebugCommand) -> Result<(), BridgeError> {
        self.ingress.debug(cmd)?;
        self.wait_counted(1)
    }
}

/// RAII shutdown also covers host panics and failed startup.
struct CloseIngress<G: Game>(Arc<Ingress<G>>);
impl<G: Game> Drop for CloseIngress<G> {
    fn drop(&mut self) {
        self.0.close();
    }
}

fn run_manual<G: Game, H: SimHost<G>>(ingress: Arc<Ingress<G>>, mut core: SimCore<G, H>) {
    while let Some(msg) = ingress.recv() {
        core.apply(msg);
    }
}

/// A producer can refill even a bounded queue indefinitely. Yield to the
/// clock after this many messages, rather than draining until it is empty.
const MAX_MESSAGES_PER_PASS: usize = 64;

fn run_realtime<G: Game, H: SimHost<G>>(ingress: Arc<Ingress<G>>, mut core: SimCore<G, H>, tick_rate: u32, max_catchup: u32) {
    let mut start = Instant::now();
    // Speed in thousandths; a change or a pause restarts the tick count.
    let mut permille: u32 = 1000;
    // Tick number k runs at `deadline(k)`, counted from `start`.
    let mut k: u64 = 1;
    loop {
        for _ in 0..MAX_MESSAGES_PER_PASS {
            if !ingress.is_open() {
                return;
            }
            let Some(msg) = ingress.try_recv() else { break; };
            core.apply(msg);
        }
        if !ingress.is_open() {
            return;
        }
        let now = Instant::now();
        let (wants, speed) = (core.host().wants_tick(), core.host().speed().permille());
        if !wants || speed != permille {
            permille = speed;
            start = now;
            k = 1;
            if !wants {
                thread::sleep(Duration::from_millis(1));
                continue;
            }
        }
        let rate = u128::from(tick_rate) * u128::from(permille);
        let deadline = |k: u64| start + Duration::from_nanos((u128::from(k) * NANOS * 1000 / rate) as u64);
        let mut ran = 0;
        while deadline(k) <= now && ran < max_catchup && ingress.is_open() {
            core.step();
            k += 1;
            ran += 1;
        }
        if deadline(k) <= now {
            // Far behind: skip the missed ticks instead of running them all.
            k = ((now - start).as_nanos() * rate / (NANOS * 1000)) as u64 + 1;
        }
        let wait = deadline(k).saturating_duration_since(Instant::now());
        thread::sleep(wait.min(Duration::from_millis(1)));
    }
}

#[cfg(test)]
#[path = "threaded_tests.rs"]
mod tests;
