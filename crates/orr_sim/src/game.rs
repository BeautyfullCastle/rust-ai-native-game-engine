use orr_ecs::{ComponentRegistryBuilder, Frame};

use crate::command::SimCommand;
use crate::input::SimInput;
use crate::system::System;

/// Bundles everything a [`crate::Simulation`] needs to know about one game:
/// its input/command/event types, how to register its component types, how
/// to build the initial `Frame`, and its fixed system pipeline.
pub trait Game: Sized + Send + 'static {
    /// Per-player, per-tick input sample.
    type Input: SimInput;
    /// One-off command payload.
    type Command: SimCommand + Clone;
    /// Sim-to-view event payload. `Pod` + `PartialEq` so
    /// `orr_session`'s event reconciliation can compare a re-emitted
    /// event's payload bytes against what it announced before a rollback.
    type Event: bytemuck::Pod + PartialEq + Send + Sync + 'static;
    /// Arguments to [`Game::setup`] (scene/level parameters, starting
    /// config). `()` if the game has none.
    type Config: Send + 'static;

    /// Registers every component, singleton and [`orr_ecs::FrameList`]
    /// element type this game's systems use. Called once per
    /// [`crate::Simulation::new`], *after* the engine has already
    /// registered its own singletons (e.g. the frame RNG) — do not
    /// register a type the engine owns.
    fn register(builder: &mut ComponentRegistryBuilder);

    /// Populates the freshly built, empty `Frame` with this game's initial
    /// entities/singletons (spawns from a scene, sets starting singleton
    /// values, etc).
    fn setup(frame: &mut Frame, config: &Self::Config);

    /// The fixed, ordered system pipeline. Called once per
    /// [`crate::Simulation::new`]; order is significant and fixed for the
    /// simulation's lifetime.
    fn systems() -> Vec<Box<dyn System<Self>>>;
}
