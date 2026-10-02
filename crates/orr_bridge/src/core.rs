//! The sim-side engine shared by every adapter: steps a [`SimHost`], turns
//! its outcome into published snapshots and events.

use std::sync::atomic::{AtomicU64, Ordering};
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
use crate::ViewEventSender;
use crate::ingress::Ingress;

/// Reliable view-to-sim messages. Held input is coalesced separately, but
/// explicit steps capture its value when admitted to this FIFO.
pub(crate) enum ToSim<G: Game> {
    Command(G::Command),
    Step { n: u32, input: G::Input },
    Control(ControlOp),
    ControlStep { n: u32, input: G::Input },
    Debug(DebugCommand),
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
    ingress: Arc<Ingress<G>>,
    host_commands: usize,
    commands: Vec<G::Command>,
    events: ViewEventSender<G::Event>,
    pending_events: Vec<BridgeEvent<G::Event>>,
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
        events: ViewEventSender<G::Event>,
        slot: SnapshotSlot,
        steps_done: Arc<AtomicU64>,
        ingress: Arc<Ingress<G>>,
    ) -> Self {
        let host_commands = host.pending_command_count();
        ingress.initialize(host.accepts_commands(), host.branch_enables_commands(), host_commands);
        let mut core = Self {
            host,
            cfg,
            ingress,
            host_commands,
            commands: Vec::new(),
            events,
            pending_events: Vec::new(),
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
        core.pending_events.push(BridgeEvent::Lifecycle(started));
        core.publish();
        core.publish_output();
        core
    }

    pub(crate) fn host(&self) -> &H {
        &self.host
    }

    /// Applies one admitted reliable message.
    pub(crate) fn apply(&mut self, msg: ToSim<G>) {
        match msg {
            ToSim::Command(command) => self.commands.push(command),
            ToSim::ControlStep { n, input } => {
                for _ in 0..n {
                    if !self.ingress.is_open() {
                        break;
                    }
                    self.run_tick(input);
                }
                self.steps_done.fetch_add(1, Ordering::Release);
            }
            ToSim::Step { n, input } => {
                for _ in 0..n {
                    if !self.ingress.is_open() {
                        break;
                    }
                    if self.host.wants_tick() {
                        self.run_tick(input);
                    }
                    self.steps_done.fetch_add(1, Ordering::Release);
                }
            }
            ToSim::Control(op) => {
                let outcome = self.host.control(op);
                self.finish_call(outcome, op == ControlOp::Branch);
            }
            ToSim::Debug(cmd) => {
                let outcome = self.host.debug_command(cmd);
                self.finish_call(outcome, false);
            }
        }
    }

    /// Sends what a control or debug call produced, publishes if anything
    /// changed, and counts the call as one finished step (manual pacing
    /// waits on the count).
    fn finish_call(&mut self, outcome: HostOutcome<G>, branch_finished: bool) {
        let HostOutcome { events, lifecycle } = outcome;
        for note in lifecycle {
            self.lifecycle(note);
        }
        self.refresh_admission(branch_finished);
        self.send_events(events);
        if self.needs_publish() {
            self.publish();
        }
        self.publish_output();
        self.steps_done.fetch_add(1, Ordering::Release);
    }

    fn send_events(&mut self, batch: EventBatch<G::Event>) {
        for (key, status) in batch.into_vec() {
            self.pending_events.push(BridgeEvent::Sim { key, status });
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
        if self.ingress.is_open() && self.host.wants_tick() {
            self.run_tick(self.ingress.input());
        }
        self.steps_done.fetch_add(1, Ordering::Release);
    }

    fn run_tick(&mut self, input: G::Input) {
        // Preserve accepted commands if a custom host becomes read-only;
        // never forward them to an intake that may discard them.
        let mut commands = Vec::new();
        if self.host.accepts_commands() {
            commands = std::mem::take(&mut self.commands);
            self.host_commands += commands.len();
            if let Some(derive) = &self.cfg.commands_from_input {
                commands.extend(derive(&input));
            }
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
            self.pending_events.push(BridgeEvent::Lifecycle(Lifecycle::Stalled { head_tick }));
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
                self.pending_events.push(BridgeEvent::Lifecycle(Lifecycle::Rollback(info)));
            }
        }
        for note in self.host.take_lifecycle() {
            self.lifecycle(note);
        }
        self.refresh_admission(false);
        self.send_events(batch);

        if self.needs_publish() {
            self.publish();
        }
        self.publish_output();
        if let (Some(start), Some(advanced), Some(observer)) = (t_start, t_advanced, self.cfg.step_observer.as_mut()) {
            // `advance` covers the host call only; the event sends in between are cheap.
            observer(StepTiming { host_advance: advanced - start, publish: advanced.elapsed(), stalled });
        }
    }

    fn lifecycle(&mut self, note: Lifecycle) {
        if matches!(note, Lifecycle::Disconnected) {
            self.ingress.close();
        }
        self.pending_events.push(BridgeEvent::Lifecycle(note));
    }

    fn refresh_admission(&mut self, branch_finished: bool) {
        // Conservative for custom hosts that submit only part of their staging:
        // all explicit permits remain charged until the host staging is empty.
        if self.host.pending_command_count() == 0 {
            self.ingress.release_commands(self.host_commands);
            self.host_commands = 0;
        }
        self.ingress.reconcile_writable(self.host.accepts_commands(), branch_finished);
    }

    fn publish_output(&mut self) {
        self.events.publish(load_snapshot(&self.slot), std::mem::take(&mut self.pending_events));
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
