use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use arc_swap::ArcSwapOption;
use orr_sim::{Game, PlayerSlot};

use orr_session::{ControlOp, Speed};
use orr_sim::DebugCommand;

use crate::bridge::{Bridge, BridgeConfig, BridgeError, SimControl};
use crate::core::{load_snapshot, SimCore, SnapshotSlot, ToSim};
use crate::event::BridgeEvent;
use crate::host::SimHost;
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
/// snapshot recovery. Input and command channels remain unbounded.
pub struct Threaded<G: Game> {
    to_sim: Sender<ToSim<G>>,
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
        let (to_sim, from_view) = channel::<ToSim<G>>();
        let (event_tx, events) = view_event_channel(cfg.view_event_capacity);
        let (ready_tx, ready_rx) = channel::<Result<(PlayerSlot, u32, u8), String>>();
        let slot: SnapshotSlot = Arc::new(ArcSwapOption::empty());
        let steps_done = Arc::new(AtomicU64::new(0));

        let (thread_slot, thread_steps) = (slot.clone(), steps_done.clone());
        let handle = thread::Builder::new()
            .name("orr-sim".to_string())
            .spawn(move || {
                let host = match make_host() {
                    Ok(host) => host,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let info = (host.local_slot(), host.tick_rate(), host.player_count());
                let core = SimCore::new(host, cfg, event_tx, thread_slot, thread_steps);
                let _ = ready_tx.send(Ok(info));
                match threaded.pacing {
                    Pacing::Manual => run_manual(from_view, core),
                    Pacing::Realtime => run_realtime(from_view, core, info.1, threaded.max_catchup.max(1)),
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
            to_sim,
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

    /// Sends a message that counts as one finished step, and under manual
    /// pacing waits for it (tests need the effect to be visible).
    fn send_counted(&mut self, msg: ToSim<G>) -> Result<(), BridgeError> {
        self.to_sim.send(msg).map_err(|_| BridgeError::Disconnected)?;
        self.steps_sent += 1;
        if self.pacing == Pacing::Manual {
            while self.steps_done.load(Ordering::Acquire) < self.steps_sent {
                if !self.is_alive() {
                    return Err(BridgeError::Disconnected);
                }
                thread::sleep(Duration::from_micros(50));
            }
        }
        Ok(())
    }

    /// Manual pacing: runs `n` ticks and waits for them to finish.
    /// Does nothing under [`Pacing::Realtime`].
    pub fn step(&mut self, n: u32) {
        if self.pacing != Pacing::Manual || n == 0 {
            return;
        }
        if self.to_sim.send(ToSim::Step(n)).is_err() {
            return;
        }
        self.steps_sent += u64::from(n);
        while self.steps_done.load(Ordering::Acquire) < self.steps_sent {
            if !self.is_alive() {
                return;
            }
            thread::sleep(Duration::from_micros(50));
        }
    }
}

impl<G: Game> Drop for Threaded<G> {
    fn drop(&mut self) {
        let _ = self.to_sim.send(ToSim::Shutdown);
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
        self.to_sim.send(ToSim::Input(input)).map_err(|_| BridgeError::Disconnected)
    }

    fn send_command(&mut self, command: G::Command) -> Result<(), BridgeError> {
        self.to_sim.send(ToSim::Command(command)).map_err(|_| BridgeError::Disconnected)
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
        self.handle.as_ref().is_some_and(|h| !h.is_finished())
    }
}

impl<G: Game> SimControl<G> for Threaded<G> {
    fn control(&mut self, op: ControlOp) -> Result<(), BridgeError> {
        self.send_counted(ToSim::Control(op))
    }

    fn debug_command(&mut self, cmd: DebugCommand) -> Result<(), BridgeError> {
        self.send_counted(ToSim::Debug(cmd))
    }
}

fn run_manual<G: Game, H: SimHost<G>>(rx: Receiver<ToSim<G>>, mut core: SimCore<G, H>) {
    while let Ok(msg) = rx.recv() {
        match msg {
            ToSim::Step(n) => {
                for _ in 0..n {
                    core.step();
                }
            }
            ToSim::Shutdown => return,
            other => core.apply(other),
        }
    }
}

fn run_realtime<G: Game, H: SimHost<G>>(rx: Receiver<ToSim<G>>, mut core: SimCore<G, H>, tick_rate: u32, max_catchup: u32) {
    let mut start = Instant::now();
    // Speed in thousandths; a change or a pause restarts the tick count.
    let mut permille: u32 = 1000;
    // Tick number k runs at `deadline(k)`, counted from `start`.
    let mut k: u64 = 1;
    loop {
        loop {
            match rx.try_recv() {
                Ok(ToSim::Shutdown) | Err(TryRecvError::Disconnected) => return,
                Ok(msg) => core.apply(msg),
                Err(TryRecvError::Empty) => break,
            }
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
        while deadline(k) <= now && ran < max_catchup {
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
