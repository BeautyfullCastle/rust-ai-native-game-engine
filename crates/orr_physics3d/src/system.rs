//! Glue between the physics step and `orr_sim`'s system pipeline.

use core::marker::PhantomData;

use orr_sim::{Game, SimContext, System};

use crate::step::{step, Scratch};

/// A [`System`] that runs one 3D physics step per tick.
pub struct PhysicsSystem<G: Game> {
    scratch: Scratch,
    _game: PhantomData<fn(&G)>,
}

impl<G: Game> PhysicsSystem<G> {
    /// Creates the system.
    pub fn new() -> Self {
        PhysicsSystem { scratch: Scratch::new(), _game: PhantomData }
    }
}

impl<G: Game> Default for PhysicsSystem<G> {
    fn default() -> Self {
        Self::new()
    }
}

impl<G: Game> System<G> for PhysicsSystem<G> {
    fn name(&self) -> &'static str {
        "PhysicsSystem3d"
    }

    fn run(&mut self, ctx: &mut SimContext<G>) {
        step(ctx.frame, &mut self.scratch);
    }
}
