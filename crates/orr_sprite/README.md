# orr_sprite

Optional GPU-free, view-only sprites. Empty default features; the only dependencies
are serde and serde_json. No engine simulation, renderer, RHI, clock or image
cooker dependency. Existing Shape/Style and ORAM/motion/PCM formats are unchanged.

## Authoring and consuming version 1

See `assets/sprite_demo/sprites.json` for a complete authorable document and
`assets/sprite_demo/generate.py` for original, reproducible four-frame artwork.
The PNG is the author-facing original; `.rgba` contains exactly the same pixels,
64 by 16, tightly packed top-to-bottom straight-alpha RGBA8 for direct upload.
Artwork and generator are MIT licensed (the adjacent LICENSE.txt).

```rust
use orr_sprite::{SpriteDocument, SpriteInstance};
let json = include_str!("../../assets/sprite_demo/sprites.json");
let document = SpriteDocument::from_json(json)?;
let walk = document.clip("walk").ok_or("missing walk clip")?;
let sampled = walk.sample(140); // Presentation elapsed milliseconds, not a sim tick.
let sprite = SpriteInstance {
    region: sampled.region,
    position: [10.0, 5.0],
    size: [2.0, 2.0],
    ..Default::default()
};
# Ok::<(), Box<dyn std::error::Error>>(())
```

`SpriteSource`, `Atlas`, `Region`, `ClipSource` and `Frame` are unvalidated authoring
data. `SpriteDocument::new` validates them; the document exposes immutable data.
JSON requires every declared field, rejects unknown/duplicate fields and unsupported
format/version/mode values, and uses unsigned integer IDs, dimensions and durations.
The root format is `orr_sprite`, version `1`; it is a separate asset document, not an
extension to simulation, ORAM, motion or PCM schemas.

Regions use top-left pixel coordinates and positive dimensions inside the atlas;
addition overflow, duplicate region/clip IDs, empty clips/IDs, missing references,
zero frame durations and total-duration overflow are rejected. Region IDs and clip
names are stable independent of array order. The atlas path is inert metadata; this
crate performs no filesystem I/O or path resolution. Applications must define their
own asset root/path policy and check the decoded pixels match declared dimensions.

Time sampling is pure: intervals are half-open, looping clips wrap at total duration,
and once clips hold the final frame and report `finished` at/after total duration.
`Playback` is an optional independent cursor with seek/reset and saturating advance.
Changing clips does not change a cursor implicitly; call reset when desired. Large
elapsed values use modulo without iteration through skipped animation cycles.

Instances are plain presentation inputs: center position, full size, radians,
straight RGBA tint, horizontal/vertical flips, and signed painter order. Renderers
validate finite numeric values and IDs, and preserve input order for equal `order`.
UV bounds are top-left normalized coordinates; flips are renderer operations.
Instances do not mutate gameplay, own entities, or modify simulation checksums.

The package itself does not prove a complete player-facing authoring workflow:
the optional renderer and playable sample must load this document, upload the actual
pixels and draw sampled regions. GPU capture and Windows-local checks are separate.

## Checks

`cargo test -p orr_sprite --release`

`cargo clippy -p orr_sprite --all-targets --release -- -D warnings`

## Resource bounds

Version 1 limits JSON to 4 MiB **before parsing**. Both loading entrypoints check
image names <=1024 UTF-8 bytes, clip IDs <=128 bytes, <=4096 regions, <=256 clips,
<=4096 frames per clip and <=16384 total frames (checked addition), before creating
runtime indexes. Both atlas dimensions must be 1..=2048, matching the portable
renderer policy. Limits are inclusive. `new` takes already allocated authoring
data; it does not bound caller allocations. Runtime clips take ownership of frame
arrays without duplicating them. Asset cloning explicitly requested by a caller
still clones its contents.

## Generate a new regular sprite sheet document

`grid::generate(Atlas, GridSpec, GridClip)` generates every requested cell in a
regular grid and one contiguous clip, then returns a validated `SpriteDocument`.
Coordinates start at the **top-left**, increasing right and down. IDs increase
left-to-right, then top-to-bottom from `first_region_id`. `first_cell` is a
zero-based cell index, independent of that ID; `frame_count` selects consecutive
cells and `duration_ms` applies to every selected frame. Only existing `Loop` and
`Once` playback modes are used. Unused border pixels are permitted, but no requested
cell is clipped or discarded. Offset precedes the first cell; spacing lies only
between cells and has no effect on a singleton axis.

The stdout-only CLI is automatically discovered by Cargo, with no extra feature:

```sh
cargo run --release -p orr_sprite --bin orr_sprite_grid -- \
  --image sprites/hero.png --atlas-width 37 --atlas-height 23 \
  --columns 3 --rows 2 --cell-width 7 --cell-height 5 \
  --offset-x 4 --offset-y 3 --spacing-x 2 --spacing-y 4 \
  --first-region-id 91 --clip walk --first-cell 1 --frame-count 4 \
  --duration-ms 13 --mode loop
```

This produces region IDs 91..=96 and a clip referencing 92, 93, 94, 95. Use `--help`
alone for all flags. Offset, spacing, first region ID and first cell default to
zero; all other flags are required. Numeric values use unsigned decimal digits.
Flags may appear in any order, but duplicates, unknown flags, missing values,
invalid UTF-8 and invalid documents fail with exit 2 and stderr, without JSON on
stdout. Success emits the exact deterministic `SpriteDocument::to_json()` bytes
plus one newline. Help prints usage instead of JSON. An output-stream failure
returns exit 1.

No image or document is opened or written: `--image` is inert metadata, even for
an absent or unreadable path. Save stdout yourself only to a chosen **new** asset;
a shell redirect can overwrite a file before the program validates its arguments.
The consuming application must still enforce its asset-root policy, decode the
image, and verify pixel dimensions. This generator does not install packages,
assign bindings, update editor history, or re-slice existing bound assets. Layout
changes can change ID meaning; do not replace an already-bound document with its
output or assume that existing IDs will be preserved.

Names, counts, last rectangle edges, final IDs, clip range and total duration are
checked before generated region/frame allocation, using the existing version-1
limits above. One clip can reference at most all 4096 generated cells, below the
schema's aggregate frame limit. The original loader and runtime sampling are
unchanged. Tests cover asymmetric coordinates, exact limits/overflow, byte limits,
round-trip/sampling, CLI equivalence/error channels and no file mutation. This is
a GPU-free document-authoring tool, not an image slicer UI or complete editor
workflow; image rendering, native interaction and export acceptance are separate.
