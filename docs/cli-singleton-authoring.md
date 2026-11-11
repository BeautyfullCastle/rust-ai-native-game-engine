# Edit scene-wide settings from the CLI

`orr sset` exposes the existing singleton operation as a direct small edit. It
uses the same host document, reflection, validation, transaction and undo history
as the existing `sset` operation inside `propose` and `apply`. It does not add a
new game schema, bypass permissions or start a host.

## Workflow

Connect to an already running, authorized host. Discover its actual types before
editing; the following Stormglass fields require its registered host:

```sh
orr schema --types
orr schema CoopDefense::Scenario
orr get @CoopDefense::Scenario
orr sset CoopDefense::Scenario.starting_gold=250 CoopDefense::Scenario.reactor_hp=75 --json
orr sset CoopDefense::WaveCounts.pair_counts[0]=5
orr undo
orr redo
orr save --write
```

Singleton names have no `@` prefix in assignments. That prefix is for `get`.
Unambiguous short names and the existing nested field/index syntax work. Values
follow the existing JSON/exact-decimal conventions. Host validation remains
final authority, including game-specific ranges and optional fields.

All assignments in one command are one undo entry. A rejected assignment rolls
back the batch; a transport failure remains an uncertain operation, not permission
to retry blindly. The command requires `read` and `scene_edit`, not `approve`.
It neither starts Play nor saves files. `save --write` separately asks the host to
save its current scene path; camera/model sidecars are not included. Stop Play
before authoring. For a registered editor use `orr gameplay control stop`.

For a larger change, keep replay verification in the workflow:

```sh
orr apply "adjust spawns" sset Scene.spawn_batch=3 --idle 120 --check "lost_bodies.max == 0"
```

That example requires the physics host's Scene and metric. Do not assume a
metric exists in another game; discover and choose checks supported by that host.

## Acceptance

`cargo test --offline --locked -p orr_cli --test singleton_authoring` launches the
real CLI against authenticated loopback physics and Stormglass hosts. It checks multiple-field
history, scene serialization, Undo/Redo, nested indexing, no-op edits,
invalid-batch rollback, singleton type
selection, read-only denial, scene-edit-only authorization, Play rejection, JSON
and offline help/usage. Stormglass coverage edits its real WaveCounts and WaveComposition singletons,
including range validation; it does not run combat or establish gameplay victory.
This is not a
file-write/reopen test, export acceptance, GPU/native-window test or full Linux
integration gate.
