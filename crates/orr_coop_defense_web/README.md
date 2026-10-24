# Stormglass Foundry browser adapter

This optional delivery crate compiles the same `orr_games/coop-defense` deterministic game into WebAssembly. JavaScript only handles input, drawing and optional synthesized audio. It does not simulate combat, economy or the AI partner.

Build: `rustup target add wasm32-unknown-unknown`, then `tools/build_coop_defense_web.sh <output-directory>`. Serve that directory over HTTP(S); it has no third-party dependencies, accounts, relay or installation requirement. Browser support needs WebAssembly and Canvas2D. Native Windows execution is not claimed.

The ready panel holds the simulation at tick zero. Start, summon (Q), select a unit then another cell to move or merge two matching role/tier units, upgrade (E), invest (R), pause (Space) and restart. The right board is explicitly a local AI partner, not an online player.

`defense_init` creates a fresh engine instance, `defense_step` advances one fixed60Hz tick with an input, and `defense_snapshot`/`defense_snapshot_len` expose UTF-8 presentation data. Copy snapshot bytes before the next snapshot or reset; never retain a WASM memory view across allocation. The initial page load initializes the runtime once. Both restart buttons send the authoritative in-game RESTART input with a higher sequence, retaining the seed and sequence history.

Run `node tools/check_coop_defense_wasm.mjs <output-directory>/game.wasm` for actual-WASM economy, move, deduplication, restart, AI and deterministic parity checks. Browser visual and input validation is separate from this ABI check. `window.defenseDebug` exposes manual development snapshot/step helpers; it does not autoplay or change rules.

`node tools/check_coop_defense_web_ui.mjs <output-directory>` exercises the JS adapter with a mock DOM and the actual WASM core: ready gating, action input, pause/resume, restart sequence and bounded input queue. It is explicitly not a browser visual test. Packaging also creates `stormglass-foundry-offline.html`, which embeds the exact WASM and can be opened without an HTTP server.
