# Stormglass scene authoring

Author resources, wave composition, counts, pacing and durability through the existing scene format. Native, registered Editor and WASM players consume the same configuration.

## Contract

```yaml
schema: orr.scene/1
entities: {}
singletons:
  CoopDefense::Scenario: {starting_gold: 250, reactor_hp: 75}
  CoopDefense::Campaign: {last_wave: 3}
  CoopDefense::WavePacing: {spawn_interval_ticks: 70}
  CoopDefense::WaveCounts: {pair_counts: [5, 6, 7, 16, 18, 20, 22, 24, 26, 28]}
  CoopDefense::EnemyHealth: {multiplier: 2}
  CoopDefense::WaveComposition:
    regular_kinds: [[3,0,2,1], [2,3,1,0], [0,2,3,1], [0,1,2,3], [1,2,3,0], [2,3,0,1], [3,0,1,2], [0,1,2,3], [1,2,3,0], [2,3,0,1]]
    opening_boss: [1,0,1,0,0,0,0,0,0,0]
```

- `Scenario`: starting gold 0–10000 per board (default 150); reactor HP 1–1000 (default 30).
- `Campaign.last_wave`: 1–10 (default 10), selecting a prefix of the campaign.
- `WavePacing.spawn_interval_ticks`: 35–350 (default 35), at 60 simulation ticks/second. The first spawn occurs at the first interval multiple at/after wave tick 180. Interval 70 starts at 210, then 280; interval 350 starts at 350. Movement and attacks retain their speeds.
- `WaveCounts.pair_counts`: exactly ten integers 1–28, defaults `[10,12,14,16,18,20,22,24,26,28]`. Each pair spawns one enemy per lane.
- `EnemyHealth.multiplier`: integer 1–4, default 1, applied after family HP arithmetic, including bosses. Maximum HP remains 9744. Counts and timing stay independently configured.
- `WaveComposition.regular_kinds`: exactly ten rows of four integers. Codes are `0=normal`, `1=fast`, `2=armored`, `3=horde`. Wave 1 uses row 0, wave 10 uses row 9. Each row repeats every four paired spawns, identically in both lanes.
- `WaveComposition.opening_boss`: exactly ten flags 0/1. A 1 replaces that wave's first pair with bosses. It does not shift the pattern: the second pair uses row index 1. For `[3,0,2,1]` with boss enabled, the first five pairs are boss, normal, armored, fast, horde. Boss-off waves use row index 0 normally.

Default composition rows rotate by wave: `[1,2,3,0]`, `[2,3,0,1]`, `[3,0,1,2]`, `[0,1,2,3]`, repeating. Only waves 5/10 default to opening bosses. This preserves historical gameplay. Authoring can change difficulty and outcomes; it does not guarantee victory.

Missing singletons default independently. Present YAML singletons require all fields, with exact array dimensions and integer bounds, including waves beyond `last_wave`. Unknown fields, entities, prefab links and runtime pools are rejected. Seed, formulas, enemy movement and pools remain fixed.

## Edit, save and play

In the registered Editor, open **Details → Authored waves** to inspect starting resources, enemy HP multiplier, first spawn tick and cadence, and the complete enemy order/count per lane for each active wave. This read-only preview follows the displayed snapshot, including unsaved edits and Undo/Redo before Play; Save/reopen retains the settings. In Play it describes session configuration, not remaining enemies or a predicted victory. It does not edit the scene or send commands.

Existing ERP `world.singleton.patch` accepts nested paths, for example:

```json
{"name":"CoopDefense::WaveComposition","path":"regular_kinds[0][0]","value":3}
```

First-field edits use reflected defaults. Undo/Redo, Save/reopen and RESTART retain the settings; Play-mode edits are rejected. Victory requires the last authored wave to finish with no surviving enemies; reactor depletion takes priority.

`stormglass-composition.scene.yaml` is a three-wave example with different patterns and bosses on waves 1/3. The short/training/counts/health fixtures remain available.

Create a validated scene with trusted local WASM/Node:

```sh
orr_pkg create-coop-scene --wasm game.wasm --output new.scene.yaml \
  --wave-kinds '3,0,2,1;2,3,1,0;0,2,3,1;0,1,2,3;1,2,3,0;2,3,0,1;3,0,1,2;0,1,2,3;1,2,3,0;2,3,0,1' \
  --opening-bosses '1,0,1,0,0,0,0,0,0,0'
```

Either option fills the omitted counterpart with defaults. Omitting both preserves the prior generated YAML bytes. Creator output is no-replace and validated before publication.

Run `coop_defense --scene PATH`, optionally `--headless --ticks 120`. Native input must be regular non-symlink UTF-8, at most 64 KiB. Browser import uses the same strict parser and bounded staging; invalid input preserves the client. Pack a saved scene with `tools/pack_coop_defense_offline.py BUILD --scene PATH --output NEW.html`; verify with `tools/check_coop_defense_offline.py BUILD NEW.html`.

## Identity and acceptance

WaveComposition v6 appends 200 bytes of checksummed Frame state. Build ID is `0x7e9825f4e2a74229`; rebuild host/starter/native/WASM together. Defaults preserve gameplay/events, not Frame bytes/checksums. Prior nonzero replay identities, including v5 `0xbc11fd6ffd218902`, are rejected without migration. Existing zero-header wildcard policy and global formats/WASM function ABI are unchanged.

`coop-defense-scene` stays optional; simulation-only use gains no YAML/Editor dependencies. `DefenseConfig` now carries six authored singletons; use `..Default::default()` for unchanged settings.

Focused acceptance covers strict/atomic admission, ERP and registered Editor edit/history/save/play, literal spawn order/HP, restart/replay, native/WASM traces and relocated source-hidden packed consumption. Native trace accepts `--composition` alongside existing modes. CPU/WASM/mock-DOM evidence is not browser/GPU/native-window or Windows acceptance. Preserve earlier evidence and original failures separately.
