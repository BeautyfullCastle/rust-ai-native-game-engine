# Stormglass starting-condition authoring

The registered Stormglass project reuses DefenseGame; it does not introduce
another simulation, engine game enum, protocol or package format.

## Contract and use

Only `CoopDefense::Scenario` is reflected: `starting_gold` is an integer
0–10000 (default 150 for each board); `reactor_hp` is an integer 1–1000
(default 30). A legacy empty scene uses those defaults. Entities, prefab
links, runtime DefenseRun/pool authoring and Play-mode edits are rejected.
Waves, enemy stats and seed remain fixed by the existing game.

In Edit mode use the existing ERP `world.singleton.patch`, for example:

```json
{"name":"CoopDefense::Scenario","path":"starting_gold","value":250}
```

Or use `orr propose "starting resources" sset
CoopDefense::Scenario.starting_gold=250`, verify and accept the proposal.
Existing undo/redo, scene.save and registered-project reopen retain the
settings. Play starts with them; RESTART resets the run/pools using the same
seed and authored values, which remain in the rollback/checksummed Frame.
The example scene stays empty for backwards source compatibility.

## Identity and migration

Scenario v1 appends a simulation singleton, changing frame bytes, registry
fingerprint and checksums even for default gameplay. `SCENARIO_BUILD_ID` is
shared by host, starter descriptor, native session and browser simulation; the
shipped manifest intentionally moves to `0x7b2a3ab98f619dca`. The game name stays
`CoopDefense`; global engine format/version and golden checksums are unchanged.
Old registered manifests fail the build-ID guard: migration is an explicit
manifest build-ID update after retaining/reviewing the scene, not automatic
rewriting. The frame decoder checks singleton count/name/size (`orr_ecs::Frame::from_bytes`);
the appended singleton makes old layouts incompatible by that source contract.
An old-snapshot fixture has not been executed for this change. Checked replay/build validation rejects mismatching nonzero identities.
Generic checked replay retains its existing zero-header wildcard: old native
recordings with build_hash=0 are not guaranteed identity rejection. Unguarded
replay APIs likewise do not gain protection from this change. Native/browser previously used a zero build-ID wildcard;
new construction now supplies the shared identity. No saved replay migration
or browser scene-loading UI is provided.


## Acceptance

Focused source tests cover default/native byte equality and 600-tick input
trace, explicit-default/legacy equivalence, authored start and restart,
ERP singleton edits/undo/redo/save/reparse, invalid whole-object atomicity,
ranges and first-field defaults, Play-edit rejection, and old manifest
rejection. These tests require execution on the exact candidate; this document
does not claim they have passed. Existing editor project tests cover Play,
command consumption and Stop. No GPU, platform or live-model authorship result
is implied by CPU/API tests.

## Native saved-scene playback

Save the authored scene through the existing ERP `scene.save`/CLI path, then
run `coop_defense --scene PATH` or
`coop_defense --scene PATH --headless --ticks 120`. The file must be regular,
non-symlink UTF-8 and at most 64 KiB. Loading and strict admission happen before
any window, GPU or session creation. Omit `--scene` to retain the old defaults.
The native HUD reads its maximum reactor HP from the same Scenario singleton.

`orr_games/coop-defense-scene` is an opt-in source-admission package feature;
plain `coop-defense` still does not enable YAML. ERP and native playback reuse
the same Scene parser/baker, reflection bounds, legacy defaults and closed
source shape. A present Scenario in source YAML requires both fields; ERP
whole-value edit defaults do not make partial source YAML valid. No editor/ERP dependency enters native simulation, and no new
protocol or package format is added. `DefenseConfig` now carries `scenario`;
Rust callers using a struct literal add `..Default::default()` or an explicit
Scenario. This configuration change does not change the Scenario frame layout
or build identity introduced above.

New source tests cover saved Editor/native initial bytes and consumed-input
traces, restart, CLI/default equivalence, invalid/oversized/non-UTF-8 sources,
nonregular files and Unix link/FIFO rejection. These are focused CPU checks,
not a substitute for the exact candidate’s mandatory integration gate.
Windows/macOS file behavior and actual native HUD rendering remain unverified.
