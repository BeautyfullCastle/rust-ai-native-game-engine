//! The negotiated ERP presentation mailbox. The decoder stages notifications
//! until an actual immutable snapshot covers them. Neither queue can make the
//! simulation wait; transport/RPC queues have a separate lifetime and budget.
use orr_bridge::{BridgeEvent, Lifecycle, LifecycleRecovery, Snapshot, ViewResync, ViewUpdate};
use serde_json::Value as J;

use crate::wire::debug_error_from_name;

const KINDS: usize = 12;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Tally {
    count: u64,
    order: u64,
    last: Option<Lifecycle>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stamp {
    pub subscription: u64,
    pub timeline: u64,
    pub cursor: u64,
    pub count: u64,
}

impl Stamp {
    pub fn parse(j: &J, frame: bool) -> Result<Self, String> {
        Ok(Self {
            subscription: exact(j, "subscription")?,
            timeline: exact(j, "timeline")?,
            cursor: exact(j, if frame { "through_cursor" } else { "cursor" })?,
            count: exact(j, "count")?,
        })
    }
}

pub(crate) fn exact(j: &J, key: &str) -> Result<u64, String> {
    let text = j
        .get(key)
        .and_then(J::as_str)
        .ok_or_else(|| format!("missing exact view delivery {key}"))?;
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("invalid view delivery {key}"));
    }
    text.parse()
        .map_err(|_| format!("view delivery {key} exceeds u64"))
}

pub(crate) fn note(n: &J) -> Result<Lifecycle, String> {
    let u = |key| {
        n.get(key)
            .and_then(J::as_u64)
            .ok_or_else(|| format!("invalid presentation note {key}"))
    };
    Ok(match n.get("kind").and_then(J::as_str) {
        Some("seeked") => Lifecycle::Seeked {
            from: u("from")?,
            to: u("to")?,
        },
        Some("branched") => Lifecycle::Branched {
            tick: u("tick")?,
            dropped: u("dropped")?,
        },
        Some("paused") => Lifecycle::Paused { tick: u("tick")? },
        Some("resumed") => Lifecycle::Resumed { tick: u("tick")? },
        Some("debug_rejected") => Lifecycle::DebugRejected(debug_error_from_name(
            n.get("error")
                .and_then(J::as_str)
                .ok_or("invalid debug rejection")?,
        )),
        Some("seek_rejected") => Lifecycle::SeekRejected {
            target: u("target")?,
        },
        _ => return Err("unknown negotiated presentation note".into()),
    })
}

fn kind(life: Lifecycle) -> usize {
    match life {
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
    }
}

struct Batch<E> {
    stamp: Stamp,
    events: Vec<BridgeEvent<E>>,
}

pub(crate) struct ViewMailbox<E> {
    capacity: usize,
    subscription: Option<u64>,
    received_cursor: u64,
    received_count: u64,
    received_timeline: u64,
    /// Fixed-size evidence retained even when an oversized batch is discarded.
    required_head: Option<(u64, u64)>,
    covered_cursor: u64,
    covered_count: u64,
    covered_timeline: Option<u64>,
    consumed_count: u64,
    covered_lifecycle: [Tally; KINDS],
    observed_lifecycle: [Tally; KINDS],
    history_complete: bool,
    consumed_lifecycle: [u64; KINDS],
    server_generation: u64,
    generation: u64,
    /// Highest discarded cursor still waiting for a covering frame.
    waiting: Option<u64>,
    reset_pending: bool,
    staged: Vec<Batch<E>>,
    ready: Vec<BridgeEvent<E>>,
    retained: usize,
    snapshot: Option<Snapshot>,
    floor: Option<(u64, u64)>,
    disconnected: bool,
    disconnect_reported: bool,
    started: Option<Lifecycle>,
}

/// A validated frame cut that can be committed after the caller has also
/// validated its codec state. Preparing is read-only; committing cannot fail.
pub(crate) struct PreparedFrame {
    stamp: Stamp,
    generation: u64,
    summaries: [Tally; KINDS],
    snapshot: Option<Snapshot>,
    reset_generation: Option<u64>,
}

pub(crate) struct PreparedRenewal {
    subscription: u64,
    cursor: u64,
    count: u64,
    generation: u64,
}

impl<E> ViewMailbox<E> {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            subscription: None,
            received_cursor: 0,
            received_count: 0,
            received_timeline: 0,
            required_head: None,
            covered_cursor: 0,
            covered_count: 0,
            covered_timeline: None,
            consumed_count: 0,
            covered_lifecycle: [Tally::default(); KINDS],
            observed_lifecycle: [Tally::default(); KINDS],
            history_complete: true,
            consumed_lifecycle: [0; KINDS],
            server_generation: 0,
            generation: 0,
            waiting: None,
            reset_pending: false,
            staged: Vec::new(),
            ready: Vec::new(),
            retained: 0,
            snapshot: None,
            floor: None,
            disconnected: false,
            disconnect_reported: false,
            started: None,
        }
    }

    pub fn negotiate(&mut self, result: &J) -> Result<(), String> {
        self.subscription = Some(exact(result, "subscription")?);
        self.received_cursor = exact(result, "cursor")?;
        self.received_count = exact(result, "count")?;
        self.history_complete = self.received_count == 0;
        self.covered_cursor = self.received_cursor;
        self.covered_count = self.received_count;
        self.consumed_count = self.received_count;
        Ok(())
    }

    /// Switches to a newly acknowledged subscription after a correlated
    /// `watch.subscribe` reset. Old notification history is discarded while
    /// the last published snapshot remains visible until a covering frame
    /// arrives on the new subscription.
    #[cfg(test)]
    pub fn renew_subscription(&mut self, result: &J) -> Result<u64, String> {
        let prepared = self.prepare_renewal(result)?;
        let subscription = prepared.subscription;
        self.commit_renewal(prepared);
        Ok(subscription)
    }

    pub fn prepare_renewal(&self, result: &J) -> Result<PreparedRenewal, String> {
        let subscription = exact(result, "subscription")?;
        let cursor = exact(result, "cursor")?;
        let count = exact(result, "count")?;
        if self.subscription.is_none() || self.subscription == Some(subscription) {
            return Err("reset subscribe did not issue a new subscription".into());
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| "presentation reset generation exhausted".to_string())?;
        Ok(PreparedRenewal {
            subscription,
            cursor,
            count,
            generation,
        })
    }

    /// Starts a correlated reset without clearing the currently displayed
    /// snapshot. No old staged or ready notification can escape while the
    /// replacement subscription is pending.
    pub fn begin_renewal(&mut self) {
        self.waiting = Some(self.received_cursor);
        self.reset_pending = false;
        self.staged.clear();
        self.ready.clear();
        self.retained = 0;
    }

    pub fn commit_renewal(&mut self, prepared: PreparedRenewal) {
        let PreparedRenewal {
            subscription,
            cursor,
            count,
            generation,
        } = prepared;
        self.subscription = Some(subscription);
        self.received_cursor = cursor;
        self.received_count = count;
        self.received_timeline = 0;
        self.required_head = None;
        self.covered_cursor = cursor;
        self.covered_count = count;
        self.covered_timeline = None;
        self.consumed_count = count;
        self.covered_lifecycle = [Tally::default(); KINDS];
        self.observed_lifecycle = [Tally::default(); KINDS];
        self.history_complete = count == 0;
        self.consumed_lifecycle = [0; KINDS];
        self.server_generation = 0;
        self.generation = generation;
        self.waiting = Some(cursor);
        self.reset_pending = false;
        self.staged.clear();
        self.ready.clear();
        self.retained = 0;
        self.floor = None;
    }

    pub fn negotiated(&self) -> bool {
        self.subscription.is_some()
    }

    pub fn started(&mut self, life: Lifecycle) {
        self.started = Some(life);
    }

    pub fn disconnect(&mut self) {
        self.disconnected = true;
    }

    pub fn fail_closed(&mut self) {
        self.disconnect();
        self.ready.clear();
        self.staged.clear();
        self.retained = 0;
        // There is no trustworthy future cut on a malformed connection.
        self.waiting = Some(self.received_cursor);
    }

    fn validate(&self, stamp: Stamp) -> Result<(), String> {
        if self.subscription != Some(stamp.subscription) || stamp.timeline < self.received_timeline
        {
            return Err("view delivery subscription/timeline changed backwards".into());
        }
        Ok(())
    }

    fn apply_reset(&mut self, cursor: u64, generation: u64) {
        self.generation = generation;
        self.waiting = Some(self.waiting.map_or(cursor, |old| old.max(cursor)));
        self.ready.clear();
        self.staged.clear();
        self.retained = 0;
    }

    pub fn stage(&mut self, stamp: Stamp, events: Vec<BridgeEvent<E>>) -> Result<(), String> {
        self.validate(stamp)?;
        let next_cursor = self
            .received_cursor
            .checked_add(1)
            .ok_or_else(|| "presentation cursor exhausted".to_string())?;
        let new_event_count = u64::try_from(events.len())
            .map_err(|_| "presentation event count exceeds u64".to_string())?;
        let minimum_count = self
            .received_count
            .checked_add(new_event_count)
            .ok_or_else(|| "presentation event count exhausted".to_string())?;
        if stamp.cursor <= self.received_cursor || stamp.count < minimum_count {
            return Err("non-monotonic negotiated notification cursor/count".into());
        }
        let mut required_head = self.required_head;
        if required_head.is_some_and(|(timeline, _)| timeline != stamp.timeline) {
            required_head = None;
        }
        let order_start = if stamp.cursor == next_cursor && stamp.count == minimum_count {
            self.received_count
        } else {
            stamp.count.saturating_sub(new_event_count)
        };
        let mut observed_lifecycle = self.observed_lifecycle;
        for (index, event) in events.iter().enumerate() {
            if let BridgeEvent::Lifecycle(life) = event {
                let tally = &mut observed_lifecycle[kind(*life)];
                tally.count = tally
                    .count
                    .checked_add(1)
                    .ok_or_else(|| "presentation lifecycle count exhausted".to_string())?;
                tally.order = order_start
                    .checked_add(index as u64)
                    .and_then(|order| order.checked_add(1))
                    .ok_or_else(|| "presentation lifecycle order exhausted".to_string())?;
                tally.last = Some(*life);
            }
            if let BridgeEvent::Sim { key, .. } = event {
                let head = required_head.map_or(key.tick, |(_, old)| old.max(key.tick));
                required_head = Some((stamp.timeline, head));
            }
        }
        let gap = stamp.cursor != next_cursor || stamp.count != minimum_count;
        let transition = self.received_timeline != 0 && self.received_timeline != stamp.timeline;
        let reset_generation = if gap
            || transition
            || self.waiting.is_some()
            || events.len() > self.capacity.saturating_sub(self.retained)
        {
            Some(
                self.generation
                    .checked_add(1)
                    .ok_or_else(|| "presentation reset generation exhausted".to_string())?,
            )
        } else {
            None
        };
        self.required_head = required_head;
        self.observed_lifecycle = observed_lifecycle;
        if gap {
            self.history_complete = false;
        }
        self.received_cursor = stamp.cursor;
        self.received_count = stamp.count;
        self.received_timeline = stamp.timeline;
        if gap
            || transition
            || self.waiting.is_some()
            || events.len() > self.capacity.saturating_sub(self.retained)
        {
            self.apply_reset(
                stamp.cursor,
                reset_generation.expect("reset was preflighted"),
            );
            return Ok(());
        }
        self.retained += events.len();
        if !events.is_empty() {
            self.staged.push(Batch { stamp, events });
        }
        Ok(())
    }

    pub fn prepare_frame(
        &self,
        metadata: &J,
        snapshot: Option<Snapshot>,
    ) -> Result<PreparedFrame, String> {
        let stamp = Stamp::parse(metadata, true)?;
        self.validate(stamp)?;
        let generation = exact(metadata, "loss_generation")?;
        if self.required_head.is_some_and(|(timeline, head)| {
            timeline == stamp.timeline && snapshot.as_ref().is_none_or(|s| s.tick() < head)
        }) {
            return Err("negotiated baseline does not cover observed event ticks".into());
        }
        if self.covered_timeline == Some(stamp.timeline) {
            match (&self.snapshot, &snapshot) {
                (Some(previous), Some(next)) if next.tick() < previous.tick() => {
                    return Err("negotiated frame tick regressed within a timeline".into());
                }
                (Some(previous), Some(next))
                    if next.tick() == previous.tick()
                        && next.predicted().checksum() != previous.predicted().checksum() =>
                {
                    return Err(
                        "negotiated frame changed at the same tick without a new timeline".into(),
                    );
                }
                (Some(_), None) | (None, Some(_)) => {
                    return Err(
                        "negotiated source availability changed without a new timeline".into(),
                    );
                }
                _ => {}
            }
        }
        if stamp.cursor < self.received_cursor
            || stamp.cursor < self.covered_cursor
            || stamp.count < self.received_count
            || generation < self.server_generation
        {
            return Err("stale negotiated frame cut".into());
        }
        let list = metadata
            .get("lifecycle")
            .and_then(J::as_array)
            .ok_or("missing view lifecycle summaries")?;
        if list.len() > KINDS {
            return Err("too many view lifecycle summaries".into());
        }
        let mut summaries = [Tally::default(); KINDS];
        for entry in list {
            let life = note(entry.get("note").ok_or("missing lifecycle note")?)?;
            let k = kind(life);
            let count = exact(entry, "count")?;
            let order = exact(entry, "order")?;
            if summaries[k].last.is_some()
                || count == 0
                || count > stamp.count
                || order == 0
                || count > order
                || order > stamp.count
                || count < self.covered_lifecycle[k].count
            {
                return Err("invalid cumulative lifecycle summary".into());
            }
            summaries[k] = Tally {
                count,
                order,
                last: Some(life),
            };
        }
        if summaries
            .iter()
            .zip(self.covered_lifecycle)
            .any(|(new, old)| new.count < old.count)
        {
            return Err("lifecycle summary count changed backwards".into());
        }
        let missing_history = !self.history_complete
            || stamp.cursor > self.received_cursor
            || stamp.count > self.received_count;
        if !missing_history && summaries != self.observed_lifecycle {
            return Err("lifecycle summaries invented unobserved history".into());
        }
        if stamp.count != u64::MAX
            && summaries.iter().map(|t| u128::from(t.count)).sum::<u128>() > u128::from(stamp.count)
        {
            return Err("lifecycle counts exceed total notifications".into());
        }
        for (index, summary) in summaries.iter().enumerate() {
            let covered = self.covered_lifecycle[index];
            let observed = self.observed_lifecycle[index];
            if summary.count < observed.count
                || summary.order < observed.order
                || (observed.last.is_some()
                    && summary.order == observed.order
                    && summary.order != u64::MAX
                    && summary.last != observed.last)
                || (summary.count == covered.count
                    && covered.count != u64::MAX
                    && *summary != covered)
                || (summary.count > covered.count
                    && covered.order != u64::MAX
                    && summary.order <= covered.order)
                || (summary.order != 0
                    && summary.order != u64::MAX
                    && summaries[..index]
                        .iter()
                        .any(|other| other.order == summary.order))
            {
                return Err("lifecycle summaries do not cover the observed history".into());
            }
        }
        let changed = self
            .covered_timeline
            .is_some_and(|old| old != stamp.timeline);
        let resetting = self.waiting.is_some()
            || self.reset_pending
            || changed
            || stamp.cursor > self.received_cursor
            || stamp.count > self.received_count
            || generation != self.server_generation;
        // Validate the complete normal batch before changing the publication.
        // A malformed suffix must never expose a partially accepted prefix.
        if !resetting {
            for batch in &self.staged {
                if batch.stamp.timeline != stamp.timeline || batch.stamp.cursor > stamp.cursor {
                    return Err("notification is not covered by negotiated frame".into());
                }
                for event in &batch.events {
                    if matches!(event, BridgeEvent::Sim { key, .. } if snapshot.as_ref().is_none_or(|snapshot| key.tick > snapshot.tick()))
                    {
                        return Err("negotiated event is ahead of its covering snapshot".into());
                    }
                }
            }
        }
        let reset_generation = if stamp.cursor > self.received_cursor
            || stamp.count > self.received_count
            || changed
            || generation != self.server_generation
        {
            Some(
                self.generation
                    .checked_add(1)
                    .ok_or_else(|| "presentation reset generation exhausted".to_string())?,
            )
        } else {
            None
        };
        Ok(PreparedFrame {
            stamp,
            generation,
            summaries,
            snapshot,
            reset_generation,
        })
    }

    /// Commits a frame cut returned by [`Self::prepare_frame`]. There are no
    /// fallible operations here, so a caller can preflight other state first.
    pub fn commit_frame(&mut self, prepared: PreparedFrame) {
        let PreparedFrame {
            stamp,
            generation,
            summaries,
            snapshot,
            reset_generation,
        } = prepared;
        if let Some(generation) = reset_generation {
            self.apply_reset(stamp.cursor, generation);
        }
        self.received_cursor = stamp.cursor;
        self.received_count = stamp.count;
        self.received_timeline = stamp.timeline;
        self.covered_cursor = stamp.cursor;
        self.covered_count = stamp.count;
        self.covered_timeline = Some(stamp.timeline);
        self.covered_lifecycle = summaries;
        self.observed_lifecycle = summaries;
        self.history_complete = true;
        self.server_generation = generation;
        self.snapshot = snapshot;
        if self.waiting.is_some_and(|cursor| cursor <= stamp.cursor) {
            self.waiting = None;
            self.reset_pending = true;
        }
        if self.reset_pending {
            self.staged.clear();
            self.ready.clear();
            self.retained = 0;
            return;
        }
        if self
            .floor
            .is_some_and(|(timeline, _)| timeline != stamp.timeline)
        {
            self.floor = None;
        }
        for batch in self.staged.drain(..) {
            for event in batch.events {
                if matches!(&event, BridgeEvent::Sim { key, .. } if self.floor.is_some_and(|(timeline, tick)| timeline == batch.stamp.timeline && key.tick <= tick))
                {
                    self.retained -= 1;
                } else {
                    self.ready.push(event);
                }
            }
        }
    }

    pub fn frame(&mut self, metadata: &J, snapshot: Option<Snapshot>) -> Result<(), String> {
        let prepared = self.prepare_frame(metadata, snapshot)?;
        self.commit_frame(prepared);
        Ok(())
    }

    pub fn poll(&mut self) -> ViewUpdate<E> {
        let mut events = Vec::new();
        if let Some(started) = self.started.take() {
            events.push(BridgeEvent::Lifecycle(started));
        }
        if self.disconnected && !self.disconnect_reported {
            self.disconnect_reported = true;
            events.push(BridgeEvent::Lifecycle(Lifecycle::Disconnected));
        }
        // A closed stream cannot turn an uncovered discarded suffix into a
        // successful reset. The terminal status above remains observable.
        if self.waiting.is_some() {
            return ViewUpdate {
                snapshot: self.snapshot.clone(),
                events,
                resync: None,
            };
        }
        let resync = if self.reset_pending {
            self.reset_pending = false;
            let mut lifecycle: Vec<_> = self
                .covered_lifecycle
                .iter()
                .enumerate()
                .filter_map(|(i, tally)| {
                    let count = tally.count.saturating_sub(self.consumed_lifecycle[i]);
                    (count > 0).then(|| {
                        (
                            tally.order,
                            LifecycleRecovery {
                                count,
                                last: tally.last.expect("nonzero lifecycle tally"),
                            },
                        )
                    })
                })
                .collect();
            lifecycle.sort_by_key(|(order, _)| *order);
            let head_tick = self.snapshot.as_ref().map_or(0, Snapshot::tick);
            self.floor = self.covered_timeline.map(|timeline| (timeline, head_tick));
            Some(ViewResync {
                generation: self.generation,
                discarded_events: self.covered_count.saturating_sub(self.consumed_count),
                head_tick,
                verified_tick: self.snapshot.as_ref().map_or(0, Snapshot::verified_tick),
                disconnected: self.disconnected,
                last_desync: None,
                lifecycle: lifecycle.into_iter().map(|(_, note)| note).collect(),
            })
        } else {
            self.retained -= self.ready.len();
            events.append(&mut self.ready);
            None
        };
        self.consumed_count = self.covered_count;
        self.consumed_lifecycle = self.covered_lifecycle.map(|tally| tally.count);
        ViewUpdate {
            snapshot: self.snapshot.clone(),
            events,
            resync,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_bridge::{BridgeStats, EventKey, EventStatus, SnapshotParts};
    use orr_ecs::{ComponentRegistryBuilder, Frame};
    use serde_json::json;
    use std::sync::Arc;

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
    fn mailbox(capacity: usize) -> ViewMailbox<u64> {
        let mut m = ViewMailbox::new(capacity);
        m.negotiate(&json!({"subscription":"7", "cursor":"0", "count":"0"}))
            .unwrap();
        m.frame(&cut(1, 0, 0, 1), Some(snapshot(0))).unwrap();
        assert!(m.poll().resync.is_some());
        m
    }
    fn cut(timeline: u64, cursor: u64, count: u64, generation: u64) -> J {
        json!({"subscription":"7", "timeline":timeline.to_string(), "through_cursor":cursor.to_string(), "count":count.to_string(), "loss_generation":generation.to_string(), "lifecycle":[]})
    }
    fn stamp(timeline: u64, cursor: u64, count: u64) -> Stamp {
        Stamp {
            subscription: 7,
            timeline,
            cursor,
            count,
        }
    }
    fn event(tick: u64) -> BridgeEvent<u64> {
        BridgeEvent::Sim {
            key: EventKey::new(tick, 0, 0),
            status: EventStatus::Verified(tick),
        }
    }

    #[test]
    fn no_event_before_a_covering_frame() {
        let mut m = mailbox(2);
        m.stage(stamp(1, 1, 1), vec![event(10)]).unwrap();
        let before = m.poll();
        assert_eq!(before.snapshot.unwrap().tick(), 0);
        assert!(before.events.is_empty());
        m.frame(&cut(1, 1, 1, 1), Some(snapshot(10))).unwrap();
        let after = m.poll();
        assert_eq!(after.snapshot.unwrap().tick(), 10);
        assert_eq!(after.events, vec![event(10)]);
        assert!(after.resync.is_none());
    }

    #[test]
    fn staging_and_render_share_one_budget_and_latest_baseline_is_pinned() {
        let mut m = mailbox(2);
        m.stage(stamp(1, 1, 1), vec![event(1)]).unwrap();
        m.frame(&cut(1, 1, 1, 1), Some(snapshot(1))).unwrap();
        m.stage(stamp(1, 2, 2), vec![event(2)]).unwrap();
        m.stage(stamp(1, 3, 3), vec![event(3)]).unwrap();
        assert!(m.retained <= 2);
        assert!(
            m.poll().resync.is_none(),
            "tick1 cannot cover discarded tick3"
        );
        m.frame(&cut(1, 3, 3, 1), Some(snapshot(3))).unwrap();
        m.stage(stamp(1, 4, 4), vec![event(4)]).unwrap();
        m.frame(&cut(1, 4, 4, 1), Some(snapshot(4))).unwrap();
        let recovered = m.poll();
        assert!(recovered.events.is_empty());
        assert_eq!(recovered.resync.unwrap().discarded_events, 4);
        assert_eq!(recovered.snapshot.as_ref().unwrap().tick(), 4);
        m.stage(stamp(1, 5, 5), vec![event(5)]).unwrap();
        m.frame(&cut(1, 5, 5, 1), Some(snapshot(5))).unwrap();
        assert_eq!(m.poll().events, vec![event(5)]);
        assert_eq!(recovered.snapshot.unwrap().tick(), 4);
    }

    #[test]
    fn late_old_key_is_filtered_only_in_its_timeline() {
        let mut m = mailbox(1);
        m.stage(stamp(1, 1, 2), vec![event(8), event(9)]).unwrap();
        m.frame(&cut(1, 1, 2, 1), Some(snapshot(9))).unwrap();
        assert!(m.poll().resync.is_some());
        m.stage(stamp(1, 2, 3), vec![event(8)]).unwrap();
        m.frame(&cut(1, 2, 3, 1), Some(snapshot(10))).unwrap();
        assert!(m.poll().events.is_empty());
        m.frame(&cut(2, 2, 3, 2), Some(snapshot(0))).unwrap();
        assert!(m.poll().resync.is_some());
        m.stage(stamp(2, 3, 4), vec![event(1)]).unwrap();
        m.frame(&cut(2, 3, 4, 2), Some(snapshot(1))).unwrap();
        assert_eq!(m.poll().events, vec![event(1)]);
    }

    #[test]
    fn gap_and_paused_diagnostics_recover_with_cumulative_summaries() {
        let mut m = mailbox(1);
        m.stage(
            stamp(1, 2, 2),
            vec![BridgeEvent::Lifecycle(Lifecycle::SeekRejected {
                target: 999,
            })],
        )
        .unwrap();
        let mut meta = cut(1, 2, 2, 1);
        meta["lifecycle"] =
            json!([{"count":"2", "order":"2", "note":{"kind":"seek_rejected", "target":999}}]);
        m.frame(&meta, Some(snapshot(0))).unwrap();
        let reset = m.poll().resync.unwrap();
        assert_eq!(reset.head_tick, 0);
        assert_eq!(reset.discarded_events, 2);
        assert_eq!(
            reset.lifecycle,
            vec![LifecycleRecovery {
                count: 2,
                last: Lifecycle::SeekRejected { target: 999 }
            }]
        );
    }

    #[test]
    fn close_during_uncovered_loss_is_observable_without_false_reset() {
        let mut m = mailbox(1);
        m.stage(stamp(1, 1, 2), vec![event(1), event(2)]).unwrap();
        m.disconnect();
        let update = m.poll();
        assert_eq!(update.snapshot.unwrap().tick(), 0);
        assert!(update.resync.is_none());
        assert_eq!(
            update.events,
            vec![BridgeEvent::Lifecycle(Lifecycle::Disconnected)]
        );
        assert!(m.poll().events.is_empty());
    }

    #[test]
    fn inactive_tombstone_clears_snapshot_and_requires_new_identity() {
        let mut m = mailbox(2);
        m.frame(&cut(2, 0, 0, 2), None).unwrap();
        let empty = m.poll();
        assert!(empty.snapshot.is_none());
        assert!(empty.resync.is_some());
        m.frame(&cut(3, 0, 0, 3), Some(snapshot(0))).unwrap();
        assert!(m.poll().resync.is_some());
    }

    #[test]
    fn stale_or_malformed_cuts_are_rejected_and_exact_u64_survives() {
        let max = json!({"subscription":u64::MAX.to_string(),"timeline":u64::MAX.to_string(),"cursor":u64::MAX.to_string(),"count":u64::MAX.to_string()});
        assert_eq!(Stamp::parse(&max, false).unwrap().cursor, u64::MAX);
        assert!(exact(&json!({"count":9007199254740993u64}), "count").is_err());
        let mut m = mailbox(2);
        m.stage(stamp(1, 1, 1), vec![event(1)]).unwrap();
        assert!(m.frame(&cut(1, 0, 0, 1), Some(snapshot(1))).is_err());
        let mut bad = cut(1, 1, 1, 1);
        bad["subscription"] = json!("8");
        assert!(m.frame(&bad, Some(snapshot(1))).is_err());
    }

    #[test]
    fn rejected_frame_preflight_preserves_staged_mailbox_state() {
        let mut m = mailbox(2);
        m.stage(stamp(1, 1, 1), vec![event(1)]).unwrap();
        let mut stale = cut(1, 0, 0, 1);
        stale["lifecycle"] = json!([]);
        assert!(m.prepare_frame(&stale, Some(snapshot(0))).is_err());
        assert_eq!(m.received_cursor, 1);
        assert_eq!(m.staged.len(), 1);
        assert_eq!(m.snapshot.as_ref().unwrap().tick(), 0);
        m.frame(&cut(1, 1, 1, 1), Some(snapshot(1))).unwrap();
        assert_eq!(m.poll().events, vec![event(1)]);
    }

    #[test]
    fn renewal_discards_old_events_but_keeps_display_until_new_full() {
        let mut m = mailbox(2);
        m.stage(stamp(1, 1, 1), vec![event(1)]).unwrap();
        assert_eq!(m.poll().snapshot.unwrap().tick(), 0);
        m.renew_subscription(&json!({"subscription":"8", "cursor":"4", "count":"5"}))
            .unwrap();
        let waiting = m.poll();
        assert_eq!(waiting.snapshot.unwrap().tick(), 0);
        assert!(waiting.events.is_empty());
        assert!(waiting.resync.is_none());

        let mut old = cut(1, 4, 5, 1);
        old["lifecycle"] = json!([]);
        assert!(m.prepare_frame(&old, Some(snapshot(1))).is_err());
        let mut first = cut(1, 4, 5, 0);
        first["subscription"] = json!("8");
        let prepared = m.prepare_frame(&first, Some(snapshot(10))).unwrap();
        m.commit_frame(prepared);
        let update = m.poll();
        assert_eq!(update.snapshot.unwrap().tick(), 10);
        assert!(update.events.is_empty());
        assert!(update.resync.is_some());
    }

    #[test]
    fn renewal_generation_overflow_is_rejected_without_state_change() {
        let mut m = mailbox(2);
        m.generation = u64::MAX;
        let snapshot_tick = m.snapshot.as_ref().unwrap().tick();
        assert!(m
            .renew_subscription(&json!({"subscription":"8", "cursor":"4", "count":"5"}))
            .is_err());
        assert_eq!(m.subscription, Some(7));
        assert_eq!(m.snapshot.as_ref().unwrap().tick(), snapshot_tick);
    }

    #[test]
    fn permanently_slow_consumer_retains_a_fixed_budget_as_producer_advances() {
        let mut m = mailbox(2);
        for tick in 1..=10_000 {
            m.stage(stamp(1, tick, tick), vec![event(tick)]).unwrap();
            m.frame(&cut(1, tick, tick, 1), Some(snapshot(tick)))
                .unwrap();
            assert!(m.retained <= 2);
            assert!(m.staged.len() <= 2);
        }
        let update = m.poll();
        assert_eq!(update.snapshot.unwrap().tick(), 10_000);
        assert_eq!(update.resync.unwrap().discarded_events, 10_000);
        assert!(update.events.is_empty());
    }
    #[test]
    fn regressing_frame_cannot_leave_ready_events_ahead_of_the_snapshot() {
        let mut m = mailbox(2);
        m.stage(stamp(1, 1, 1), vec![event(1)]).unwrap();
        m.frame(&cut(1, 1, 1, 1), Some(snapshot(1))).unwrap();
        assert!(m.frame(&cut(1, 1, 1, 1), Some(snapshot(0))).is_err());
        let update = m.poll();
        assert_eq!(update.snapshot.unwrap().tick(), 1);
        assert_eq!(update.events, vec![event(1)]);
    }

    #[test]
    fn oversized_batch_keeps_fixed_size_tick_evidence_until_covered() {
        let mut m = mailbox(1);
        m.stage(stamp(1, 1, 2), vec![event(99), event(100)])
            .unwrap();
        assert_eq!(m.retained, 0);
        assert!(m.frame(&cut(1, 1, 2, 1), Some(snapshot(1))).is_err());
        assert!(m.poll().resync.is_none());
        m.frame(&cut(1, 1, 2, 1), Some(snapshot(100))).unwrap();
        assert_eq!(m.poll().resync.unwrap().head_tick, 100);
    }
    #[test]
    fn inconsistent_or_missing_lifecycle_summaries_are_rejected() {
        let mut m = mailbox(1);
        m.stage(
            stamp(1, 1, 1),
            vec![BridgeEvent::Lifecycle(Lifecycle::SeekRejected {
                target: 9,
            })],
        )
        .unwrap();
        assert!(m.frame(&cut(1, 1, 1, 1), Some(snapshot(0))).is_err());
        let mut duplicate = cut(1, 1, 1, 1);
        duplicate["lifecycle"] = json!([
            {"count":"1","order":"1","note":{"kind":"seek_rejected","target":9}},
            {"count":"1","order":"1","note":{"kind":"debug_rejected","error":"unsupported"}}
        ]);
        assert!(m.frame(&duplicate, Some(snapshot(0))).is_err());
        let mut good = cut(1, 1, 1, 1);
        good["lifecycle"] =
            json!([{ "count":"1", "order":"1", "note":{"kind":"seek_rejected","target":9} }]);
        m.frame(&good, Some(snapshot(0))).unwrap();
        let mut rewritten = good.clone();
        rewritten["lifecycle"][0]["note"]["target"] = json!(10);
        assert!(m.frame(&rewritten, Some(snapshot(0))).is_err());
    }
    #[test]
    fn same_tick_payload_change_requires_new_timeline() {
        let mut m = mailbox(1);
        let mut frame = Frame::new(ComponentRegistryBuilder::new().build());
        frame.spawn();
        let frame = Arc::new(frame);
        let changed = Snapshot::from_parts(SnapshotParts {
            seq: 2,
            tick: 0,
            verified_tick: 0,
            tick_rate: 60,
            predicted: frame.clone(),
            predicted_prev: None,
            verified: Some(frame),
            stats: BridgeStats::default(),
            last_rollback: None,
            timeline: None,
        });
        assert!(m.frame(&cut(1, 0, 0, 1), Some(changed.clone())).is_err());
        m.frame(&cut(2, 0, 0, 2), Some(changed)).unwrap();
        assert!(m.poll().resync.is_some());
    }
    #[test]
    fn complete_notification_history_cannot_invent_lifecycle_diagnostics() {
        let mut m = mailbox(1);
        m.stage(stamp(1, 1, 1), vec![event(1)]).unwrap();
        let mut forged = cut(1, 1, 1, 2);
        forged["lifecycle"] =
            json!([{ "count":"1", "order":"1", "note":{"kind":"seek_rejected","target":9} }]);
        assert!(m.frame(&forged, Some(snapshot(1))).is_err());
        let mut impossible = cut(1, 2, 100, 2);
        impossible["lifecycle"] =
            json!([{ "count":"100", "order":"1", "note":{"kind":"seek_rejected","target":9} }]);
        assert!(m.frame(&impossible, Some(snapshot(1))).is_err());
    }
}
