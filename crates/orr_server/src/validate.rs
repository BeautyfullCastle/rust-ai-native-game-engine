//! The Relay + Validate hook: the server checks every input before it can
//! become part of a confirmed tick. The server does not simulate, so the
//! check only sees bytes (input range, rate, command legality is up to the
//! game's implementation of [`InputValidator`]).

/// What the server knows about one submitted input.
pub struct InputCtx<'a> {
    pub room: u64,
    pub slot: u8,
    /// The tick the input is for.
    pub tick: u64,
    /// Last tick the server finalized.
    pub finalized: u64,
    /// The slot's most recent accepted input (the room's default input
    /// before the first).
    pub last_input: &'a [u8],
    /// Server time of arrival (microseconds), for rate limits.
    pub now_us: u64,
}

/// The answer of an [`InputValidator`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Accept,
    /// Keep the input, throw away its commands.
    DropCommands,
    /// Treat the input as if it never arrived (the tick is repeated).
    Reject,
}

/// Input range, rate and command legality checks.
pub trait InputValidator {
    /// Called once per input entry, the first time the server sees the
    /// (slot, tick) pair. `commands` are encoded `SimCommand`s.
    fn validate(&mut self, ctx: &InputCtx<'_>, input: &[u8], commands: &[Vec<u8>]) -> Verdict;
}

/// The default: accept everything (plain Relay mode).
#[derive(Clone, Copy, Debug, Default)]
pub struct AcceptAll;

impl InputValidator for AcceptAll {
    fn validate(&mut self, _ctx: &InputCtx<'_>, _input: &[u8], _commands: &[Vec<u8>]) -> Verdict {
        Verdict::Accept
    }
}
