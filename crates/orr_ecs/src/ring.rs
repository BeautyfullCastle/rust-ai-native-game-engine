use std::sync::Arc;

use crate::frame::Frame;
use crate::registry::ComponentRegistry;

/// Fixed-capacity ring buffer of [`Frame`] snapshots, one per tick, used to
/// hold recent history for rollback resimulation. Slots are reused by
/// `tick % capacity`; `store`/`restore_into` go through [`Frame::copy_from`],
/// so allocations are reused after the ring has warmed up (no further heap
/// activity in steady state, assuming stable entity/component counts).
pub struct FrameRing {
    frames: Vec<Frame>,
    slot_tick: Vec<Option<u64>>,
}

impl FrameRing {
    pub fn new(capacity: u32, registry: Arc<ComponentRegistry>) -> Self {
        assert!(capacity > 0, "orr_ecs: FrameRing capacity must be non-zero");
        let frames = (0..capacity).map(|_| Frame::new(registry.clone())).collect();
        Self { frames, slot_tick: vec![None; capacity as usize] }
    }

    pub fn capacity(&self) -> u32 {
        self.frames.len() as u32
    }

    fn slot_of(&self, tick: u64) -> usize {
        (tick % self.frames.len() as u64) as usize
    }

    /// Snapshots `frame` into the ring slot for its current tick.
    pub fn store(&mut self, frame: &Frame) {
        let slot = self.slot_of(frame.tick());
        self.frames[slot].copy_from(frame);
        self.slot_tick[slot] = Some(frame.tick());
    }

    /// Returns the stored snapshot for `tick`, if that tick is still the one
    /// occupying its slot (i.e. hasn't been overwritten by a later tick).
    pub fn get(&self, tick: u64) -> Option<&Frame> {
        let slot = self.slot_of(tick);
        if self.slot_tick[slot] == Some(tick) {
            Some(&self.frames[slot])
        } else {
            None
        }
    }

    /// Restores the snapshot for `tick` into `target`, reusing `target`'s
    /// allocations. Returns whether `tick` was available.
    pub fn restore_into(&self, tick: u64, target: &mut Frame) -> bool {
        match self.get(tick) {
            Some(f) => {
                target.copy_from(f);
                true
            }
            None => false,
        }
    }
}
