# Stormglass starting-condition authoring

Stormglass reuses DefenseGame and existing protocol/package formats.

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
Undo/redo, scene.save and reopen retain settings. Play and RESTART use authored
values in the rollback/checksummed Frame.
The example scene remains empty for compatibility.

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
new construction now supplies the shared identity. No saved replay migration is provided.


## Acceptance

Source tests cover default/native bytes and input traces, authored start/restart,
ERP edits/undo/redo/save/reparse, atomic invalid-value rejection, ranges/defaults,
Play-edit rejection and old manifest rejection. CPU/API checks do not establish
GPU, platform or live-model authorship results.

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
Scenario. Scenario frame layout/build identity are unchanged.

New source tests cover saved Editor/native initial bytes and consumed-input
traces, restart, CLI/default equivalence, invalid/oversized/non-UTF-8 sources,
nonregular files and Unix link/FIFO rejection. The exact candidate still requires its integration gate.
Windows/macOS file behavior and actual native HUD rendering remain unverified.

## Browser saved-scene playback

The optional browser adapter accepts the same saved Scene YAML through its local
file picker, without uploading contents. A fixed adapter-owned 64 KiB WASM staging
buffer and length/status admission reuse the shared strict parser; malformed
UTF-8, oversized or invalid scenes leave the existing client/source identity
unchanged. A successful load starts a new ready session, clears queued gestures
and commands, and uses the canonical `DefenseConfig` default seed. The legacy
browser `defense_init(seed)` path remains seed-compatible. Restart retains the
imported Scenario and the HUD uses authored `maxHp`, not a fixed 30.

Overlapping asynchronous file reads are generation-guarded; a later selection or
restart supersedes earlier reads. Served and generated offline pages use the same
picker. The exporter also delivers saved scenes without a later picker step.
Actual WASM/simulated DOM checks are distinct from browser/GPU/manual QA.


```sh
python3 tools/pack_coop_defense_offline.py dist/coop-defense-web \
  --scene saved.scene.yaml --output authored-game.html
python3 tools/check_coop_defense_offline.py dist/coop-defense-web authored-game.html
```

Authored delivery requires a new explicit output path. The packer snapshots regular,
non-symlink inputs, admits the exact UTF-8 scene (at most 64 KiB) through the trusted
local build’s WASM with a bounded Node subprocess, then publishes atomically without
replacing existing files. No scene-derived paths or network requests are used.
The default packaging command remains compatible. Node must be installed.

The HTML carries base64 scene bytes and a versioned receipt containing scene/WASM
SHA-256 hashes, scene byte count and tick-zero checksum—not local paths. These are
content identities, not signatures or proof of authorship. Opening the page verifies
the receipt and starts the authored ready screen at tick zero; invalid payloads fail
closed rather than falling back to the default game. Restart retains the Scenario.
WebCrypto is required at startup; unavailable verification fails closed.
This validates trusted engine output, not hostile WASM in a security sandbox.
