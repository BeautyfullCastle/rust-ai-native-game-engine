//! Glue between the physics step and `orr_sim`'s system pipeline and
//! event stream.

use orr_sim::{Game, SimContext, System};

use crate::step::{step, Scratch};
use crate::types::TriggerEvent;

/// A [`System`] that runs one physics step per tick and emits each
/// trigger enter/exit as a game event through [`SimContext::emit`].
///
/// The game supplies `map` to convert a [`TriggerEvent`] into its own
/// `Game::Event` type. Events use the usual `EventKey`
/// (tick, system index, sequence), so `orr_session` reconciles them across
/// rollbacks like any other event: after a rollback the overlap set is
/// restored from the frame and the same events are produced again.
pub struct PhysicsSystem<G: Game> {
    scratch: Scratch,
    events: Vec<TriggerEvent>,
    map: fn(TriggerEvent) -> G::Event,
}

impl<G: Game> PhysicsSystem<G> {
    /// Creates the system. `map` converts trigger events to game events.
    pub fn new(map: fn(TriggerEvent) -> G::Event) -> Self {
        PhysicsSystem { scratch: Scratch::new(), events: Vec::new(), map }
    }
}

impl<G: Game> System<G> for PhysicsSystem<G> {
    fn name(&self) -> &'static str {
        "PhysicsSystem"
    }

    fn run(&mut self, ctx: &mut SimContext<G>) {
        self.events.clear();
        step(ctx.frame, &mut self.scratch, &mut self.events);
        for ev in self.events.drain(..) {
            ctx.emit((self.map)(ev));
        }
    }
}
