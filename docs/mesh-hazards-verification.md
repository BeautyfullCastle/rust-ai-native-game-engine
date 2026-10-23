# MeshHazardsV1 implementation and focused validation

Base: PR205 `e1ba3ea732405defb5da7c19e7b51bf14632fb67`.
Environment: Linux cloud workspace, Rust 1.99.1, 2026-10-19 KST.

## Scope

Opt-in sphere hazards with deterministic swept point-agent contact, a separate
Game/host identity, explicit CLI scene upgrade/inspection/loopback serving, and
ERP editing. Loss rejects the complete tentative movement/input tick and retains
the pre-tick position. Lowest authored hazard index touched anywhere in that tick
wins; this is not earliest-time contact. Hazards precede collection/exit. Loss is
terminal gameplay state, distinct from an engine failure.

Legacy MeshNavigation with its existing mesh-mission feature keeps its registry,
layout and checksums. mesh-hazards implies mesh-mission: this does not promise
identity with a binary previously compiled without mesh-mission. Old scenes are
upgraded explicitly; old runtimes reject the new singleton types. No editor
panel, standalone exported bundle, GPU rendering or native Windows acceptance is
claimed by this milestone.

## Executed checks

- Runtime geometry: 11 focused tests plus 15 existing integration tests passed in
  debug/release; default-profile 15 integration tests and strict all-target
  Clippy passed. Includes exact tangent, high-speed crossing, corner, overflow,
  full-input admission and invalid trailing-data cases.
- Navigation path seam: six new polyline tests; all 58 navigation tests and
  strict Clippy passed. Endpoints share the existing sampler; crossed portals
  are preserved, so a corner is not replaced by a chord.
- Game profile: debug/release 22 unit + five existing physics goldens + six
  integration tests passed. Two later test-only additions passed in the focused
  12-test hazard suite (disabled gameplay equivalence and malformed disabled
  run rejection). Strict all-target Clippy passed before those test-only adds.
- Actual detached base build with mesh-mission produced initial/two-goal-tick
  checksums 14079670917705819130, 15298626416119841259,
  11243942499133049614. The new unchanged-value assertion passed in both
  successor mesh-mission and mesh-hazards builds. No existing golden changed.
- Remote host: 11 mesh library tests (including five new hazards tests) and five
  existing integration tests passed; strict library Clippy passed. Covers
  verified ERP proposals, undo/redo, save/reopen, Play loss, frozen state and Stop.
- CLI: five binary tests and strict binary Clippy passed. Enabled-mission upgrade
  preserves source and mission, rejects overwrite/traversal/symlinks and malformed
  arguments, and reopens canonical zero runtime state.
- Fresh production binary built successfully; actual `--help`, blank-scene `inspect`
  and bounded loopback `serve --stop-after-ms 100` processes exited 0. The latter
  confirms profile identity/startup only, not a new mission-win acceptance.
- Independent source reviews accepted geometry, exact path seam, gameplay,
  remote admission/identity and CLI safety; scoped diff whitespace checks passed.

## Preserved failures and limits

The first ERP fixture requested verification without a check and expected true;
the API correctly returned null. The fixture now explicitly checks entities.max
and passes. An initial offline host build lacked cached ctrlc; ordinary dependency
fetch completed. Sharing a build target with the detached baseline briefly reused
a stale path-dependency artifact; only generated navigation artifacts were cleaned
and the successor was rebuilt. These attempts are not reclassified as passes.

Repeated debug/release runs are not added into a fictitious unique-test total.
Focused checks do not establish whole-engine correctness or performance. The
profile currently clones a Frame for transactional movement; no performance claim
is made. CLI file writing uses existing create_new/write/sync and parent checks,
not crash-atomic publication or race-proof adversarial directory traversal.
Original local logs are retained but not uploaded as remotely accessible evidence.
Required hosted CI and shared development integration remain separate and pending.
