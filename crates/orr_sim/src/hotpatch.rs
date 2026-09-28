//! Function-level hot-patch hook for system invocation (design doc §0
//! decision #10, §3.4): every system call in [`crate::Simulation::step`]
//! goes through the single indirection point [`invoke_system`], so a
//! desktop dev build can swap in a recompiled system body without
//! restarting the simulation, subsecond/Dioxus-style.
//!
//! # Status
//!
//! Gated behind the `hotpatch` cargo feature, **off by default** (and never
//! meant to be enabled in a release, CI or wasm32 build — see below). With
//! the feature off, [`invoke_system`] is a direct call with zero overhead:
//! this module still compiles and is usable (so callers/tests don't need
//! `#[cfg]` gymnastics), it just has nothing to patch.
//!
//! With the feature on, [`invoke_system`] routes the call through
//! `subsecond::call` (confirmed to build as a plain library dependency in
//! this workspace — see `Cargo.toml`). `subsecond::call` is a no-op
//! passthrough in release builds (`if !cfg!(debug_assertions) { return f()
//! }`) and only engages its jump-table/panic-unwind machinery in a debug
//! build, so enabling the feature is safe to leave compiled into a binary
//! as long as that binary only actually *uses* it in a desktop dev loop.
//!
//! subsecond's own docs describe it as desktop/dev-loop oriented (it loads
//! a recompiled dylib via `libloading` and swaps function pointers via a
//! jump table) and it does not support wasm32 code-swapping (only a
//! best-effort wasm story via `wasm-bindgen`, explicitly called out as
//! experimental upstream). Per the design doc's own risk note: **hot
//! patching is a desktop development-time feature only.** It must stay off
//! for:
//! - release builds (ship a normal static binary),
//! - CI / the determinism golden tests (a patch changes code, which is
//!   exactly the thing a build-hash bump is for — see
//!   [`crate::Simulation::build_hash`]),
//! - wasm32 targets (no supported code-swap path).
//!
//! # The `HotPatchHook` trait
//!
//! [`HotPatchHook`] is the documented extension point mentioned in the
//! design doc: a way to plug in a different invocation strategy later
//! (e.g. wiring an explicit per-`Simulation` hook object instead of a
//! crate-wide `cfg`, or a test double that counts invocations) without
//! touching `Simulation::step` again. [`DirectCall`] is the always-available
//! default implementation ([`invoke_system`] itself doesn't use a `dyn
//! HotPatchHook` — it's a free function chosen by `cfg` for zero overhead —
//! but `DirectCall`/`HotPatchHook` are kept as the documented seam a future
//! per-`Simulation` hook would plug into).
use crate::context::SimContext;
use crate::game::Game;
use crate::system::System;

/// The pluggable seam a future seam for hot-patching (or instrumentation,
/// or a test double) attaches to: something that knows how to invoke one
/// system for one tick. See the module docs for why the default path
/// ([`invoke_system`]) doesn't need a trait object today, and why this
/// trait is kept anyway as the documented extension point.
pub trait HotPatchHook<G: Game>: Send + Sync {
    fn invoke(&self, sys: &mut dyn System<G>, ctx: &mut SimContext<G>);
}

/// The default [`HotPatchHook`]: a plain, direct call. Used whenever the
/// `hotpatch` feature is off, and available even when it's on (e.g. for a
/// headless server or CI binary that links a `hotpatch`-enabled build of
/// `orr_sim` but must not actually hot-swap).
#[derive(Default, Clone, Copy)]
pub struct DirectCall;

impl<G: Game> HotPatchHook<G> for DirectCall {
    #[inline]
    fn invoke(&self, sys: &mut dyn System<G>, ctx: &mut SimContext<G>) {
        sys.run(ctx);
    }
}

/// The single indirection point every system call in
/// [`crate::Simulation::step`] goes through.
///
/// With the `hotpatch` feature off (the default), this is `sys.run(ctx)`
/// with nothing in between — inlined away entirely in a release build.
/// With it on, the call is wrapped in `subsecond::call`, so a connected
/// dev-loop tool that patches the running process's code can make the next
/// call to this system's body use the freshly compiled version.
#[inline]
pub(crate) fn invoke_system<G: Game>(sys: &mut dyn System<G>, ctx: &mut SimContext<G>) {
    #[cfg(feature = "hotpatch")]
    {
        // `subsecond::call` takes `FnMut() -> O`; the closure only ever
        // runs the body once per `invoke_system` call in the steady state
        // (no patch pending). It may re-run it if `subsecond` catches its
        // own "code above this call changed" unwind signal after a patch
        // lands — that's an explicit, documented subsecond behavior for
        // dev-loop use, not something that happens in a normal (non-patch)
        // tick, so it does not affect determinism of a from-genesis replay
        // (any tick that involved a live patch bumps `patch_generation`,
        // hence `Simulation::build_hash`, so it is never compared against
        // a pre-patch recording in the first place).
        subsecond::call(|| sys.run(ctx));
    }
    #[cfg(not(feature = "hotpatch"))]
    {
        sys.run(ctx);
    }
}
