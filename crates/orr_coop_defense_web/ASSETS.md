# Original tower art provenance

`web/units.png` was generated for this project on2026-10-24 with the built-in image generation tool, then copied unchanged from `stormglass-towers-v2.png`. No commercial game's images, characters, logos or audio were extracted or incorporated.

Design brief (summary, not a verbatim generator prompt): six distinct original industrial-garden defense machines, painted bronze and cream ceramic with teal glass and moss, coherent isometric/top-down game silhouettes, transparent PNG atlas. Roles are gunner, mortar, slowing crystal, armor-breaking hammer, haste/support resonator and miner. No text or borrowed characters.

`web/units.json` records image dimensions, per-sprite source rectangles and the core-role mapping. The generator did not honor equal grid padding, so the renderer uses explicit rectangles and aspect-fit bottom-center anchoring. Alpha compositing was inspected on the actual game background; no opaque atlas background or neighboring sprites were observed. Vector art remains the fallback if the PNG fails to load.

This is newly generated project artwork, not a downloaded stock asset or a third-party game asset. No third-party asset license or exclusivity claim is asserted. Game code licensing remains governed by the repository's licenses.
