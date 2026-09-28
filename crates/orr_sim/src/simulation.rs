use std::sync::Arc;

use orr_ecs::{Commands, ComponentRegistry, ComponentRegistryBuilder, Frame};
use orr_fp::FrameRng;

use crate::context::SimContext;
use crate::event::SimEvent;
use crate::game::Game;
use crate::hotpatch::invoke_system;
use crate::input::TickInputs;
use crate::system::System;

/// Owns one game's registry, current `Frame`, fixed system list and tick
/// rate, and advances it one tick at a time.
///
/// `Frame(t+1) = step(Frame(t), Inputs(t), Commands(t))` — this is the
/// entire simulation contract. The same `Simulation<G>` runs identically on
/// a client, a headless authoritative server, in a replay verifier, and in
/// CI, given the same tick rate, seed and input sequence.
pub struct Simulation<G: Game> {
    registry: Arc<ComponentRegistry>,
    frame: Frame,
    systems: Vec<Box<dyn System<G>>>,
    tick_rate: u32,
    commands: Commands,
    /// User-provided build identity (e.g. a hash of the game binary/DLL, a
    /// git commit, or a content-addressed build id). Combined with
    /// `patch_generation` by [`Simulation::build_hash`]. Defaults to `0`
    /// (meaning: "not tracking a build id" — see
    /// [`Simulation::build_hash`]'s doc for what that implies for replay
    /// verification).
    build_id: u64,
    /// Incremented once per applied hot patch (see
    /// [`Simulation::on_patch_applied`]). Folded into
    /// [`Simulation::build_hash`] so a peer or replay verifier that hasn't
    /// seen the same patches never silently compares state against a
    /// simulation that has.
    patch_generation: u32,
}

/// A fixed, deterministic 64-bit mix (SplitMix64's finalizer, integer-only)
/// used by [`build_hash_of`] to combine `build_id` and `patch_generation`.
/// Not a general-purpose hash — just a cheap, portable way to make the two
/// inputs collide as rarely as a 64-bit space allows, with no external
/// hashing dependency needed in this crate.
fn mix64(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    x
}

/// Computes the `build_hash` a `Simulation` with this `build_id` and
/// `patch_generation` would report (see [`Simulation::build_hash`]),
/// without needing a live `Simulation<G>` instance. Used by
/// `orr_session::replay_verify_checked` to check a replay's recorded
/// build_hash against a caller-supplied `build_id` before resimulating
/// anything.
pub fn build_hash_of(build_id: u64, patch_generation: u32) -> u64 {
    if build_id == 0 {
        return 0;
    }
    mix64(build_id ^ mix64(patch_generation as u64).rotate_left(17))
}

impl<G: Game> Simulation<G> {
    /// Builds a fresh simulation: registers `G`'s types (after the engine's
    /// own, so `SimContext::rng` always has a slot), constructs an empty
    /// `Frame`, seeds its RNG, runs `G::setup`, and instantiates `G`'s
    /// system pipeline. `build_id` defaults to `0`; use
    /// [`Simulation::with_build_id`] when the caller has a real one (a
    /// networked session or anything that will write/verify a `.orrp`
    /// replay against a build-hash check should).
    pub fn new(config: G::Config, tick_rate: u32, seed: u64) -> Self {
        Self::with_build_id(config, tick_rate, seed, 0)
    }

    /// Like [`Simulation::new`], but with an explicit `build_id` (see
    /// [`Simulation::build_hash`]).
    pub fn with_build_id(config: G::Config, tick_rate: u32, seed: u64, build_id: u64) -> Self {
        let mut builder = ComponentRegistryBuilder::new();
        builder.register_singleton::<FrameRng>("orr_sim::FrameRng");
        G::register(&mut builder);
        let registry = builder.build();

        let mut frame = Frame::new(registry.clone());
        frame.set_singleton(FrameRng::new(seed));
        G::setup(&mut frame, &config);

        Self {
            registry,
            frame,
            systems: G::systems(),
            tick_rate,
            commands: Commands::new(),
            build_id,
            patch_generation: 0,
        }
    }

    /// A `u64` identifying this simulation's exact running code: a
    /// deterministic mix of the user-provided `build_id` and how many hot
    /// patches ([`Simulation::on_patch_applied`]) have been applied since
    /// start. Two simulations only ever agree on `build_hash` when they
    /// are running the *same* build with the *same* patches applied (or
    /// neither is tracking a build id at all — `build_id == 0` on both
    /// sides is treated by callers, e.g. [`crate::Simulation::new`]'s
    /// default, as "not tracking", so it deliberately does *not* mix in
    /// `patch_generation` in that case, keeping `build_hash() == 0` stable
    /// for tests/tools that never call `with_build_id`).
    ///
    /// Multiplayer sessions and `.orrp` replays are expected to record
    /// this value and refuse to mix peers/resimulate a recording whose
    /// `build_hash` disagrees (see `orr_session`'s `SessionConfig` and
    /// `replay_verify_checked`) — a patch changes what code a tick's
    /// inputs run through, so a mismatched build_hash means "not
    /// comparable", exactly like a version mismatch.
    pub fn build_hash(&self) -> u64 {
        build_hash_of(self.build_id, self.patch_generation)
    }

    /// Records that a hot patch was applied (see `orr_sim::hotpatch`
    /// module docs / the design doc's decision #10): bumps
    /// `patch_generation`, which changes [`Simulation::build_hash`] from
    /// this point on. Call this from whatever applies the patch (a
    /// `subsecond::register_handler` callback in a real dev loop, or a
    /// test that wants to simulate one).
    pub fn on_patch_applied(&mut self) {
        self.patch_generation = self.patch_generation.wrapping_add(1);
    }

    pub fn build_id(&self) -> u64 {
        self.build_id
    }

    pub fn patch_generation(&self) -> u32 {
        self.patch_generation
    }

    pub fn registry(&self) -> &Arc<ComponentRegistry> {
        &self.registry
    }

    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    pub fn frame_mut(&mut self) -> &mut Frame {
        &mut self.frame
    }

    pub fn tick(&self) -> u64 {
        self.frame.tick()
    }

    pub fn tick_rate(&self) -> u32 {
        self.tick_rate
    }

    pub fn checksum(&self) -> u64 {
        self.frame.checksum()
    }

    /// Overwrites this simulation's frame with `snapshot`'s contents (used
    /// by `orr_session` to restore a ring-buffer snapshot before
    /// resimulating). Both frames must share the same registry.
    pub fn restore(&mut self, snapshot: &Frame) {
        self.frame.copy_from(snapshot);
    }

    /// Runs every system once, in fixed registration order, applying each
    /// system's deferred [`Commands`] immediately after it runs (so
    /// entity/component changes are visible to the *next* system this
    /// tick, and structural changes are always applied in
    /// `(system order, recorded order)` — deterministic regardless of any
    /// future parallel execution).
    ///
    /// **Parallel execution hook**: systems whose declared component access
    /// sets don't overlap could run concurrently (per the design doc, plain
    /// integer arithmetic is order-independent) as long as command-buffer
    /// application stays serialized in `(system order, entity id)` order.
    /// `Simulation` intentionally exposes `systems` as a linear `Vec` and
    /// runs them sequentially so that hook can be added later (e.g. a
    /// rayon-backed executor keyed on `QueryTuple::component_ids`) without
    /// changing this method's external behavior for non-overlapping
    /// systems.
    pub fn step(&mut self, inputs: &TickInputs<G::Input, G::Command>) -> Vec<SimEvent<G::Event>> {
        let tick = self.frame.tick() + 1;
        self.frame.set_tick(tick);
        let mut events = Vec::new();

        for (i, sys) in self.systems.iter_mut().enumerate() {
            let mut ctx = SimContext {
                frame: &mut self.frame,
                inputs,
                tick,
                system_index: i as u16,
                seq: 0,
                events: &mut events,
                commands: &mut self.commands,
            };
            // Every system call goes through this one indirection point —
            // see `crate::hotpatch` for why (function-level hot patching,
            // off by default behind the `hotpatch` feature).
            invoke_system::<G>(sys.as_mut(), &mut ctx);
            self.commands.apply(&mut self.frame);
        }

        events
    }
}
