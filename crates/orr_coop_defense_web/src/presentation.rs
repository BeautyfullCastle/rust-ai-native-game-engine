//! Storage adapter for the local, verified-only WASM ABI, not another event bus.
//! The existing engine SimEvent/EventKey is preserved. Network/rollback clients
//! use orr_session + orr_bridge instead of this latest-tick adapter.
use orr_sim::SimEvent;

pub const MAX_PRESENTATION_EVENTS: usize = 1024;

pub(crate) struct LatestTickEvents<E> {
    pub events: Vec<SimEvent<E>>,
    pub dropped: usize,
}
impl<E> LatestTickEvents<E> {
    pub fn new() -> Self {
        Self {
            events: Vec::with_capacity(MAX_PRESENTATION_EVENTS),
            dropped: 0,
        }
    }
    /// Replace, never append across ticks. Oversize batches are discarded in
    /// full: rendering a partial hit/death sequence would claim false causality.
    pub fn replace(&mut self, events: Vec<SimEvent<E>>) {
        self.events.clear();
        self.dropped = if events.len() > MAX_PRESENTATION_EVENTS {
            events.len()
        } else {
            0
        };
        if self.dropped == 0 {
            self.events.extend(events);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_sim::EventKey;
    fn events(count: usize) -> Vec<SimEvent<u32>> {
        (0..count)
            .map(|seq| SimEvent {
                key: EventKey::new(3, 0, seq as u32),
                payload: seq as u32,
            })
            .collect()
    }
    #[test]
    fn exact_capacity_is_retained_and_overflow_requests_full_resync() {
        let mut batch = LatestTickEvents::new();
        batch.replace(events(MAX_PRESENTATION_EVENTS));
        assert_eq!(batch.events.len(), MAX_PRESENTATION_EVENTS);
        assert_eq!(batch.dropped, 0);
        batch.replace(events(MAX_PRESENTATION_EVENTS + 1));
        assert!(batch.events.is_empty());
        assert_eq!(batch.dropped, MAX_PRESENTATION_EVENTS + 1);
        assert_eq!(batch.events.capacity(), MAX_PRESENTATION_EVENTS);
        batch.replace(events(1));
        assert_eq!(batch.dropped, 0);
        assert_eq!(batch.events.len(), 1);
    }
    #[test]
    fn empty_next_tick_does_not_replay_previous_effects() {
        let mut batch = LatestTickEvents::new();
        batch.replace(events(2));
        batch.replace(Vec::new());
        assert!(batch.events.is_empty());
    }
}
