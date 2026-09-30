//! The sim-side engine shared by every adapter: steps a [`SimHost`], turns
//! its outcome into published snapshots and events.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Instant;

use arc_swap::ArcSwapOption;
use orr_ecs::Frame;
use orr_session::{AdvanceResult, ControlOp, EventBatch, Timeline};
use orr_sim::{DebugCommand, Game};

use crate::bridge::{BridgeConfig, StepTiming};
use crate::event::{BridgeEvent, BridgeStats, Lifecycle};
use crate::host::{HostOutcome, SimHost};
use crate::snapshot::{Snapshot, SnapshotData};

/// View to sim messages (Threaded). `Step` and `Shutdown` are control, not
/// game data: `Step` is used by manual pacing only.
pub(crate) enum ToSim<G: Game> {
    Input(G::Input),
    Command(G::Command),
    Step(u32),
    /// A timeline control (play session).
    Control(ControlOp),
    /// A debug command (play session).
    Debug(DebugCommand),
    Shutdown,
}

/// The slot the newest snapshot is published to; lock-free to read.
pub(crate) type SnapshotSlot = Arc<ArcSwapOption<SnapshotData>>;

/// Reuses frame allocations: a published frame comes back to the pool when
/// the last snapshot holding it is dropped.
#[derive(Default)]
struct FramePool {
    held: Vec<Arc<Frame>>,
}

impl FramePool {
    fn acquire(&mut self, src: &Frame) -> Arc<Frame> {
        for a in &mut self.held {
            // Only the pool holds it, so no snapshot can see it: safe to overwrite.
            if let Some(frame) = Arc::get_mut(a) {
                frame.copy_from(src);
                return a.clone();
            }
        }
        let a = Arc::new(src.clone());
        self.held.push(a.clone());
        a
    }
}

struct Published {
    head: u64,
    verified_tick: u64,
    rollbacks: u64,
    epoch: u64,
    timeline: Option<Timeline>,
    predicted: Arc<Frame>,
    prev: Option<Arc<Frame>>,
    verified: Option<Arc<Frame>>,
}

pub(crate) struct SimCore<G: Game, H: SimHost<G>> {
    host: H,
    cfg: BridgeConfig<G>,
    input: G::Input,
    commands: Vec<G::Command>,
    events: Sender<BridgeEvent<G::Event>>,
    slot: SnapshotSlot,
    /// Counts finished steps; lets a manual-paced caller wait for a step.
    steps_done: Arc<AtomicU64>,
    pool: FramePool,
    seq: u64,
    stats: BridgeStats,
    last: Option<Published>,
    seen_rollbacks: u64,
    was_stalled: bool,
}

impl<G: Game, H: SimHost<G>> SimCore<G, H> {
    pub(crate) fn new(
        host: H,
        cfg: BridgeConfig<G>,
        events: Sender<BridgeEvent<G::Event>>,
        slot: SnapshotSlot,
        steps_done: Arc<AtomicU64>,
    ) -> Self {
        let mut core = Self {
            host,
            cfg,
            input: G::Input::default(),
            commands: Vec::new(),
            events,
            slot,
            steps_done,
            pool: FramePool::default(),
            seq: 0,
            stats: BridgeStats::default(),
            last: None,
            seen_rollbacks: 0,
            was_stalled: false,
        };
        let started = Lifecycle::SessionStarted {
            tick_rate: core.host.tick_rate(),
            local_slot: core.host.local_slot(),
            player_count: core.host.player_count(),
        };
        let _ = core.events.send(BridgeEvent::Lifecycle(started));
        core.publish();
        core
    }

    pub(crate) fn host(&self) -> &H {
        &self.host
    }

    /// Applies one view message. `Step` and `Shutdown` are handled by the caller.
    pub(crate) fn apply(&mut self, msg: ToSim<G>) {
        match msg {
            ToSim::Input(input) => self.input = input,
            ToSim::Command(command) => self.commands.push(command),
            ToSim::Control(ControlOp::Step(n)) => {
                for _ in 0..n {
                    self.run_tick();
                }
                self.steps_done.fetch_add(1, Ordering::Release);
            }
            ToSim::Control(op) => {
                let outcome = self.host.control(op);
                self.finish_call(outcome);
            }
            ToSim::Debug(cmd) => {
                let outcome = self.host.debug_command(cmd);
                self.finish_call(outcome);
            }
            ToSim::Step(_) | ToSim::Shutdown => {}
        }
    }

    /// Sends what a control or debug call produced, publishes if anything
    /// changed, and counts the call as one finished step (manual pacing
    /// waits on the count).
    fn finish_call(&mut self, outcome: HostOutcome<G>) {
        let HostOutcome { events, lifecycle } = outcome;
        for note in lifecycle {
            let _ = self.events.send(BridgeEvent::Lifecycle(note));
        }
        self.send_events(events);
        if self.needs_publish() {
            self.publish();
        }
        self.steps_done.fetch_add(1, Ordering::Release);
    }

    fn send_events(&self, batch: EventBatch<G::Event>) {
        for (key, status) in batch.into_vec() {
            let _ = self.events.send(BridgeEvent::Sim { key, status });
        }
    }

    fn needs_publish(&self) -> bool {
        let rollbacks = self.seen_rollbacks;
        self.last.as_ref().is_none_or(|p| {
            p.head != self.host.head_tick()
                || p.verified_tick != self.host.verified_tick()
                || p.rollbacks != rollbacks
                || p.epoch != self.host.epoch()
                || p.timeline != self.host.timeline()
        })
    }

    /// One clock tick: runs a sim tick and publishes the result. Does
    /// nothing while the host does not want a tick (a paused play session).
    pub(crate) fn step(&mut self) {
        if self.host.wants_tick() {
            self.run_tick();
        }
        self.steps_done.fetch_add(1, Ordering::Release);
    }

    fn run_tick(&mut self) {
        let input = self.input;
        let mut commands = std::mem::take(&mut self.commands);
        if let Some(derive) = &self.cfg.commands_from_input {
            commands.extend(derive(&input));
        }
        let observing = self.cfg.step_observer.is_some();
        let t_start = observing.then(Instant::now);
        let (batch, stalled) = match self.host.advance(input, commands) {
            AdvanceResult::Advanced { events, .. } => {
                self.stats.ticks += 1;
                (events, false)
            }
            AdvanceResult::Stalled { events } => {
                self.stats.stalls += 1;
                (events, true)
            }
        };
        let t_advanced = observing.then(Instant::now);
        if stalled && !self.was_stalled {
            let head_tick = self.host.head_tick();
            let _ = self.events.send(BridgeEvent::Lifecycle(Lifecycle::Stalled { head_tick }));
        }
        self.was_stalled = stalled;

        let rollbacks = self.host.rollback_count();
        if rollbacks > self.seen_rollbacks {
            self.seen_rollbacks = rollbacks;
            if let Some(info) = self.host.last_rollback() {
                self.stats.rollbacks += 1;
                self.stats.resimulated_ticks += u64::from(info.resim_count);
                let depth = (info.to_tick + 1 - info.from_tick) as u32;
                self.stats.max_rollback_depth = self.stats.max_rollback_depth.max(depth);
                let _ = self.events.send(BridgeEvent::Lifecycle(Lifecycle::Rollback(info)));
            }
        }
        for note in self.host.take_lifecycle() {
            let _ = self.events.send(BridgeEvent::Lifecycle(note));
        }
        self.send_events(batch);

        if self.needs_publish() {
            self.publish();
        }
        if let (Some(start), Some(advanced), Some(observer)) = (t_start, t_advanced, self.cfg.step_observer.as_mut()) {
            // `advance` covers the host call only; the event sends in between are cheap.
            observer(StepTiming { host_advance: advanced - start, publish: advanced.elapsed(), stalled });
        }
    }

    fn publish(&mut self) {
        let head = self.host.head_tick();
        let verified_tick = self.host.verified_tick();
        let rollbacks = self.seen_rollbacks;
        let epoch = self.host.epoch();
        let timeline = self.host.timeline();

        let same_history = self.last.as_ref().filter(|p| p.rollbacks == rollbacks && p.epoch == epoch);
        let same_head = same_history.filter(|p| p.head == head);
        let predicted = match same_head {
            Some(p) => p.predicted.clone(),
            None => self.pool.acquire(self.host.predicted_frame()),
        };
        let prev = if head == 0 {
            None
        } else if let Some(p) = same_head {
            p.prev.clone()
        } else if let Some(p) = same_history.filter(|p| p.head + 1 == head) {
            // No rollback since the last publish: the old head is now the previous tick.
            Some(p.predicted.clone())
        } else {
            self.host.frame_at(head - 1).map(|f| self.pool.acquire(f))
        };
        // A verified frame never changes, so it is reused until the tick moves.
        let verified = match self.last.as_ref().filter(|p| p.epoch == epoch).and_then(|p| p.verified.as_ref()) {
            Some(v) if v.tick() == verified_tick => Some(v.clone()),
            _ => self.host.verified_frame().map(|f| self.pool.acquire(f)),
        };

        self.seq += 1;
        let data = Arc::new(SnapshotData {
            seq: self.seq,
            tick: head,
            verified_tick,
            tick_rate: self.host.tick_rate(),
            predicted: predicted.clone(),
            predicted_prev: prev.clone(),
            verified: verified.clone(),
            stats: self.stats,
            last_rollback: self.host.last_rollback(),
            timeline: timeline.clone(),
        });
        self.slot.store(Some(data));
        self.last = Some(Published { head, verified_tick, rollbacks, epoch, timeline, predicted, prev, verified });
    }
}

pub(crate) fn load_snapshot(slot: &SnapshotSlot) -> Option<Snapshot> {
    slot.load_full().map(Snapshot)
}
