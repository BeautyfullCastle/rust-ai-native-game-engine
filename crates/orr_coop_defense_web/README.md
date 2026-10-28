# Optional CoopDefense browser adapter

This delivery crate compiles `orr_games/coop-defense` into WebAssembly. JavaScript
handles input, drawing and optional synthesized audio, never combat, economy or
the AI partner. The portrait six-role game is a temporary validation fixture,
not a claim of original-game fidelity or a completed remake.

## Build and use

Install the official Rust `wasm32-unknown-unknown` target, then run
`bash tools/build_coop_defense_web.sh /absolute/output-directory`. Serve that
folder over HTTP(S), or open `stormglass-foundry-offline.html` without a server.
The package includes both ES modules; offline packaging embeds their closed
import graph and exact WASM/art bytes. There are no third-party runtime services,
accounts or relay requirements. WebAssembly and Canvas2D are required.

Start, summon (Q), move by drag or two taps, merge matching role/tier units,
upgrade (E), invest (R), pause (Space), and restart. The right board is local AI.
Ready/pause/hidden states freeze presentation time. Audio is muted by default and
requires a trusted gesture. Current character images use procedural fallback
motion, not authored animations; audio cues are synthesized, not final design.

## Engine contract

`defense_init` creates a new source instance. `defense_step` advances one fixed
60 Hz tick. Copy snapshot UTF-8 bytes before another snapshot/reset, never retain
a WASM memory view across allocation. Restart uses the authoritative RESTART input
with a higher sequence and the same seed.

The strict consumer reads stable actor IDs and actual engine event keys after
every step, not HP/cooldown differences or nearest-projectile guesses. Missing,
unsupported or malformed contracts fail closed. Gaps, restart, overflow and
source changes resync without invented effects. Windup is unsupported. See the
[producer/consumer contract](../../docs/coop-defense-presentation.md).
`window.defenseDebug` exposes development helpers, not autoplay or new rules.

## Verification

Run `bash tools/check-linux-local.sh` for mandatory repository acceptance and
`bash tools/check_coop_defense_web.sh /absolute/output-directory` for this optional
path. The latter requires Node and Python, plus an already installed
`@napi-rs/canvas` for the native-rendering oracle. Set `ORR_CANVAS_MODULE` to its
module path if it is not resolvable normally. Set `ORR_DEFENSE_PRIOR_WASM` to the
preserved pre-event WASM to additionally compare legacy gameplay fields.

The focused gate builds current WASM, validates its ABI, tests the strict event
stream, pause lifecycle and atlas clipping, checks mock-DOM gestures, and verifies
offline payloads and JavaScript syntax. `ORR_DEFENSE_WASM` selects that freshly
built artifact for direct Node tests; its hash is logged, not tied to one compiler.
Tests/fixtures are not shipped in the playable package. Browser layout/input,
acoustic audio, cross-platform and hardware/GPU claims need separate evidence.

## Saved Scenario playback

Use **저장 장면 불러오기** to select a local UTF-8 Scene YAML (at most 64 KiB).
No file contents are uploaded. Admission reuses `orr_games/coop-defense-scene`;
only the closed Scenario starting conditions are accepted, with both authored
fields required. Success replaces the session in ready state and clears pending
inputs. Rejected or superseded asynchronous loads do not replace the game.
Restart retains the imported starting gold/reactor HP, including the HUD maximum.
Imported scenes use `DefenseConfig::default().seed` for native/ERP parity; the
legacy `defense_init(seed)` ABI still honors its caller's seed.

ABI: `defense_scene_capacity()` returns 65536, `defense_scene_buffer()` returns
adapter-owned staging memory. Copy into a fresh memory view, then call
`defense_init_scene(length)`: 0 success, 1 excessive length, 2 invalid UTF-8,
3 rejected Scene. No external pointer is accepted. Failure preserves the full
client, checksum and presentation source identity. Snapshot `maxHp` is authored.
The offline exporter includes this same picker; it does not automatically embed your selected
scene or persist it across page loads.


### Deliver a saved scene offline

`python3 tools/pack_coop_defense_offline.py BUILD_DIR --scene saved.scene.yaml --output authored-game.html`
validates the exact saved bytes using the trusted local `game.wasm` and Node, embeds
an initial-state/hash receipt, and atomically creates a **new** single-file game.
The authored scene opens ready at tick zero and survives restart. Existing output
paths, symlinks, nonregular or oversized inputs are rejected; the input is untouched.
No scene upload is involved. Hashes identify content, not authorship. WebCrypto is
required for authored startup integrity; unavailable verification fails closed.
See [the authoring contract](../../docs/stormglass-authoring.md).
