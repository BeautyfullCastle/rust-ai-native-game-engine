# Regular-grid PNG sprite package

The optional Linux `png_grid_package` example converts one full rectangular
spritesheet into an installable sprite package, without cropping/re-encoding PNG
bytes or manually authoring Sprite JSON. It does not install, edit a project,
create actors, or claim rendering/interactive acceptance.

```sh
cargo run --release -p orr_sample --example png_grid_package --features project-create -- \
  --recipe /path/to/grid.json --out /existing/parent/new-package
```

All options are required once. `--help` prints usage. Other builds fail before
reading arguments or touching files. Paths inside the recipe are relative to its
parent and cannot traverse upward or use symlinks. For example:

```json
{
  "schema": 1,
  "name": "hero-grid",
  "version": "1.0.0",
  "license": "LICENSE.txt",
  "png": "hero.png",
  "cell_width": 32,
  "cell_height": 32,
  "columns": 2,
  "rows": 2,
  "clips": [
    {"id": "idle", "mode": "loop", "frames": [{"cell": 0, "duration_ms": 200}]},
    {"id": "walk", "mode": "once", "frames": [
      {"cell": 2, "duration_ms": 80}, {"cell": 1, "duration_ms": 100}
    ]}
  ]
}
```

Cells have stable zero-based row-major IDs, starting at the top-left. The PNG
must be exactly `cell_width * columns` by `cell_height * rows`. Every cell gets
one region; references can repeat or reorder them. `loop` wraps at total duration;
`once` holds the last frame after completion. All frame durations are positive
integer milliseconds with a non-overflowing total. Clip IDs must be nonempty and
unique. Unknown/duplicate fields and positional-array records are rejected.

The existing SpriteDocument contracts impose at most 2048 pixels per dimension,
4096 cells, 256 clips, 4096 references per clip and 16384 references total. Input
limits are recipe 1 MiB, compressed PNG 16 MiB, decoded RGBA 16 MiB, and UTF-8
nonempty license 64 KiB. No margin/spacing, partial edge cells, trim/rotation,
pivot guessing, resampling, or external editor format is supported.

Output is `sprites.json`, byte-identical `atlas.png`, byte-identical `LICENSE.txt`,
and `orr.package.json`. The existing PNG decoder validates actual dimensions and
format before publication. Package metadata uses the existing sprite capability;
no core/runtime/shared schema/Cargo dependency is added. Install and consume it
through the existing package manager and sprite authoring path afterward.

Filesystem checks and Linux no-replace publication reuse the sequence importer's
private helper. All directories must remain trusted and quiescent: this is not
hostile-filesystem race protection. Staging is renamed atomically without
replacing an existing destination, but parent-directory fsync/crash durability is
not promised. Failure before publication leaves the source and existing output
unchanged.

Focused CPU tests share the example harness to avoid linking unrelated binaries:

```sh
cargo test --release -p orr_sample --features project-create --example png_grid_package
```

The integration target contains the same cases and is not additional coverage.
Run existing `png_sequence_package` tests in the same build batch to verify the
shared helper extraction. CPU package/clip checks alone are not proof of editor
Save/reopen/Play, GPU pixels, or native input. Those acceptance stages must be
reported separately with actual evidence.

Format reference: [W3C PNG Third Edition, Recommendation 2025-06-24](https://www.w3.org/TR/png-3/)
defines both static and animated raster images. This tool accepts static sheets
only; animated PNG is intentionally unsupported, not invalid PNG in general.

## Focused production-pixel acceptance

The explicit ignored test `grid_cli_installed_locomotion_twelve_capture_pixels_and_frame_parity`
uses the real CLI, installed snapshots, `PreparedRuntime`/PlayHost and production
`ProjectCompositor`. A 2x2 grid of internally asymmetric 2x2 cells covers idle Loop
and walk Once boundaries, return-to-idle reset and one wrong-cell-order control.
It records exactly 12 Vulkan readbacks while checking authoritative frame bytes
and checksums on every tick (54 checks across normal and negative runs). Expected
pixels come from literal input texels, not generated regions or prior captures.
No 675-image matrix or native widget acceptance is implied.

Explicit execution requires `ORR_REQUIRE_GPU=1`, `ORR_PNG_GRID_PACKAGE_BIN` pointing
to the built CLI, and `ORR_GRID_PIXEL_EVIDENCE` naming a new directory whose parent
exists. Run this test with `--ignored --exact`; missing CLI/GPU or evidence reuse
fails. The default ignored state is not GPU success. Keep raw captures and receipt,
and report the actual adapter (including software rendering) and execution result.
