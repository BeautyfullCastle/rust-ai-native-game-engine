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
or new native/browser scene-loading UI is provided.


## Acceptance

Focused source tests cover default/native byte equality and 600-tick input
trace, explicit-default/legacy equivalence, authored start and restart,
ERP singleton edits/undo/redo/save/reparse, invalid whole-object atomicity,
ranges and first-field defaults, Play-edit rejection, and old manifest
rejection. These tests require execution on the exact candidate; this document
does not claim they have passed. Existing editor project tests cover Play,
command consumption and Stop. No GPU, platform or live-model authorship result
is implied by CPU/API tests.
