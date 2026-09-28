use orr_ecs::{Commands, Frame};
use orr_fp::FrameRng;

use crate::event::{EventKey, SimEvent};
use crate::game::Game;
use crate::input::TickInputs;

/// Everything one [`crate::System::run`] call gets: mutable access to the
/// `Frame`, this tick's inputs, a deterministic RNG stream, a deferred
/// [`Commands`] buffer for structural changes, and [`SimContext::emit`] for
/// sim-to-view events.
pub struct SimContext<'a, G: Game> {
    pub frame: &'a mut Frame,
    pub inputs: &'a TickInputs<G::Input, G::Command>,
    pub tick: u64,
    pub(crate) system_index: u16,
    pub(crate) seq: u32,
    pub(crate) events: &'a mut Vec<SimEvent<G::Event>>,
    pub(crate) commands: &'a mut Commands,
}

impl<'a, G: Game> SimContext<'a, G> {
    /// The frame-local deterministic RNG stream. Stored *inside* the
    /// `Frame` (as a singleton), so it snapshots and rolls back with
    /// everything else — two resimulations of the same tick range always
    /// draw the same random numbers.
    pub fn rng(&mut self) -> &mut FrameRng {
        self.frame.singleton_mut::<FrameRng>()
    }

    /// This tick's deferred structural-change buffer. Applied to the frame
    /// right after this system returns, in recorded order, so spawns from
    /// this system are visible to the *next* system this tick.
    pub fn commands(&mut self) -> &mut Commands {
        self.commands
    }

    /// Emits a sim-to-view event with a deterministic [`EventKey`]
    /// (`tick`, this system's registration index, and a per-system,
    /// per-tick sequence number).
    pub fn emit(&mut self, payload: G::Event) {
        let key = EventKey::new(self.tick, self.system_index, self.seq);
        self.seq += 1;
        self.events.push(SimEvent { key, payload });
    }
}
