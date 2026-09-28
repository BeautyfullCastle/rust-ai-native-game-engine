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
/// one) is a [`System`] whose name is its item path.
impl<G: Game> System<G> for fn(&mut SimContext<G>) {
    fn name(&self) -> &'static str {
        core::any::type_name::<Self>()
    }
    fn run(&mut self, ctx: &mut SimContext<G>) {
        (self)(ctx)
    }
}
