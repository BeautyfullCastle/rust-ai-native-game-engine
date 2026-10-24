# Original tower art provenance

`web/units.png` was generated for this project on2026-10-24 with the built-in image generation tool, then copied unchanged from `stormglass-towers-v2.png`. No commercial game's images, characters, logos or audio were extracted or incorporated.

Design brief (summary, not a verbatim generator prompt): six distinct original industrial-garden defense machines, painted bronze and cream ceramic with teal glass and moss, coherent isometric/top-down game silhouettes, transparent PNG atlas. Roles are gunner, mortar, slowing crystal, armor-breaking hammer, haste/support resonator and miner. No text or borrowed characters.

`web/units.json` records image dimensions, per-sprite source rectangles and the core-role mapping. The generator did not honor equal grid padding, so the renderer uses explicit rectangles and aspect-fit bottom-center anchoring. Alpha compositing was inspected on the actual game background; no opaque atlas background or neighboring sprites were observed. Vector art remains the fallback if the PNG fails to load.

This is newly generated project artwork, not a downloaded stock asset or a third-party game asset. No third-party asset license or exclusivity claim is asserted. Game code licensing remains governed by the repository's licenses.

## v6 forest characters

The active browser character assets are `web/assets/units.png` (unchanged copy of `forest-heroes-v2.png`) and `web/assets/enemies.png` (unchanged copy of `forest-enemies-v1.png`), generated originally for this project on2026-10-24 with the built-in image generation tool. The revised design uses friendly fantasy forest defenders and distinct fantasy enemies rather than the v5 industrial turrets. No raster edits were performed during integration. `web/assets/characters.json` contains the exact source rectangles; the renderer owns role mapping and animations. Offline packaging embeds both PNG byte streams and that JSON under `tower-atlas`, `enemy-atlas` and `character-atlas-meta`, respectively. The earlier v5 art remains documented above; it is not the active v6 character atlas.

## Strict-event integration and test-only clip fixture

The v6 forest PNGs and rectangle JSON remain byte-identical in this integration;
the retained v5 files remain inactive. The v7 portrait layout and later strict
consumer changes do not replace that art or certify the reference game's map,
roster, economy or rules. No historical v9 map fixture is included.

`web/tests/fixtures/moss-spearfighter-v3.png` and its animation manifest are copied
unchanged from the separate v8/v10 generated goblin playback fixture. They test
30 atlas frames across five clips, with estimated contact pivots and known row
size drift/gutter noise. This is project-generated temporary art, not extracted
commercial-game art or a finished roster. It is never loaded by game.js or copied
into the playable/offline package. Its SHA256 is
`8c386973fbd5543b5abd9ce1e3c7e717187fd25bd9fde6388dae7d980671c69c`.
