# Compose game animation roles

The optional Stormglass web tools can combine individually authored unit/enemy
manifests into one file consumed by the existing offline packer. This changes no
simulation, runtime format, native SpriteDocument, or renderer API.

Create each role with `tools/create_coop_defense_animation.py`, or supply existing
valid role manifests. All image names are relative to the **same explicit assets
root**, not to each manifest's directory. The composer never rebases or copies
images, guesses role mappings, or silently replaces a duplicate role.

```sh
python tools/compose_coop_defense_animations.py \
  --assets-dir build/web/assets \
  --manifest art/hero.webanimations.json \
  --manifest art/enemy.webanimations.json \
  --output build/game.webanimations.json
python tools/pack_coop_defense_offline.py build/web \
  --animations build/game.webanimations.json --output build/game.html
```

The output's parent must exist; existing outputs and symlink paths are rejected.
One to eleven input files and the existing eleven role keys are supported.
Strict JSON rejects duplicate keys and nonfinite numbers. Each input and the
combined output are bounded to 64 KiB. Distinct PNGs are read once from the common
root, limited to 8 MiB each and 32 MiB total. Existing PNG decoding, rectangle
bounds, and FPS sampler admission run before atomic no-overwrite publication.
All existing clip timing, frame pivots, anchors and logical sizes are retained.
Output bytes are deterministic regardless of input order. The JSON receipt lists
the output hash, image hashes, sorted roles, and input hashes in command order.

Acceptance: focused Python tests cover composition, ordering, duplicate role/key,
limits, paths, PNG bounds, CLI and no-clobber. The dedicated actual test consumes
CLI-generated multi-role output through the unchanged offline packer and trusted
historical WASM with bundled JavaScript: live unit and enemy frames/pivots,
unchanged gameplay checksums, Pause and Restart, and no network. Its DOM/Image/
Canvas are mocks, not browser rendering, hardware GPU or a new WASM build.
An unverified or failed run is not acceptance. Role composition does not supply
missing artwork, animation states, licensing evidence, or a complete roster.
