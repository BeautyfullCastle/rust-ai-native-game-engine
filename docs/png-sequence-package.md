# Offline PNG frame-sequence package authoring

`png_sequence_package` builds one sprite clip from an explicitly ordered sequence
of PNG images. Schema 1 requires equal-sized frames; schema 2 explicitly registers
variable-sized frames in a common canvas. It is an optional Linux authoring example, enabled by
`project-create`. The existing PNG + Sprite JSON importer remains unchanged.

```sh
cargo run --locked -p orr_sample --features project-create \
  --example png_sequence_package -- --recipe sequence.json --out new-package
```

The output parent must already exist and the output directory must not exist.
Both options are required exactly once; `--help` must be used alone. Without the
feature, or on unsupported platforms, the command refuses before source/output
I/O. No new feature, runtime API, dependency, host or renderer is introduced.

## Schema 1 recipe (unchanged)

```json
{
  "schema": 1,
  "name": "my-animation",
  "version": "1.0.0",
  "license": "LICENSE.txt",
  "clip": "run",
  "mode": "loop",
  "columns": 2,
  "frames": [
    {"png": "frames/first.png", "duration_ms": 30},
    {"png": "frames/second.png", "duration_ms": 70},
    {"png": "frames/third.png", "duration_ms": 20}
  ]
}
```

Paths resolve relative to the recipe's directory. Use portable relative paths;
absolute paths, parent traversal and symlink components are rejected. Array order
is animation order, regardless of filenames. `mode` is exactly `loop` or `once`.
Columns must be between one and the number of frames. Durations are positive integer milliseconds whose sum must fit an unsigned 64-bit
integer. Unknown fields and unsupported schema versions are errors. Supply your
own license or permission text for your own images; the tool does not infer rights.

Frames are decoded using the existing runtime PNG decoder, normalized to RGBA8,
with the existing `EXPAND | STRIP_16` policy (16-bit channels are reduced to 8-bit),
and packed row-major with no spacing, rotation, trimming or resampling. For schema 1, all frame
dimensions must match. Region IDs follow array order starting at zero. Unused
cells on the last row are transparent zero RGBA. No edge extrusion or guarantee
against filtering bleed is provided. The generated clip preserves
frame durations exactly. Animated PNG is rejected; use separate static frames.

## Schema 2: explicit common-canvas registration

```json
{
  "schema": 2,
  "name": "registered-animation",
  "version": "1.0.0",
  "license": "LICENSE.txt",
  "clip": "run",
  "mode": "loop",
  "columns": 2,
  "canvas": {"width": 8, "height": 6},
  "frames": [
    {"png": "frames/first.png", "duration_ms": 30, "offset": {"x": 1, "y": 2}},
    {"png": "frames/second.png", "duration_ms": 70, "offset": {"x": 3, "y": 0}},
    {"png": "frames/third.png", "duration_ms": 20, "offset": {"x": 0, "y": 1}}
  ]
}
```

Schema 2 requires a `canvas` with positive integer `width` and `height`, and an
explicit `offset` on every frame with nonnegative integer `x` and `y`. The origin
is the top-left of each canvas cell: positive x goes right and positive y goes
down. A frame's complete decoded rectangle must fit inside the canvas at that
offset. Nothing is clipped, even a fully transparent out-of-bounds pixel. Missing,
unknown, fractional, negative or out-of-range values are rejected; offsets are
never inferred. Schema 1 rejects these new fields and retains its original strict
shape, equal-size requirement and output bytes for existing recipes.

Every canvas starts at transparent zero RGBA. The complete normalized source
RGBA is copied at its explicit offset, preserving RGB even where alpha is zero;
there is no alpha-based RGB discard or premultiplication. There is no content
bounding-box inference, automatic trim, scaling, rotation or pivot inference.
All generated regions cover the entire common canvas, keeping the same origin
and dimensions throughout the animation. Empty atlas cells remain transparent
zero RGBA. Source order, durations, Loop/Once playback and source immutability
follow the same rules as schema 1.

The authoring recipe version is separate from the output package manifest,
Sprite JSON, project bindings and runtime schemas; none of those schemas or
runtime APIs changes. This is an explicit PNG-sequence workflow, not an Aseprite
file or metadata importer. The official [Aseprite CLI documentation](https://www.aseprite.org/docs/cli/)
and [sprite-sheet documentation](https://www.aseprite.org/docs/sprite-sheet/)
provide contextual background on exported images and sprite sheets only; they
are not an import-compatibility claim.

## Independent bounds

- Recipe: 1 MiB, strict UTF-8 JSON
- Frames: 1–256
- Each encoded PNG: 16 MiB; aggregate encoded frame input: 32 MiB
- Decoded frame RGBA aggregate: 16 MiB; generated atlas RGBA: 16 MiB
- Encoded output atlas PNG: 16 MiB (incompressible RGBA at the decoded cap may exceed it and be rejected)
- Atlas axes: at most 2048 pixels each
- License: 64 KiB of non-whitespace UTF-8 text

Both recipe versions retain every bound above. Decoded-source budgeting uses the
actual decoded size of each source, including repeated references, rather than
the canvas size. Atlas preflight separately checks common-canvas cell dimensions,
row/column layout, axis limits and total atlas RGBA before allocating the atlas.
A small source does not exempt a large canvas from these checks.

Each repeated frame reference counts toward frame and byte budgets. Checked
arithmetic rejects overflowing dimensions and duration totals. These bounds are
not a total process-memory or CPU budget: source buffers, decoded images, JSON,
encoder/decoder allocations and generated bytes can coexist. This is an offline
trusted-filesystem authoring tool, not a sandbox for hostile image decoders.

## Output and filesystem behavior

The new directory contains exactly four regular files:

- `atlas.png`: generated RGBA8 atlas
- `sprites.json`: generated `orr_sprite` document with one clip
- `LICENSE.txt`: the supplied license bytes unchanged
- `orr.package.json`: sprite-capable package manifest

The recipe and input PNG bytes are never modified. Identical content and recipe
produce identical package bytes, including when relocated to another directory.
The command does not install packages, edit projects or bindings, or claim that
runtime rendering occurred. Installation is a separate explicit operation.

Inputs must be regular files. Static symlinks and FIFOs are rejected before
opening; all ancestors must be trusted and stable throughout the operation.
Linux publication uses no-replace directory rename after staging, so an existing
output is never replaced. This does not make ordinary input path checks safe
against malicious concurrent path replacement. A crash may leave staging data;
no crash-durability guarantee is made. Inspect leftover temporary data before
retrying. Never run against directories controlled by untrusted concurrent writers.

## Verification

The private example tests independently check the complete atlas RGBA, transparent
padding, region coordinates, explicit ordering, clip durations and every frame
boundary for Loop and Once. They check repeatable bytes and unchanged sources,
strict versioned schemas, malformed/APNG frames, unequal schema-1 frames, budgets, invalid paths, FIFO/symlink
inputs and output collisions. The original PNG importer tests remain separate.

The Linux acceptance workflow also builds the real CLI, generates and installs
packages, hides original sources, opens the installed project through Arena's
compiled runtime and exercises the production sprite renderer on software Vulkan.
Missing required GPU support fails the acceptance run. Rendering evidence is
separate from the producer's success message; see the retained CI receipts for
the exact tested configuration and outcomes.

Schema-2 acceptance additionally uses unequal asymmetric source images and
explicit offsets in a shared canvas. It checks the full source-known pixel oracle,
transparent padding and zero-alpha RGB preservation, unchanged source/package
bytes, Loop/Once boundaries and real PlayHost playback. A wrong-offset package
must disagree with the correct-registration oracle, so a successful CLI run or
static render cannot substitute for registration evidence. CI retains raw GPU
captures and a receipt, checks their hashes and exact expected capture inventory,
and requires both the original schema-1 GPU test and the separate schema-2 test.
No missing adapter, empty test filter or skipped GPU run is accepted as success.
