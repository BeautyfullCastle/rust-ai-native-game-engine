# Optional CoopDefense browser adapter

This crate compiles `orr_games/coop-defense` into WebAssembly. JavaScript handles
input, drawing and optional synthesized audio, never combat, economy or the AI
partner. The portrait six-role game is a temporary validation fixture, not a
completed remake or claim of original-game fidelity.

## Build and play

Install Rust's official `wasm32-unknown-unknown` target, then run
`bash tools/build_coop_defense_web.sh /absolute/output-directory`. Serve that
folder over HTTP(S), or open `stormglass-foundry-offline.html` without a server.
Packaging embeds the exact WASM/art and closed ES-module graph. WebAssembly and
Canvas2D are required; no account, relay or third-party runtime service is used.

Start, summon (Q), move by drag or two taps, merge matching role/tier units,
upgrade (E), invest (R), pause (Space), and restart. The other board is local AI.
Ready/pause/hidden states freeze presentation time. Audio starts muted and needs
a trusted gesture. Default character images use procedural fallback motion, not
authored animation; synthesized cues are not final sound design.

## Engine and saved scenes

`defense_init` creates a source instance; `defense_step` advances one 60 Hz tick.
Copy snapshot UTF-8 before another snapshot/reset; never retain a WASM memory
view across allocation. Restart is an authoritative input with a higher sequence.
The strict consumer uses actual actor IDs/events, not HP/cooldown differences.
Malformed contracts fail closed; gaps/restart/overflow/source changes resync
without invented effects. See the [presentation contract](../../docs/coop-defense-presentation.md).
`window.defenseDebug` is for inspection, not additional gameplay rules.

**저장 장면 불러오기** reads local UTF-8 Scene YAML (at most 64 KiB), without upload.
Shared Scenario/Campaign admission replaces a valid session in ready state and
clears pending inputs. Rejected/superseded loads preserve the game. Restart keeps
authored gold, reactor HP and final wave. Authored sources use the default seed;
legacy `defense_init(seed)` still honors its caller.

ABI: `defense_scene_capacity()` is 65536; copy into `defense_scene_buffer()` and
call `defense_init_scene(length)`: 0 success, 1 excessive length, 2 invalid UTF-8,
3 rejected Scene. Failure preserves the client, checksum and source identity.
The picker does not automatically persist a selected file into exported HTML.

## Deliver authored content offline

```sh
python3 tools/pack_coop_defense_offline.py BUILD_DIR \
  --scene saved.scene.yaml --animations animations.json --output new-game.html
```

Both authored options are optional and compose independently. Either requires an
explicit **new** output. The historical no-option command remains supported.
Build-directory JavaScript/WASM must be trusted; authored JSON/PNGs are data.
Node validates scenes against locally built WASM and exercises the exact
snapshotted browser sampler. Animation admission additionally requires installed
Pillow; its PNG verification **and full pixel decode** run before publication.
Inputs are unchanged. Publication is atomic/no-clobber; missing, changed-during-read,
invalid, symlink or nonregular inputs leave no new output. Files replaced after a
completed snapshot cannot substitute bytes later: validation and embedding reuse
that snapshot. Hashes identify content, not authorship. Authored startup requires
WebCrypto and fails visibly on missing/corrupt payloads, never silently falling
back. The generated inventory is checked together before animation adoption.

### Existing browser/FPS manifest contract

`animations.json` is the existing role-keyed JS format, **not** the unintegrated
canonical/native integer-ms sprite format or a converter:

```json
{"unit:0":{"image":"hero.png","pivot":[16,32],"clips":{
  "idle":{"frames":[[0,0,32,32],[32,0,32,32]],"fps":8,"loop":true}}}}
```

Images resolve only under `BUILD_DIR/assets/`. Names are case-sensitive relative
`.png` paths, at most 128 ASCII characters; slash-separated components start with
alphanumeric/underscore/hyphen and otherwise allow those characters plus dots.
URLs, absolute paths, traversal, symlinks and manifest source indirection are
rejected. Only referenced images are added; the default two atlases remain.

Limits: 64 KiB UTF-8 manifest; 1–11 roles (`unit:0..5`, `enemy:0..4`); at most 11
images, 8 MiB each/32 MiB total, dimensions 1–4096 each. Static PNG only, no APNG.
Clips are `idle/move/attack/hit/death`, each 1–256 rectangles `[x,y,w,h]` fully
inside its image (finite coordinates, positive sizes). Optional `fps` is 1–120;
`loop` is boolean. Definition `anchor` pairs are bounded ±16, `pivot` ±4096,
`logicalSize` positive ≤4096; clip `pivots` has one ±4096 pair per frame. Only
attack accepts `windupMs/releaseMs/recoveryMs`, each 1–2000. Omitted fields retain
the existing sampler defaults. Unknown fields, duplicate JSON keys and nonfinite
numbers are errors. Existing served inline/`src` hooks still work, but offline
packaging requires `--animations` rather than a dangling served hook.

## Verification

Run mandatory `bash tools/check-linux-local.sh` and the optional focused
`bash tools/check_coop_defense_web.sh /absolute/output-directory`. The latter
needs Node/Python and installed `@napi-rs/canvas` (`ORR_CANVAS_MODULE` may select it).
`ORR_DEFENSE_WASM` and `ORR_DEFENSE_CAMPAIGN_TRACE` select proven artifacts for
Node tests without rebuilding. Focused checks cover actual WASM/checksum parity,
mock-DOM lifecycle and no-network authored loading, PNG rejection, snapshots,
publication and offline integrity. Tests are not shipped. Real-browser layout,
input/audio, hardware/GPU and other operating systems require separate evidence.
