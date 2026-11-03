# Stormglass scene authoring

Author Stormglass resources, wave counts and durability through its existing scene format.

## Contract and use

```yaml
schema: orr.scene/1
entities: {}
singletons:
  CoopDefense::Scenario: {starting_gold: 250, reactor_hp: 75}
  CoopDefense::Campaign: {last_wave: 3}
  CoopDefense::WavePacing: {spawn_interval_ticks: 70}
  CoopDefense::WaveCounts: {pair_counts: [1, 2, 3, 16, 18, 20, 22, 24, 26, 28]}
  CoopDefense::EnemyHealth: {multiplier: 2}
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
outside the selected campaign. Zero/skipped waves and kind/order overrides
are unsupported. Existing family HP is computed first, then scaled below.
`EnemyHealth.multiplier` is one global integer 1–4, default 1. It multiplies
historical family HP with checked integer arithmetic and sets both `hp` and
`max_hp`, including bosses and both lanes (maximum 9744). No rounding,
percentages, per-wave values or family exceptions are introduced. Kind/order,
counts and configured spawn intervals remain unchanged; longer-lived enemies
can alter kills, leaks, wave completion, later spawn times and the outcome.
Missing singletons default independently; old scenes remain valid.
A present singleton in source YAML requires all its fields. `{}`, unknown
fields, invalid ranges, entities, prefab links and runtime pools are rejected.
Seed and other enemy/wave formulas remain fixed. Play-mode edits are rejected.

In Edit mode use existing ERP `world.singleton.patch`:

```json
{"name":"CoopDefense::Campaign","path":"last_wave","value":3}
```

First-field edits use reflected defaults; whole-singleton edits and YAML remain strict. Undo/redo, save and reopen retain all settings.
Play and RESTART use the same rollback/checksummed Frame. Victory requires the
last authored wave to finish with no surviving enemies; reactor depletion takes
priority. Terminal gameplay stops until restart, which retains the campaign.
HUDs and ready-screen instructions show the authored wave count.

The default scene remains empty; `stormglass-short.scene.yaml` is the 250-gold/75-HP/three-wave fixture.
`stormglass-training.scene.yaml` additionally sets the interval to 70.
`stormglass-counts.scene.yaml` supplies 1/2/3 pairs for its three waves.
`stormglass-health.scene.yaml` uses those counts with 4× enemy health.

EnemyHealth appends game Frame state and changes the Stormglass identity from
v4 to v5. Default 1× preserves historical gameplay/events, not Frame bytes or
checksums. Old nonzero build-identity replays are rejected rather than silently
migrated; the existing generic zero-header compatibility policy is unchanged.
Restart retains the authored multiplier. This is a game-specific contract, not
a global scene or engine format change.

## Saved-scene playback and delivery

Save through existing ERP/CLI, then run `coop_defense --scene PATH` or
`coop_defense --scene PATH --headless --ticks 120`. Native admission requires a
regular, non-symlink UTF-8 file no larger than 64 KiB, before any window/session.
Omit `--scene` for defaults. Browser file import uses the same strict parser and
a fixed 64 KiB staging buffer. Invalid input preserves the current client; newer
selections or restart supersede pending reads. Successful import starts ready at
tick zero with the canonical seed, and restart retains all authored singletons.

```sh
python3 tools/pack_coop_defense_offline.py dist/coop-defense-web \
  --scene examples/static-game-starter/scenes/stormglass-short.scene.yaml \
  --output short-game.html
python3 tools/check_coop_defense_offline.py dist/coop-defense-web short-game.html
```

The no-replace packer uses trusted local WASM/Node. Receipts bind scene/WASM
bytes and initial checksum, not authorship. Startup verifies the payload offline.

## Identity and migration

EnemyHealth v5 appends a 4-byte checksummed singleton after WaveCounts v4.
Frame bytes, registry fingerprint and checksums change even at 1×.
`SCENARIO_BUILD_ID` is `0xbc11fd6ffd218902`; host, starter manifest, native and
browser must agree. Prior v4 (`0xe6f403d2595bd8cf`), v3, v2, v1 and other
mismatching nonzero replay identities are rejected. There is no migration.
Global frame/protocol/package formats and WASM function ABI are unchanged;
rebuild matching WASM rather than reusing old binary receipts.

`coop-defense-scene` remains optional; simulation-only `coop-defense` does not
enable YAML or Editor/ERP dependencies. `DefenseConfig` carries the five authored
singletons; use `..Default::default()` for unchanged settings. Typed setup rejects
invalid values before ticking; source admission returns an error.

## Acceptance and limits

Focused tests cover strict/atomic admission, independent defaults, Editor/ERP
Undo/Redo/Save/reopen, combat, restart and replay. The native trace accepts
`--health`, `--counts` or `--training`; default selects the original short scene.
Pass matching scene, WASM and trace to `tools/check_coop_defense_wasm.mjs`.
Offline/source-hidden consumption is checked separately.

Historical v4 evidence remains separate: 1,794-input count trace through two
wins/restart, relocated embedded-WASM replay, and 12,001 default v3/v4 snapshots
and events (checksum excluded). Its old-header rejection was synthetic, not an
archived gameplay replay. Preserve those receipts and original failures; they
do not substitute for v5 checks.

Linux verification and integration remain separate gates.
CPU/WASM and mock-DOM results do not establish browser/GPU or other-platform acceptance.
