# Rust sprite package creation (optional)

Build `cargo build --release -p orr_package --bin orr_pkg --features sprite-authoring`.
The Linux command directly shares the existing PNG decoder and SpriteDocument
admission; no Python or separately supplied packer executable is needed:

```sh
orr_pkg create-sprite --png atlas.png --license LICENSE.txt --name hero \
  --version 1.0.0 --out new-package --atlas-size 8 4 \
  --region 10 1 0 2 3 --region 20 5 1 3 2 \
  --clip idle loop 10:200 --clip attack once 10:80,20:120,10:80
```

Alternatively replace `--atlas-size`/all `--region`/`--clip` arguments with
`--json sprites.json`. This original-document route copies its bytes exactly,
including whitespace; mixing routes is rejected. Explicit rectangles use top-left
pixels and clips use per-frame **duration_ms**, not FPS. No pivot, FPS, shell,
`--packer`, installation, runtime activation or preview option is supported.
The feature is default-off. Legacy Python and `png_sprite_package` flags remain.

Successful output is the same four files as the old producer: `sprites.json`,
the document's portable PNG leaf (explicit mode: `atlas.png`), `LICENSE.txt`, and
`orr.package.json`. Generated JSON preserves Python's two-space pretty layout,
UTF-8 characters and trailing newline. Original PNG/license bytes and the fixed
manifest (`engine: ^0.0.1`, capability `sprite`) are unchanged. License text is
user-provided, not rights verification.

Existing bounds and semantics remain: argv/JSON 4 MiB, PNG 16 MiB, license 64 KiB,
atlas dimensions 1..2048, decoded RGBA 16 MiB, regions4096/clips256,
4096 frames per clip/16384 total. Invalid rectangles/references/durations,
unknown fields, APNG, dimension mismatch and malformed PNG are refused. PNG17
EXPAND/STRIP_16 and RGB/gray-to-RGBA conversion are unchanged; no new strict
trailing-byte policy or whole-process allocation/time sandbox is claimed.

Inputs/output ancestors must be trusted, stable and symlink-free. Existing
outputs (including dangling links) are refused. The existing producer claims a
new directory before writing four files: publication is **visible-partial, not
atomic or crash-durable**. A write failure now preserves the partial directory
and any foreign content for manual inspection instead of recursive deletion.
Do not blindly retry, overwrite or delete after this error. Admission failures
before directory creation leave no output. This conservative cleanup change is
intentional; successful bytes/digests remain compatible.

Focused differential tests exercise original Python+native producer versus the
new Rust frontend and the actual installed Arena consumer after hiding inputs.
RGBA and clip sampling are CPU evidence, not GPU playback/visual certification.
Only Linux execution is claimed; retained Windows helper tests are not an actual
Windows execution result. The frozen sprite candidate's full path inventory is
still unavailable; this change does not replace or publish that candidate.
