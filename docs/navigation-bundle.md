# Closed authored NavigationYard3D bundle (optional Linux x86-64)

Build the optional Rust tools (no Python frontend):

```sh
cargo build --locked --release -p orr_navigation_view --features authored-bundle --bin navigation_player --bin navigation_export
navigation_export --scene /trusted/author/navigation.scene.yaml \
  --runtime /trusted/navigation_player --runtime-sha256 LOWERCASE_SHA256 \
  --trusted-runtime --output /trusted/new-bundle --source-revision user-claim
/path/to/new-bundle/run-navigation --ticks 60 --restart-check
```

Save the terrain and built point route in the NavigationYard3D editor, then Save
its scene. Export takes those **saved bytes**, not dirty editor fields. It copies
exactly five files: original scene at `project/game.scene.yaml`, original terrain
at the unchanged scene-relative pin path under `project`, `bin/navigation_player`,
the fixed `run-navigation` launcher and `navigation.bundle.json`. It does not copy
arbitrary neighboring sidecars, add assets, or rewrite terrain/graph revisions.

`authored-bundle` is default-off and includes `authored`; the older authored-only
player does not accept bundles. No renderer, window or input feature is added.
Point-agent semantics remain radius/headroom/max-step clearance zero, terrain at
most17×17, no physics/avoidance/general-navmesh promise.

## Admission, trust and publication

The bounded closed manifest binds exact paths, roles, modes, lengths, SHA256s,
initial Frame checksum, terrain identity/revisions and unverified user provenance.
Unexpected files or directories, symlinks, nonregular files, missing data and
mismatches are refused. Bundle mode also binds the current executable's path and
bytes to its manifest entry. Scene/terrain are re-admitted through the existing
NavigationYard3D contract. The manifest digest is integrity, **not a signature**.

The caller must explicitly trust the supplied already-built executable and supply
its expected SHA256. ELF/capability checks and successful smoke do not authenticate
code, attest source, validate license rights or sandbox execution. The copied
runtime is actually executed with bounded timeout/output and process-group cleanup;
its60-tick/restart receipt must equal independently derived local Simulation output.
Caller `source_revision` is merely an owner-provided record, never build proof.

Source scene<=1MiB, terrain follows the existing decoder bound, executable<=64MiB,
manifest<=64KiB and fixed launcher<=4KiB. Paths/arguments are bounded; input and
output must not overlap. All input snapshots precede staging. A private sibling
stage is admitted, smoke-tested and rechecked with sources and exact inventory
before fsync and atomic NOREPLACE rename. Existing outputs are never overwritten.
An error before publication retains the private stage for inspection rather than
recursively deleting possibly foreign content. After rename, failure reports a
published result with uncertain durability/identity, not rollback or safe retry.

Inputs and directory ancestry must be trusted and quiescent. Descriptor no-follow
checks reduce accidental path substitution but are not a hostile-concurrent-user
sandbox or an atomic source-project snapshot. Same-host tested Linux system/shared
libraries remain dependencies: copying this executable is not a universal portable
SDK or Windows/browser build. Capture/window/input and editor Export UI are outside
this first bundle slice.

## Evidence boundaries

Focused tests distinguish synthetic manifest integrity from real trusted runtime
execution. Actual Editor Save→export→hidden original scene/terrain/runtime→bundle
relocation with spaces→empty-cwd launcher checks use the real binaries and compare
checksums against authoritative editor ticks. This is ordinary filesystem relocation,
not isolated mounts. Existing runtime pixel evidence is separate; bundle tests do
not claim new GPU/browser/physical-input acceptance. Original Python test/oracle
workflows and frozen candidates remain unchanged.
