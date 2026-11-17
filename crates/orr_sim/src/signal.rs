//! Player signals: "a player joined / dropped / came back / left", carried in
//! the deterministic input of a tick (see [`crate::TickInputs::signals`]).
//!
//! A signal is part of the recorded input. Every peer receives the same
//! signals for the same tick, so applying them keeps the simulation
//! deterministic. A game that does not implement [`Players`] never sees them
//! (the signals stay readable through `TickInputs::signals`).

use core::marker::PhantomData;

use bytemuck::Pod;
use orr_ecs::Frame;

use crate::context::SimContext;
use crate::event::EventKey;
use crate::game::Game;
use crate::input::PlayerSlot;
use crate::system::System;

/// Which of the four signals a [`RawPlayerSignal`] is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum SignalKind {
    /// A player entered. Carries the player's start data.
    Added = 0,
    /// The server judged the player gone for now. The slot is kept.
    Disconnected = 1,
    /// The player came back to the same slot.
    Reconnected = 2,
    /// The slot is given up for good.
    Removed = 3,
}

impl SignalKind {
    /// The byte used in recordings and on the wire.
    pub const fn to_byte(self) -> u8 {
        self as u8
    }

    /// `None` for a byte that is no signal kind.
    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Added),
            1 => Some(Self::Disconnected),
            2 => Some(Self::Reconnected),
            3 => Some(Self::Removed),
            _ => None,
        }
    }
}

/// One signal in its stored form: the kind, the slot, and the start data as
/// bytes (only `Added` has data). It does not depend on the game's player
/// type, so sessions, recordings and the view bridge can carry it as is.
/// [`RawPlayerSignal::typed`] decodes it for a [`Players`] game.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RawPlayerSignal {
    pub kind: SignalKind,
    pub slot: PlayerSlot,
    /// The bytes of `Players::Player` for `Added`. Empty means "the default
    /// value". Always empty for the other kinds.
    pub data: Vec<u8>,
}

impl RawPlayerSignal {
    /// A new player with the default start data.
    pub fn added(slot: PlayerSlot) -> Self {
        Self { kind: SignalKind::Added, slot, data: Vec::new() }
    }

    /// A new player with start data.
    pub fn added_with<P: Pod>(slot: PlayerSlot, data: &P) -> Self {
        Self { kind: SignalKind::Added, slot, data: bytemuck::bytes_of(data).to_vec() }
    }

    pub fn disconnected(slot: PlayerSlot) -> Self {
        Self { kind: SignalKind::Disconnected, slot, data: Vec::new() }
    }

    pub fn reconnected(slot: PlayerSlot) -> Self {
        Self { kind: SignalKind::Reconnected, slot, data: Vec::new() }
    }

    pub fn removed(slot: PlayerSlot) -> Self {
        Self { kind: SignalKind::Removed, slot, data: Vec::new() }
    }

    /// Decodes the start data as `P`. Empty data, or data of another size
    /// than `P`, gives `P::default()`. The result depends only on the bytes,
    /// so every peer decodes the same value.
    pub fn typed<P: Pod + Default>(&self) -> PlayerSignal<P> {
        match self.kind {
            SignalKind::Added => {
                let data = bytemuck::try_pod_read_unaligned(&self.data).unwrap_or_default();
                PlayerSignal::Added { slot: self.slot, data }
            }
            SignalKind::Disconnected => PlayerSignal::Disconnected { slot: self.slot },
            SignalKind::Reconnected => PlayerSignal::Reconnected { slot: self.slot },
            SignalKind::Removed => PlayerSignal::Removed { slot: self.slot },
        }
    }
}

/// A player signal with typed start data.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlayerSignal<P> {
    /// A player entered (first time).
    Added { slot: PlayerSlot, data: P },
    /// The server judged the player gone. Examples: stop the avatar, let an AI steer.
    Disconnected { slot: PlayerSlot },
    /// The player came back to the same slot. Example: give control back.
    Reconnected { slot: PlayerSlot },
    /// The slot is free for good. Examples: delete the avatar, hand over its items.
    Removed { slot: PlayerSlot },
}

impl<P> PlayerSignal<P> {
    pub fn slot(&self) -> PlayerSlot {
        match self {
            Self::Added { slot, .. }
            | Self::Disconnected { slot }
            | Self::Reconnected { slot }
            | Self::Removed { slot } => *slot,
        }
    }
}

/// A game that reacts to player signals. Opt in with one trait impl and one
/// entry in [`Game::systems`]: put [`player_signals`] first.
///
/// ```ignore
/// fn systems() -> Vec<Box<dyn System<Self>>> {
///     let mut systems = vec![player_signals::<Self>()];
///     systems.extend(fn_systems([move_paddles]));
///     systems
/// }
/// ```
///
/// Existing `Game` impls stay as they are. Without the system, signals are
/// ignored by the simulation.
pub trait Players: Game {
    /// The start data of one player, chosen when the player enters (a name,
    /// a character, a color). Fixed size, so it can ride in a signal.
    type Player: Pod + Default;

    /// Called once per signal of the tick, before the other systems run,
    /// in slot order. It changes the frame like any system. It must depend
    /// only on the frame and the signal.
    fn on_player(frame: &mut Frame, signal: PlayerSignal<Self::Player>);
}

/// The system that feeds a tick's signals to [`Players::on_player`].
/// Place it first in [`Game::systems`].
pub fn player_signals<G: Players>() -> Box<dyn System<G>> {
    Box::new(PlayerSignalSystem::<G>(PhantomData))
}

struct PlayerSignalSystem<G>(PhantomData<fn() -> G>);

impl<G: Players> System<G> for PlayerSignalSystem<G> {
    fn name(&self) -> &'static str {
        "orr_sim::player_signals"
    }

    fn run(&mut self, ctx: &mut SimContext<G>) {
        let inputs = ctx.inputs;
        for signal in inputs.signals() {
            G::on_player(ctx.frame, signal.typed::<G::Player>());
        }
    }
}

/// The `system_index` that the [`EventKey`] of a signal note carries. No
/// system has this index, so a key of a signal never equals a key of a game event.
pub const SIGNAL_SYSTEM_INDEX: u16 = u16::MAX;

/// The identity of a signal: its tick, its slot, and its place among the
/// signals of that slot in that tick (`ordinal`, 0 for the first). It is the
/// same in every resimulation, so a view can match a `Verified` or `Canceled`
/// note to its `Predicted` one. A signal added later for another slot does
/// not change the keys of the signals that are already there.
pub fn signal_key(tick: u64, slot: PlayerSlot, ordinal: u16) -> EventKey {
    EventKey::new(tick, SIGNAL_SYSTEM_INDEX, (u32::from(slot.0) << 16) | u32::from(ordinal))
}

/// Pairs each signal of one tick (in canonical order, see [`insert_signal`])
/// with its [`signal_key`].
pub fn keyed_signals(tick: u64, signals: &[RawPlayerSignal]) -> Vec<(EventKey, &RawPlayerSignal)> {
    let mut ordinal = 0u16;
    let mut previous: Option<PlayerSlot> = None;
    signals
        .iter()
        .map(|signal| {
            ordinal = if previous == Some(signal.slot) { ordinal.saturating_add(1) } else { 0 };
            previous = Some(signal.slot);
            (signal_key(tick, signal.slot, ordinal), signal)
        })
        .collect()
}

/// Adds `signal` to `list` in the canonical order: by slot, and in the order
/// of insertion among signals of one slot. Peers that insert the same signals
/// of a slot in the same order (the relay gives one order) get the same list.
pub fn insert_signal(list: &mut Vec<RawPlayerSignal>, signal: RawPlayerSignal) {
    let at = list.partition_point(|s| s.slot <= signal.slot);
    list.insert(at, signal);
}
