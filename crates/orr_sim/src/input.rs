use bytemuck::Pod;

/// A fixed-size, per-player input sample for one tick.
///
/// `Pod` gives a stable byte layout (needed for replay delta-compression and
/// network transport); `Default` is the "no input" / "nothing pressed"
/// value; `Eq` lets a [`crate::Simulation`]/`orr_session::Session` detect
/// when a newly-confirmed input differs from what was predicted.
pub trait SimInput: Pod + Default + Eq + Send + Sync + 'static {}
impl<T: Pod + Default + Eq + Send + Sync + 'static> SimInput for T {}

/// Index of a player's input slot within a [`TickInputs`]. Stable for the
/// lifetime of a session (players don't get renumbered on disconnect).
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PlayerSlot(pub u8);

/// Per-player metadata that rides alongside an input sample.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PlayerFlags {
    /// This tick's input for this player was *not* confirmed yet; it is a
    /// prediction (repeat of the last confirmed input), liable to be
    /// corrected by a rollback once the real input arrives.
    pub predicted: bool,
    /// This player is currently disconnected. Systems may choose to ignore
    /// a disconnected player's input entirely rather than treat `predicted`
    /// repeats as real intent.
    pub disconnected: bool,
}

/// The full set of inputs (and one-off commands) for every player, for one
/// simulation tick. Built by [`crate::Simulation::step`]'s caller
/// (`orr_session::Session` in the networked/predicted case, or directly by
/// test code / a headless runner).
#[derive(Clone)]
pub struct TickInputs<I: SimInput, C: Clone> {
    tick: u64,
    inputs: Vec<I>,
    flags: Vec<PlayerFlags>,
    commands: Vec<(PlayerSlot, C)>,
}

impl<I: SimInput, C: Clone> TickInputs<I, C> {
    /// A `TickInputs` for `player_count` players, all with default
    /// (zero/neutral) input and no flags set.
    pub fn new(tick: u64, player_count: u8) -> Self {
        Self {
            tick,
            inputs: (0..player_count).map(|_| I::default()).collect(),
            flags: vec![PlayerFlags::default(); player_count as usize],
            commands: Vec::new(),
        }
    }

    /// The tick number these inputs apply to.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub fn player_count(&self) -> u8 {
        self.inputs.len() as u8
    }

    pub fn input(&self, slot: PlayerSlot) -> &I {
        &self.inputs[slot.0 as usize]
    }

    pub fn set_input(&mut self, slot: PlayerSlot, input: I) {
        self.inputs[slot.0 as usize] = input;
    }

    pub fn flags(&self, slot: PlayerSlot) -> PlayerFlags {
        self.flags[slot.0 as usize]
    }

    pub fn set_flags(&mut self, slot: PlayerSlot, flags: PlayerFlags) {
        self.flags[slot.0 as usize] = flags;
    }

    /// Every one-off command submitted for this tick, in the deterministic
    /// order they were pushed (which must itself be produced
    /// deterministically by the caller, e.g. sorted by `PlayerSlot` then
    /// submission order, so replays reproduce the exact same order).
    pub fn commands(&self) -> &[(PlayerSlot, C)] {
        &self.commands
    }

    pub fn push_command(&mut self, slot: PlayerSlot, command: C) {
        self.commands.push((slot, command));
    }

    pub fn set_commands(&mut self, commands: Vec<(PlayerSlot, C)>) {
        self.commands = commands;
    }
}
