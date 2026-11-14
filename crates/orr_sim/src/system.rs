use crate::context::SimContext;
use crate::game::Game;

/// One step of a [`crate::Simulation`]'s fixed, ordered pipeline.
///
/// Systems run strictly in registration order today
/// (`Simulation::step` — see its doc comment for the parallel-execution
/// hook this leaves for later: systems whose declared/inferred component
/// access sets don't overlap could run concurrently and still produce a
/// bit-identical result, per the design doc's determinism rules, but M0
/// keeps the executor single-threaded and sequential for simplicity).
pub trait System<G: Game>: Send {
    /// A stable name for diagnostics/profiling (NET panel, schedule view).
    fn name(&self) -> &'static str;
    fn run(&mut self, ctx: &mut SimContext<G>);
}

/// Any plain `fn(&mut SimContext<G>)` (or non-capturing closure coercible to
/// one) is a [`System`]. Its name is the function pointer type, so every
/// function system reports the same name; implement [`System`] on a type
/// when a distinct diagnostic name matters.
impl<G: Game> System<G> for fn(&mut SimContext<G>) {
    fn name(&self) -> &'static str {
        core::any::type_name::<Self>()
    }
    fn run(&mut self, ctx: &mut SimContext<G>) {
        (self)(ctx)
    }
}

/// Builds a [`crate::Game::systems`] pipeline from plain functions, run in
/// array order.
///
/// ```
/// # use orr_sim::{fn_systems, Game, SimContext, System};
/// # struct Demo;
/// # impl Game for Demo {
/// #     type Input = u32; type Command = (); type Event = (); type Config = ();
/// #     fn register(_: &mut orr_sim::ComponentRegistryBuilder) {}
/// #     fn setup(_: &mut orr_sim::Frame, _: &()) {}
/// fn systems() -> Vec<Box<dyn System<Self>>> {
///     fn_systems([gravity, bounce])
/// }
/// # }
/// # fn gravity(_: &mut SimContext<Demo>) {}
/// # fn bounce(_: &mut SimContext<Demo>) {}
/// ```
pub fn fn_systems<G: Game, const N: usize>(
    systems: [fn(&mut SimContext<G>); N],
) -> Vec<Box<dyn System<G>>> {
    systems
        .into_iter()
        .map(|system| Box::new(system) as Box<dyn System<G>>)
        .collect()
}
