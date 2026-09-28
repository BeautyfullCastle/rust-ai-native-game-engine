use bytemuck::{Pod, Zeroable};

/// A deterministic identity for one event occurrence: which tick, which
/// system (by registration index) emitted it, and its sequence number
/// among events emitted by *that system on that tick*.
///
/// Because system order and per-tick behavior are deterministic, replaying
/// the same tick with the same inputs always produces the same sequence of
/// `EventKey`s, so `orr_session` can match a re-emitted event after a
/// rollback resimulation against the one it announced before, rather than
/// treating it as new.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Pod, Zeroable)]
pub struct EventKey {
    pub tick: u64,
    pub system_index: u16,
    /// Padding to keep the struct's layout stable/portable; always zero.
    pub _pad: u16,
    pub seq: u32,
}

impl EventKey {
    pub fn new(tick: u64, system_index: u16, seq: u32) -> Self {
        Self { tick, system_index, _pad: 0, seq }
    }
}

/// One event emitted by a system during a tick, with its deterministic key.
#[derive(Clone, Copy, Debug)]
pub struct SimEvent<E> {
    pub key: EventKey,
    pub payload: E,
}
