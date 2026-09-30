use orr_sim::EventKey;

/// What happened to one event this [`crate::Session::advance`] call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventStatus<E> {
    /// Emitted during a predicted (not-yet-verified) tick. May still be
    /// [`EventStatus::Canceled`] by a later rollback; view-layer reactions
    /// to it must be reversible (sounds, VFX — not achievements/analytics).
    Predicted(E),
    /// The tick that emitted this event is now verified: it will never be
    /// rolled back. Announced exactly once per event.
    Verified(E),
    /// A previously `Predicted` event did not reappear (or reappeared with
    /// a different payload) after a rollback resimulation and must be
    /// treated as if it never happened.
    Canceled,
}

/// One [`crate::Session::advance`] call's worth of event notifications for
/// the view layer, in a deterministic order (resimulated ticks first, in
/// tick order; then the freshly simulated tick, if any; verified
/// announcements last).
#[derive(Clone, Debug, Default)]
pub struct EventBatch<E> {
    items: Vec<(EventKey, EventStatus<E>)>,
}

impl<E> EventBatch<E> {
    pub(crate) fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub(crate) fn push(&mut self, key: EventKey, status: EventStatus<E>) {
        self.items.push((key, status));
    }

    /// An empty batch.
    pub fn empty() -> Self {
        Self { items: Vec::new() }
    }

    /// Adds every item of `other` after this batch's own, keeping order.
    pub fn append(&mut self, other: EventBatch<E>) {
        self.items.extend(other.items);
    }

    pub fn iter(&self) -> impl Iterator<Item = &(EventKey, EventStatus<E>)> {
        self.items.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn into_vec(self) -> Vec<(EventKey, EventStatus<E>)> {
        self.items
    }
}
