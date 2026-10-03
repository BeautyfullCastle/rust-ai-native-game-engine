# Simulation float-type guard

Repository-owned simulation libraries must use integer/fixed-point types in
their state. `bytemuck::Pod` permits `f32` and `f64`; the blanket `Component`
implementation therefore does **not** prove that a component is float-free.
Clippy's arithmetic lint also cannot reject an arithmetic-free float field.

Run the additional type check and its executable regression fixtures with
Python 3.11+ and Rust/Clippy installed:

```sh
python tools/check_sim_float_types.py
python tools/test_sim_float_guard.py
cargo test -p orr_fp -p orr_ecs -p orr_sim -p orr_session --release
cargo clippy -p orr_fp -p orr_ecs -p orr_sim -p orr_session --all-targets -- -D warnings
```

The type guard runs real `cargo clippy --lib` invocations for `orr_fp`,
`orr_ecs`, `orr_sim`, `orr_session`, `orr_testgame`, `orr_physics`,
`orr_physics3d`, and `orr_games`. It selects
`tools/sim-float-guard/clippy.toml` with `CLIPPY_CONF_DIR`, adding primitive
`f32`/`f64` to the existing forbidden collection/clock types. It checks that
this configuration still contains every type forbidden by the workspace
configuration. The workspace's normal all-targets lint remains a separate,
required CI step; this guard does not replace it. All four native matrix legs
run the guard and fixtures before their existing tests and platform checks.

Clippy rejects explicit primitive uses in fields, arrays, nested type
definitions, generic arguments, qualified primitive paths, and repository-owned
type alias declarations, even without arithmetic. The fixture runner first
compiles the fixtures with `cargo check`, including their derived POD and
`Component` bounds, then checks that only the expected Clippy type diagnostics
cause negative fixtures to fail. FP-backed state and view interop must pass.

## Exceptions

The guard checks library targets. Integration tests, benches, and profiling
examples can use floats as independent accuracy or measurement oracles; they
cannot be used as authoritative simulation state. View crates use the normal
workspace policy, so their float rendering/UI types are unaffected.

Within guarded libraries the sole float exception is
`crates/orr_fp/src/float_interop.rs`, enabled by `float-interop`. The guard
checks FP both without that feature and with `float-interop,serde`; the module's
local allowance preserves the view conversion API. Other simulation libraries
use `-F clippy::disallowed_types`, which rejects local attempts to relax this
lint. FP's default-feature check also uses `-F`. Its interop feature check needs
`-D` so its conversion module can use a local `allow`; enforcing the sole
exception in that feature build depends on source review. A new local allowance
in FP could bypass the lint there and must not be added outside the conversion
module. Interop values must never feed back into
authoritative simulation state.

## Limits

This is a repository lint contract, not a breaking change to `Component` and
not a static proof about arbitrary downstream types. It does not recursively
inspect the layout of an opaque type or alias defined in an unchecked dependency,
prove what raw integer bits represent, or prevent floats introduced through
uninspected generated/external code. Generic associated types and inferred
types are not promised to reveal an underlying float at every use site. An
alias defined in a checked library is rejected at its primitive declaration;
importing an already-defined external alias is outside that guarantee.

The guard does not prove the absence of float instructions. Keep the existing
arithmetic lint and exact cross-platform golden checks as independent checks.
Running ordinary `cargo check` alone is insufficient: Clippy and the dedicated
guard command must execute. No golden values or simulation behavior change.

Clippy configuration details: [configuration lookup](https://doc.rust-lang.org/stable/clippy/configuration.html)
and [disallowed-types](https://doc.rust-lang.org/stable/clippy/lint_configuration.html#disallowed-types).
