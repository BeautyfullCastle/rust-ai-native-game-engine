# Stormglass web animation authoring

Create a new one-role `animations.json` without hand-writing JSON. This optional
CoopDefense web adapter tool is not the engine's native `orr_sprite` editor or
an importer/converter. Python 3.11+, Pillow and Node are required for admission;
no WASM build or simulation is needed to create the manifest.

```sh
python3 tools/create_coop_defense_animation.py \
  --assets-dir build/coop-defense-web/assets --output animations.json \
  --role unit:0 --image authored/hero.png \
  --frame idle 0 0 32 32 --frame idle 32 0 32 32 \
  --clip idle 10 loop --pivot 16 28
python3 tools/pack_coop_defense_offline.py \
  build/coop-defense-web --animations animations.json \
  --output game-with-animation.html
```

Use an existing trusted build and put the PNG beneath its `assets` directory.
Creation reads but never modifies/copies that PNG. The packer re-admits its own
snapshot, so a changed image must still pass packing; the creation receipt hashes
identify the originally admitted manifest and PNG, not a future file guarantee.
Both output files must be new and their parent directories must already exist.
Existing files, symlinks and dangling symlinks are never overwritten.

## Contract

- Exactly one `--role`: `unit:0` through `unit:5`, or `enemy:0` through `enemy:4`.
- Repeat `--frame CLIP X Y W H` in playback order. Supported clips: `idle`,
  `move`, `attack`, `hit`, `death`; 1–256 frames per clip. Coordinates may be
  fractional, must be finite/nonnegative and remain inside the fully decoded
  PNG; width and height must be positive. PNG limits match the offline packer.
- Optional `--clip CLIP FPS loop|once` sets one existing clip, once only. FPS is
  finite, 1–120. Omission preserves the existing sampler defaults: 10 FPS,
  looping idle/move, one-shot attack/hit/death.
- Optional `--pivot X Y` sets a shared pixel pivot (each coordinate −4096…4096).
  Omission retains the sampler's default anchor `[0.5, 0.86]` and no pixel pivot.
- Image references use bounded relative `.png` paths; traversal, absolute paths,
  symlink files/directories and non-regular files are rejected. Captured bounded
  PNG bytes are CRC-checked and fully decoded; the existing sampler admits the
  exact manifest bytes that are atomically published without replacement.

This first bridge does not expose multiple roles, per-frame pivots, anchor,
logical size or attack timing. It reads no existing manifest, so it cannot
silently discard those fields. Existing advanced manifests remain usable with
the offline packer. Other roles retain their existing fallback presentation.

Acceptance covers focused invalid-input/no-clobber checks and generated-manifest
packing. Actual WASM + Node mock DOM/Image/Canvas checks can establish actor frame
selection and unchanged simulation checksums, not browser pixels or GPU coverage.
