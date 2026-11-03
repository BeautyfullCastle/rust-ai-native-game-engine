# Stormglass scene authoring

Stormglass reuses DefenseGame and existing protocol/package formats. Author the
starting resources and a short campaign without changing simulation code.

## Contract and use

```yaml
schema: orr.scene/1
entities: {}
singletons:
  CoopDefense::Scenario: {starting_gold: 250, reactor_hp: 75}
  CoopDefense::Campaign: {last_wave: 3}
  CoopDefense::WavePacing: {spawn_interval_ticks: 70}
  CoopDefense::WaveCounts: {pair_counts: [1, 2, 3, 16, 18, 20, 22, 24, 26, 28]}
```

`Scenario.starting_gold` is an integer 0–10000 (default 150 per board);
`reactor_hp` is 1–1000 (default 30). `Campaign.last_wave` is 1–10 (default 10):
it selects a prefix of the existing ten waves, not arbitrary enemy/wave editing.
`WavePacing.spawn_interval_ticks` is an integer 35–350 (default 35), at 60
simulation ticks/second. It slows the existing paired-lane spawns, not game
speed, movement, attacks, counts, bosses or enemy composition. Spawns retain
wave-tick alignment: the first interval multiple at/after tick 180, then every
interval. For example 70 starts at tick 210, then 280; 350 starts at 350.
`WaveCounts.pair_counts` is exactly ten integers, each 1–28. Each entry is
one wave's number of paired-lane spawns: total enemies are twice the entry.
Defaults are [10,12,14,16,18,20,22,24,26,28]. All ten entries are validated even
outside the selected campaign. Zero/skipped waves and kind/HP/order overrides
are unsupported. The existing ordinal kind, HP and boss rules stay unchanged.
Missing singletons default independently; old scenes remain valid.
A present singleton in source YAML requires all its fields. `{}`, unknown
fields, invalid ranges, entities, prefab links and runtime pools are rejected.
Seed and other enemy/wave formulas remain fixed. Play-mode edits are rejected.

In Edit mode use existing ERP `world.singleton.patch`:

```json
{"name":"CoopDefense::Campaign","path":"last_wave","value":3}
```

Or `orr propose "three-wave campaign" sset CoopDefense::Campaign.last_wave=3`,
verify and accept. First-field edits use reflected type defaults; whole-singleton wire edits and
source YAML remain strict. Undo/redo, save and reopen retain all settings.
Play and RESTART use the same rollback/checksummed Frame. Victory requires the
last authored wave to finish with no surviving enemies; reactor depletion takes
priority. Terminal gameplay stops until restart, which retains the campaign.
HUDs and ready-screen instructions show the authored wave count.

The default example remains empty; `examples/static-game-starter/scenes/stormglass-short.scene.yaml`
is the explicit 250-gold/75-HP/three-wave fixture.
`stormglass-training.scene.yaml` additionally sets the interval to 70.
`stormglass-counts.scene.yaml` supplies 1/2/3 pairs for its three waves.

## Saved-scene playback and delivery

Save through existing ERP/CLI, then run `coop_defense --scene PATH` or
`coop_defense --scene PATH --headless --ticks 120`. Native admission requires a
regular, non-symlink UTF-8 file no larger than 64 KiB, before any window/session.
Omit `--scene` for defaults. Browser file import uses the same strict parser and
a fixed 64 KiB staging buffer. Invalid input preserves the current client; newer
selections or restart supersede pending reads. Successful import starts ready at
tick zero with the canonical seed, and restart retains all four singletons.

```sh
python3 tools/pack_coop_defense_offline.py dist/coop-defense-web \
  --scene examples/static-game-starter/scenes/stormglass-short.scene.yaml \
  --output short-game.html
python3 tools/check_coop_defense_offline.py dist/coop-defense-web short-game.html
```

The unchanged packer admits exact UTF-8 bytes through trusted local WASM and
publishes to a new explicit path without replacing files. Node is required.
The receipt binds scene/WASM hashes, size and initial checksum, not authorship.
Startup verifies the payload without scene-derived paths or network requests.

## Identity and migration

WaveCounts v4 appends a 40-byte checksummed singleton, changing Frame bytes,
registry fingerprint and checksums even with default counts. The game-specific
`SCENARIO_BUILD_ID` is `0xe6f403d2595bd8cf`; the source-derived identity must agree
across host, starter manifest, native and browser. Prior WavePacing-v3 identity
`0xb2a3336f29697b0b`, Campaign-v2 `0x04ee32c4873f6941`, Scenario-v1
`0x7b2a3ab98f619dca` and other mismatching
nonzero replay identities are rejected. Old YAML compatibility does not imply
binary replay compatibility. The generic zero-header replay wildcard remains;
there is no replay migration. Global frame/protocol/package formats and WASM
function ABI are unchanged, but the matching WASM must be rebuilt. Never reuse
an older game binary or receipt as evidence for this simulation.

`coop-defense-scene` remains optional: simulation-only `coop-defense` does not
enable YAML; editor/ERP dependencies do not enter native simulation.
`DefenseConfig` carries `scenario`, `campaign`, `wave_pacing` and `wave_counts`; use
`..Default::default()` for unchanged settings. Typed setup rejects out-of-range
pacing/counts before ticking; source admission returns an error instead.

## Acceptance and limits

Tests cover strict/atomic admission, independent defaults,
ERP edit/undo/redo/save/reopen/Play, exact paired spawn timing, terminal/restart
retention and replay. The native trace example accepts `--counts` for the count fixture or `--training` for two
complete 70-tick three-wave games without runtime-state edits; pass the matching
scene to `tools/check_coop_defense_wasm.mjs` after the WASM and trace paths. Default trace use still selects the original short scene.
Use the same training scene with the existing offline packer/checker above,
then verify source-hidden delivery and matching consumed-input checksums.

The v4 native and actual-WASM count fixture matches all 1,794 consumed inputs
through two wins and restart. A relocated HTML with packaging inputs hidden
replays the same embedded-WASM trace. Default v3/v4 snapshot gameplay and every
event match across 12,001 snapshots, excluding changed checksums. Replay-loader
rejection uses a recorded nonzero v3 header, not an archived old gameplay replay.
Historical receipts and failures remain preserved.

Linux verification and integration remain separate gates.
CPU/WASM and mock-DOM results do not establish browser/GPU or other-platform acceptance.
