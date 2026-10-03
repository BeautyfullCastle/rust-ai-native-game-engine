//! Bounded, best-effort presentation notifications. Authoritative consumers
//! (achievements, persistence, accounting) must consume the session directly.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Arc;

use arc_swap::ArcSwapOption;

use crate::{BridgeEvent, Lifecycle, Snapshot};

/// Default maximum number of queued presentation notifications.
pub const DEFAULT_VIEW_EVENT_CAPACITY: usize = 4096;
const LIFECYCLE_KINDS: usize = 12;

/// Diagnostics coalesced while a view was behind. `last` is the most recent
/// occurrence, not an acknowledgement of every individual control request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LifecycleRecovery {
    pub count: u64,
    pub last: Lifecycle,
}

/// A discontinuity in presentation event delivery. Rebuild persistent visuals
/// from the accompanying snapshot and forget speculative effects and history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewResync {
    pub generation: u64,
    /// Notifications discarded since the preceding poll, including queued ones.
    pub discarded_events: u64,
    pub head_tick: u64,
    pub verified_tick: u64,
    /// Sticky session diagnostics survive resets even if previously observed.
    pub disconnected: bool,
    pub last_desync: Option<u64>,
    /// At most one summary per lifecycle kind, ordered by its last occurrence.
    /// Counts saturate at u64::MAX. Sim events are deliberately not replayed.
    pub lifecycle: Vec<LifecycleRecovery>,
}

/// One finite read of a bridge's presentation output. A resync's snapshot
/// covers every discarded notification; later polls cannot replay that prefix.
/// Normal polls may show a snapshot newer than their event tail.
pub struct ViewUpdate<E> {
    pub snapshot: Option<Snapshot>,
    pub events: Vec<BridgeEvent<E>>,
    pub resync: Option<ViewResync>,
}

#[derive(Clone, Copy, Default)]
struct Tally {
    count: u64,
    last: Option<Lifecycle>,
    order: u64,
}

struct Publication {
    seq: u64,
    generation: u64,
    event_count: u64,
    lifecycle: [Tally; LIFECYCLE_KINDS],
    snapshot: Option<Snapshot>,
}

struct Batch<E> {
    seq: u64,
    epoch: Option<u64>,
    events: Vec<BridgeEvent<E>>,
}

struct Shared {
    latest: ArcSwapOption<Publication>,
    queued: AtomicUsize,
}

/// Single-producer side of a bounded presentation mailbox. Each publication
/// must contain a snapshot taken AFTER all its notifications' state changes.
/// RPC responses and durable consumers must use a separate reliable protocol.
pub struct ViewEventSender<E> {
    tx: SyncSender<Batch<E>>,
    shared: Arc<Shared>,
    capacity: usize,
    seq: u64,
    generation: u64,
    event_count: u64,
    lifecycle: [Tally; LIFECYCLE_KINDS],
}

/// Single-consumer side. Dropping notifications never stops the producer.
pub struct ViewEventReceiver<E> {
    rx: Receiver<Batch<E>>,
    shared: Arc<Shared>,
    capacity: usize,
    cursor: u64,
    generation: u64,
    event_count: u64,
    lifecycle_counts: [u64; LIFECYCLE_KINDS],
    recovery_tick: Option<u64>,
    recovery_epoch: Option<u64>,
    pending_batch: Option<Batch<E>>,
}

/// Capacity is in notifications (not batches), clamped to at least one.
/// The mailbox retains at most this many notifications plus one snapshot and
/// fixed-size diagnostic summaries; a single oversized batch is discarded.
pub fn view_event_channel<E>(capacity: usize) -> (ViewEventSender<E>, ViewEventReceiver<E>) {
    let capacity = capacity.max(1);
    let (tx, rx) = sync_channel(capacity);
    let shared = Arc::new(Shared {
        latest: ArcSwapOption::empty(),
        queued: AtomicUsize::new(0),
    });
    (
        ViewEventSender {
            tx,
            shared: shared.clone(),
            capacity,
            seq: 0,
            generation: 0,
            event_count: 0,
            lifecycle: [Tally::default(); LIFECYCLE_KINDS],
        },
        ViewEventReceiver {
            rx,
            shared,
            capacity,
            cursor: 0,
            generation: 0,
            event_count: 0,
            lifecycle_counts: [0; LIFECYCLE_KINDS],
            recovery_tick: None,
            recovery_epoch: None,
            pending_batch: None,
        },
    )
}

impl<E> ViewEventSender<E> {
    /// Publishes one complete simulation outcome without waiting for a view.
    /// A full queue drops the WHOLE batch and marks a new recovery generation.
    pub fn publish(&mut self, snapshot: Option<Snapshot>, events: Vec<BridgeEvent<E>>) {
        self.publish_before_enqueue(snapshot, events, || {});
    }

    fn publish_before_enqueue(
        &mut self,
        snapshot: Option<Snapshot>,
        events: Vec<BridgeEvent<E>>,
        before_enqueue: impl FnOnce(),
    ) {
        self.seq = self
            .seq
            .checked_add(1)
            .expect("view publication sequence exhausted");
        let len = events.len();
        for event in &events {
            self.event_count = self.event_count.saturating_add(1);
            if let BridgeEvent::Lifecycle(note) = event {
                if let Some(kind) = lifecycle_kind(*note) {
                    let tally = &mut self.lifecycle[kind];
                    tally.count = tally.count.saturating_add(1);
                    tally.last = Some(*note);
                    tally.order = self.event_count;
                }
            }
        }
        // One producer reserves notification credits before publishing. Since
        // every queued batch has >= 1 item, these credits also guarantee a free
        // channel slot. The consumer returns credits as it removes batches.
        #[allow(deprecated)] // fetch_update also supports the project's older stable toolchains.
        let reserved = len > 0
            && len <= self.capacity
            && self
                .shared
                .queued
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                    n.checked_add(len).filter(|total| *total <= self.capacity)
                })
                .is_ok();
        if len > 0 && !reserved {
            self.generation = self
                .generation
                .checked_add(1)
                .expect("view recovery generation exhausted");
        }
        // Publish BEFORE enqueue: receiving a batch implies a snapshot covering
        // it is already visible. A reset may skip an as-yet unenqueued prefix.
        let epoch = snapshot
            .as_ref()
            .and_then(|s| s.timeline().map(|t| t.epoch));
        self.shared.latest.store(Some(Arc::new(Publication {
            seq: self.seq,
            generation: self.generation,
            event_count: self.event_count,
            lifecycle: self.lifecycle,
            snapshot,
        })));
        before_enqueue();
        if reserved
            && self
                .tx
                .try_send(Batch {
                    seq: self.seq,
                    epoch,
                    events,
                })
                .is_err()
        {
            // A live consumer cannot fill the queue beyond reserved credits.
            // Disconnection needs no recovery and never terminates the sim.
            self.shared.queued.fetch_sub(len, Ordering::AcqRel);
        }
    }
}

impl<E> ViewEventReceiver<E> {
    /// Takes a bounded number of complete batches, then pins ONE latest
    /// publication. No retry-until-caught-up loop, even with a fast producer.
    pub fn poll(&mut self) -> ViewUpdate<E> {
        let mut batches = Vec::new();
        let mut taken = 0;
        for _ in 0..self.capacity {
            let batch = match self
                .pending_batch
                .take()
                .or_else(|| self.rx.try_recv().ok())
            {
                Some(batch) => batch,
                None => break,
            };
            if taken + batch.events.len() > self.capacity {
                // Keep its credits until the next poll, so the lookahead is
                // included in the same hard notification bound as the queue.
                self.pending_batch = Some(batch);
                break;
            }
            self.shared
                .queued
                .fetch_sub(batch.events.len(), Ordering::AcqRel);
            taken += batch.events.len();
            batches.push(batch);
            if taken >= self.capacity {
                break;
            }
        }
        let Some(latest) = self.shared.latest.load_full() else {
            return ViewUpdate {
                snapshot: None,
                events: Vec::new(),
                resync: None,
            };
        };
        let snapshot = latest.snapshot.clone();
        let epoch = snapshot
            .as_ref()
            .and_then(|s| s.timeline().map(|t| t.epoch));
        if latest.generation != self.generation {
            let mut recovered: Vec<_> = latest
                .lifecycle
                .iter()
                .enumerate()
                .filter_map(|(i, tally)| {
                    let count = tally.count.saturating_sub(self.lifecycle_counts[i]);
                    (count > 0).then(|| {
                        (
                            tally.order,
                            LifecycleRecovery {
                                count,
                                last: tally.last.expect("nonempty lifecycle tally"),
                            },
                        )
                    })
                })
                .collect();
            recovered.sort_by_key(|(order, _)| *order);
            let resync = ViewResync {
                generation: latest.generation,
                discarded_events: latest.event_count.saturating_sub(self.event_count),
                head_tick: snapshot.as_ref().map_or(0, Snapshot::tick),
                verified_tick: snapshot.as_ref().map_or(0, Snapshot::verified_tick),
                disconnected: latest.lifecycle[4].last.is_some(),
                last_desync: match latest.lifecycle[3].last {
                    Some(Lifecycle::Desync { tick }) => Some(tick),
                    _ => None,
                },
                lifecycle: recovered.into_iter().map(|(_, r)| r).collect(),
            };
            self.cursor = latest.seq;
            if self
                .pending_batch
                .as_ref()
                .is_some_and(|batch| batch.seq <= self.cursor)
            {
                let batch = self.pending_batch.take().expect("pending prefix");
                self.shared
                    .queued
                    .fetch_sub(batch.events.len(), Ordering::AcqRel);
            }
            self.generation = latest.generation;
            self.event_count = latest.event_count;
            self.lifecycle_counts = latest.lifecycle.map(|t| t.count);
            self.recovery_tick = Some(resync.head_tick);
            self.recovery_epoch = epoch;
            return ViewUpdate {
                snapshot,
                events: Vec::new(),
                resync: Some(resync),
            };
        }
        let mut events = Vec::new();
        for batch in batches {
            if batch.seq <= self.cursor {
                continue;
            }
            self.cursor = batch.seq;
            // Attribute the floor to this ordered batch, not a newer snapshot
            // which may already belong to a later seek/branch.
            if self.recovery_epoch != batch.epoch {
                self.recovery_tick = None;
                self.recovery_epoch = batch.epoch;
            }
            for event in batch.events {
                self.event_count = self.event_count.saturating_add(1);
                match &event {
                    BridgeEvent::Lifecycle(note) => {
                        if let Some(kind) = lifecycle_kind(*note) {
                            self.lifecycle_counts[kind] =
                                self.lifecycle_counts[kind].saturating_add(1);
                        }
                        if matches!(note, Lifecycle::Seeked { .. } | Lifecycle::Branched { .. }) {
                            self.recovery_tick = None;
                        }
                    }
                    // A late confirmation/cancellation of a pre-reset effect
                    // must not resurrect a VFX whose predicted start was lost.
                    BridgeEvent::Sim { key, .. }
                        if self.recovery_tick.is_some_and(|tick| key.tick <= tick) =>
                    {
                        continue
                    }
                    _ => {}
                }
                events.push(event);
            }
        }
        ViewUpdate {
            snapshot,
            events,
            resync: None,
        }
    }

    /// Compatibility path: recovery is explicit, never silently consumed.
    /// Read a new snapshot after this call and reset presentation state when
    /// `BridgeEvent::ViewResynced` occurs. Prefer `Bridge::poll_view` for a pinned
    /// snapshot/recovery pair.
    pub fn drain_events(&mut self) -> Vec<BridgeEvent<E>> {
        let mut update = self.poll();
        if let Some(reset) = update.resync {
            update.events.insert(0, BridgeEvent::ViewResynced(reset));
        }
        update.events
    }
}

fn lifecycle_kind(note: Lifecycle) -> Option<usize> {
    Some(match note {
        Lifecycle::SessionStarted { .. } => 0,
        Lifecycle::Rollback(_) => 1,
        Lifecycle::Stalled { .. } => 2,
        Lifecycle::Desync { .. } => 3,
        Lifecycle::Disconnected => 4,
        Lifecycle::DelayChanged { .. } => 5,
        Lifecycle::Seeked { .. } => 6,
        Lifecycle::Branched { .. } => 7,
        Lifecycle::Paused { .. } => 8,
        Lifecycle::Resumed { .. } => 9,
        Lifecycle::DebugRejected(_) => 10,
        Lifecycle::SeekRejected { .. } => 11,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BridgeStats, SnapshotParts};
    use orr_ecs::{ComponentRegistryBuilder, Frame};

    fn snapshot(tick: u64) -> Snapshot {
        let mut frame = Frame::new(ComponentRegistryBuilder::new().build());
        frame.set_tick(tick);
        let frame = Arc::new(frame);
        Snapshot::from_parts(SnapshotParts {
            seq: tick + 1,
            tick,
            verified_tick: tick,
            tick_rate: 60,
            predicted: frame.clone(),
            predicted_prev: None,
            verified: Some(frame),
            stats: BridgeStats::default(),
            last_rollback: None,
            timeline: None,
        })
    }

    #[test]
    fn publication_before_enqueue_does_not_skip_normal_events() {
        let (mut tx, mut rx) = view_event_channel::<()>(2);
        let event = BridgeEvent::Lifecycle(Lifecycle::Paused { tick: 7 });
        tx.publish_before_enqueue(Some(snapshot(7)), vec![event.clone()], || {
            let update = rx.poll();
            assert_eq!(update.snapshot.unwrap().tick(), 7);
            assert!(update.events.is_empty());
            assert!(update.resync.is_none());
        });
        assert_eq!(rx.poll().events, vec![event]);
    }

    #[test]
    fn reset_before_enqueue_skips_exact_prefix_then_accepts_new_tail() {
        let (mut tx, mut rx) = view_event_channel::<()>(1);
        tx.publish(
            Some(snapshot(1)),
            vec![BridgeEvent::Lifecycle(Lifecycle::Paused { tick: 1 }); 2],
        );
        tx.publish_before_enqueue(
            Some(snapshot(2)),
            vec![BridgeEvent::Lifecycle(Lifecycle::Paused { tick: 2 })],
            || {
                let update = rx.poll();
                assert_eq!(update.snapshot.unwrap().tick(), 2);
                assert_eq!(update.resync.unwrap().discarded_events, 3);
            },
        );
        assert!(
            rx.poll().events.is_empty(),
            "late enqueue belongs to discarded prefix"
        );
        tx.publish(
            Some(snapshot(3)),
            vec![BridgeEvent::Lifecycle(Lifecycle::Resumed { tick: 3 })],
        );
        assert_eq!(
            rx.poll().events,
            vec![BridgeEvent::Lifecycle(Lifecycle::Resumed { tick: 3 })]
        );
    }

    #[test]
    fn producer_advances_across_a_barrier_pinned_recovery() {
        use std::sync::Barrier;
        let (mut tx, mut rx) = view_event_channel::<()>(1);
        tx.publish(
            Some(snapshot(1)),
            vec![BridgeEvent::Lifecycle(Lifecycle::Paused { tick: 1 }); 2],
        );
        let published = Arc::new(Barrier::new(2));
        let enqueue = Arc::new(Barrier::new(2));
        let tail = Arc::new(Barrier::new(2));
        let producer = {
            let (published, enqueue, tail) = (published.clone(), enqueue.clone(), tail.clone());
            std::thread::spawn(move || {
                tx.publish_before_enqueue(
                    Some(snapshot(2)),
                    vec![BridgeEvent::Lifecycle(Lifecycle::Paused { tick: 2 })],
                    || {
                        published.wait();
                        enqueue.wait();
                    },
                );
                tail.wait();
                tx.publish(
                    Some(snapshot(3)),
                    vec![BridgeEvent::Lifecycle(Lifecycle::Resumed { tick: 3 })],
                );
            })
        };
        published.wait();
        let update = rx.poll();
        assert_eq!(update.snapshot.as_ref().unwrap().tick(), 2);
        assert_eq!(update.resync.as_ref().unwrap().head_tick, 2);
        enqueue.wait();
        // Wait until the prefix is enqueued before removing it. A second
        // barrier is needed here; receiving is polled only in this unit test.
        while rx.shared.queued.load(Ordering::Acquire) > 0 {
            assert!(rx.poll().events.is_empty());
            std::thread::yield_now();
        }
        tail.wait();
        producer.join().unwrap();
        let newest = rx.poll();
        assert_eq!(newest.snapshot.unwrap().tick(), 3);
        assert_eq!(
            newest.events,
            vec![BridgeEvent::Lifecycle(Lifecycle::Resumed { tick: 3 })]
        );
        assert_eq!(
            update.snapshot.unwrap().tick(),
            2,
            "the recovery baseline stayed immutable"
        );
    }

    #[test]
    fn concurrent_fast_producer_keeps_each_poll_bounded_and_accounts_for_losses() {
        use std::sync::atomic::AtomicBool;
        const COUNT: u64 = 10_000;
        const CAPACITY: usize = 7;
        let (mut tx, mut rx) = view_event_channel::<()>(CAPACITY);
        let done = Arc::new(AtomicBool::new(false));
        let producer_done = done.clone();
        let producer = std::thread::spawn(move || {
            for tick in 1..=COUNT {
                tx.publish(
                    Some(snapshot(tick)),
                    vec![BridgeEvent::Lifecycle(Lifecycle::Paused { tick })],
                );
            }
            producer_done.store(true, Ordering::Release);
        });
        let mut covered = 0;
        let mut last_head = 0;
        let mut observe = |update: ViewUpdate<()>| {
            assert!(update.events.len() <= CAPACITY);
            if let Some(snapshot) = update.snapshot {
                assert!(snapshot.tick() >= last_head);
                last_head = snapshot.tick();
            }
            covered += update.events.len() as u64;
            if let Some(reset) = update.resync {
                covered += reset.discarded_events;
                assert!(reset.lifecycle.len() <= LIFECYCLE_KINDS);
            }
        };
        while !done.load(Ordering::Acquire) {
            observe(rx.poll());
            std::thread::yield_now();
        }
        producer.join().unwrap();
        loop {
            observe(rx.poll());
            if rx.shared.queued.load(Ordering::Acquire) == 0 {
                break;
            }
        }
        assert_eq!(covered, COUNT);
        assert_eq!(last_head, COUNT);
    }

    #[test]
    fn reset_keeps_previously_observed_terminal_diagnostics() {
        let (mut tx, mut rx) = view_event_channel::<()>(2);
        tx.publish(
            Some(snapshot(5)),
            vec![
                BridgeEvent::Lifecycle(Lifecycle::Desync { tick: 4 }),
                BridgeEvent::Lifecycle(Lifecycle::Disconnected),
            ],
        );
        assert_eq!(rx.poll().events.len(), 2);
        tx.publish(
            Some(snapshot(5)),
            vec![BridgeEvent::Lifecycle(Lifecycle::Stalled { head_tick: 5 }); 3],
        );
        let reset = rx.poll().resync.unwrap();
        assert!(reset.disconnected);
        assert_eq!(reset.last_desync, Some(4));
        assert_eq!(
            reset.lifecycle.len(),
            1,
            "terminal notes are sticky state, not replayed diagnostics"
        );
    }
}
