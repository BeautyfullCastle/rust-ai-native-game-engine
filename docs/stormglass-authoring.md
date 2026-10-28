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
```

`Scenario.starting_gold` is an integer 0–10000 (default 150 per board);
`reactor_hp` is 1–1000 (default 30). `Campaign.last_wave` is 1–10 (default 10):
it selects a prefix of the existing ten waves, not arbitrary enemy/wave editing.
Missing singletons default independently: empty and old Scenario-only scenes
remain compatible; Campaign-only scenes start with 150 gold and 30 HP.
A present singleton in source YAML requires all its fields. `{}`, unknown
fields, invalid ranges, entities, prefab links and runtime pools are rejected.
Seed and enemy/wave formulas remain fixed. Play-mode edits are rejected.

In Edit mode use existing ERP `world.singleton.patch`:

```json
{"name":"CoopDefense::Campaign","path":"last_wave","value":3}
```

Or `orr propose "three-wave campaign" sset CoopDefense::Campaign.last_wave=3`,
verify and accept. First-field edits use reflected type defaults; whole-singleton wire edits and
source YAML remain strict. Undo/redo, save and reopen retain both settings.
Play and RESTART use the same rollback/checksummed Frame. Victory requires the
last authored wave to finish with no surviving enemies; reactor depletion takes
priority. Terminal gameplay stops until restart, which retains the campaign.
HUDs and ready-screen instructions show the authored wave count.

The default example remains empty; `examples/static-game-starter/scenes/stormglass-short.scene.yaml`
is the explicit 250-gold/75-HP/three-wave fixture.

## Saved-scene playback and delivery

Save through existing ERP/CLI, then run `coop_defense --scene PATH` or
`coop_defense --scene PATH --headless --ticks 120`. Native admission requires a
regular, non-symlink UTF-8 file no larger than 64 KiB, before any window/session.
Omit `--scene` for defaults. Browser file import uses the same strict parser and
a fixed 64 KiB staging buffer. Invalid input preserves the current client; newer
selections or restart supersede pending reads. Successful import starts ready at
tick zero with the canonical seed, and restart retains both singletons.

```sh
python3 tools/pack_coop_defense_offline.py dist/coop-defense-web \
  --scene examples/static-game-starter/scenes/stormglass-short.scene.yaml \
  --output short-game.html
python3 tools/check_coop_defense_offline.py dist/coop-defense-web short-game.html
```

The unchanged packer admits exact UTF-8 bytes through trusted local WASM and
publishes to a new explicit path without replacing files. Node is required.
The unchanged receipt carries scene/WASM SHA-256, byte count and tick-zero
checksum—not local paths. These are content identities, not signatures or
proof of authorship. Startup verifies them and fails closed without WebCrypto
or valid payload. No scene-derived paths or network requests are used.

## Identity and migration

Campaign v2 appends a singleton, intentionally changing frame bytes, registry
fingerprint, checksums and shared `SCENARIO_BUILD_ID`. Host, starter manifest,
native and browser use the same identity `0x04ee32c4873f6941`. Previous Scenario-v1 identity
`0x7b2a3ab98f619dca` and mismatching nonzero replay identities are rejected; retaining valid
old YAML does not imply binary replay compatibility. Manifest migration is an
explicit reviewed build-ID update, never an automatic rewrite.
Global frame/protocol/package formats remain unchanged. No replay migration is
provided. Generic checked replay still accepts its existing zero-header
wildcard; old zero-ID recordings and unguarded replay APIs gain no new identity
protection. Frame decoding checks singleton count/name/size.

`coop-defense-scene` remains optional: simulation-only `coop-defense` does not
enable YAML; editor/ERP dependencies do not enter native simulation.
`DefenseConfig` carries both `scenario` and `campaign`; Rust struct literals
should include `..Default::default()` or both explicit values.

## Acceptance and limits

Reproducible tests cover independent defaults, strict/atomic rejection, ERP
schema/edit/undo/redo/save/reopen/Play preservation, terminal edge cases and
restart retention. The native campaign-trace example runs two complete genuine
three-wave games and exact same-input replay; the WASM checker consumes every
recorded input/checksum, including terminal and restart ticks. No wave-four
spawn is permitted. Existing default-game goldens change only for the appended
singleton and are retained as historical evidence in checker comments.
CPU/WASM and simulated-DOM results are not actual browser/GPU, Windows/macOS,
Android or real-model authorship verification. Those remain separate checks.
