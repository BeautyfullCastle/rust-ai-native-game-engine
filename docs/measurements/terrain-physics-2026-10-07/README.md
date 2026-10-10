# Terrain sphere integration verification

This records the initial implementation proof at `96f6e966`. Independent review
found follow-up corrections; see [repair and final proof](review-repair/README.md).

Base: `7e2a6a6948972781876611ac0a405c3e95a5bfbf` (reviewed terrain editor).
Scope: bounded one-static-heightfield/dynamic-sphere slice of #102. Broader terrain
collision remains open. See [contract](../../terrain-physics.md).

All Cargo work uses the restored `rust-setup/cargo-release` wrapper: release,
2 jobs, incremental disabled, shared target, and a 1 GiB free-space guard.
Only focused package/target checks were requested; this is not a workspace-wide
pass. Native Windows, physical GPU and native window operation are unverified.

## Preserved failures

- First behavior run: 16 passed, 2 failed. Refreshed speculative contacts hovered
  at the query envelope because the old target formula clamped negative gap/h
  to zero. The optional checked solver now permits approach while separated;
  original convex-only target behavior and goldens remain unchanged.
- First editor feature check: one `Option<PathBuf>` versus `Option<&Path>` compile
  error. Corrected with `as_deref`.
- First host test compile: `ClientError` was incorrectly queried as `RpcError`.
  Corrected by matching the RPC variant.
- First host test execution: 4 passed, 1 failed. Existing hosts silently ignore
  scene path overrides when paths are disabled. Terrain now uses an explicit
  opt-in strict rejection flag; ordinary host behavior remains unchanged.
- First adapter strict Clippy: one test-only field reassignment after Default.
  Corrected with a struct initializer, without lint suppression.

## Review-driven corrections before final verification

- Isolated proposal acceptance and restored failed GUID reservations
- Snapshot-based transaction rollback without re-reading missing external assets
- Refused raw ERP debug and play-to-scene capture for asset-backed play
- Checked narrow solver intermediates before frame publication
- Refreshed endpoint contact ownership and cleared sleep support after last-step loss
- Validated live list handles distinctly from live empty lists
- Kept no-terrain package/entry behavior separate from bounded terrain validation

## Focused final verification

Commands below use `cargo-release` (which supplies `--release -j 2`).

| Package and focused targets | Result | Log |
| --- | --- | --- |
| `orr_physics3d --lib --test static_contacts --test golden --test session3d` | 46 passed: 20 unit, 15 seam, 10 unchanged golden, 1 rollback session | `terrain-physics-solver-final.log` |
| `orr_physics3d --test physics3d` | 26 passed, existing behavior tests | `terrain-physics-existing-physics-final.log` |
| `orr_terrain_physics3d --tests` | 41 passed: 12 assets, 28 geometry/runtime, 1 separately pinned terrain golden | `terrain-physics-adapter-pinned.log` |
| `orr_edit --test bake_admission --test atomic_batch --test proposal --test play` | 30 passed: 5 admission, 5 atomic batch, 11 proposal, 9 play | `terrain-physics-edit-final.log` |
| `orr_ecs --test list_liveness --test checksum --test serialize` | 20 passed | `terrain-physics-ecs-tests.log` |
| `orr_remote --features terrain-physics --test terrain_yard3d` | 5 passed | `terrain-physics-host-frozen.log` |
| `orr_editor --features terrain-physics --test terrain_physics --test deps` | 5 passed: 2 production ERP/editor/egui, 3 dependency architecture | `terrain-physics-editor-tests.log` |
| `orr_editor --features terrain-physics --test terrain_physics_gpu -- --nocapture` | 1 mandatory headless production-viewport test passed, no skip | `terrain-physics-gpu-test.log` |
| `orr_games --features terrain-physics --lib` | 2 passed, including explicit missing-admission failure | `terrain-physics-game-tests.log` |
| `orr_bridge --lib` | 14 passed, including immutable frame-list read | `terrain-physics-bridge-tests.log` |

Strict Clippy uses `-- -D warnings` and passes on the adapter (all targets),
physics library/static contacts/golden/session tests, editor library/CPU/GPU/deps
tests, remote library/terrain host test/binary, edit library/admission/atomic/
proposal/play tests, ECS library/list-liveness test, bridge library, and optional
game library. See each `*-clippy*.log` for exact package build output.

`cargo check -p orr_games --no-default-features --lib` passes. The recorded normal
dependency tree for that configuration contains no terrain crate. Core ECS,
simulation, session and physics manifests acquire no terrain dependency.

## Separate new terrain golden

`orr_terrain_physics3d/tests/golden.rs` pins the fixture at 60/120/240 steps:
`8d551ecfcd815005`, `9c8701f6d712c0e7`, `a7fc4ddbfbedeefe`.
The initial baseline run also asserts one sphere is resting/asleep at the actual
terrain surface and the other has fallen more than 20 units through the hole.
Existing `orr_physics3d` golden constants were not edited.

## Actual headless viewport captures

The mandatory viewport test reports `llvmpipe (LLVM 19.1.7, 256 bits)`, software
rendering. Mesa shader cache writes were denied by the read-only home directory;
Mesa explicitly disabled its cache and rendering still passed. No permissions
or system settings were changed.

- [01 admitted](captures/01-admitted.png): two suspended spheres, solid surface
  and open 2×2-cell hole
- [02 Play](captures/02-play-solid-and-hole.png): one sphere resting on terrain,
  the other fallen through the hole, with the source asset deleted
- [03 Stop](captures/03-stop-restores-scene.png): exact initial viewport restored

01 and 03 share SHA-256
`e6664989c8f925447055275c521413121d703f9e2aaef8adc26208d94067037b`.
02 is `7bc494caf31c7d930e45601945ec5e8f2d278886a0603bac63ccb32ce9a0b1fc`.
The images were visually inspected and the test asserts pixel equality for Stop.
These do not establish native-window, physical-GPU or Windows operation.

All earlier failure logs remain alongside the successful reruns.

Total: 190 focused tests passed. All listed strict Clippy and no-terrain checks passed.
Independent exact-commit review is a separate publication gate.
