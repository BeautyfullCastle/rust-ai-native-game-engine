# Optional bounded heightfields

`orr_terrain` is a GPU-free opt-in asset library. No existing engine default,
scene, renderer, physics, editor, or network dependency is changed. `orr_terrain_view`
is the optional presentation/package demonstration adapter.

## Coordinates and surface contract

- Y up, XZ grid, suggested one unit = one metre. Origin is `[X,Z]`; heights
  and all coordinates use `orr_fp::FP` Q48.16 raw `i64` values.
- Width/depth are vertex counts, each 2–129. Spacing is one strictly positive
  FP value shared by X/Z. Heights have exactly width × depth elements; holes
  have exactly (width−1) × (depth−1) boolean elements. Both are row-major X-fast.
- All FP raw values are legal if both far-edge coordinates fit `i64`.
  Construction/load reject invalid extents, counts, IDs and spacing. Wide
  integer arithmetic avoids overflowing intermediate coordinates and heights.
- Cell corners are a=(x,z), b=(x+1,z), c=(x,z+1), d=(x+1,z+1).
  Every non-hole cell emits `[a,c,d]` then `[a,d,b]`, with upward winding.
  The diagonal is always a–d. Queries use the corresponding piecewise plane,
  with the first triangle owning the diagonal. This is not bilinear interpolation.
- Integer barycentric weights sum to spacing. Height rounds once toward
  negative infinity at the raw FP unit. No prior fraction quantization occurs.
- The closed outer rectangular boundary belongs to the final cell. Internal
  grid seams belong to the cell on the +X/+Z side. Hole queries return None,
  including boundaries owned by that hole. There is no fallback to a neighbour.
- `surface` reports the same height and selected plane's deterministic normal
  and rise/run slope (not angle). Normals scale wide coefficients to 30 bits,
  integer-square-root their length and truncate into Q16. Extremely steep
  normals may quantize Y to zero. Slope truncates the two Q16 gradients before
  integer square root; `None` slope means it cannot fit FP, not a missing surface.
  A missing surface is the outer Option. Queries are not rigid-body collision.

## Canonical persistence and identity

The format is little endian and has no padding:

1. Eight bytes `ORRTHF`, version byte 1, reserved byte 0
2. u16 asset-ID byte count and UTF-8 asset ID, at most 256 bytes
3. u32 width, u32 depth
4. i64 raw origin X, origin Z, spacing
5. width × depth i64 raw heights
6. (width−1) × (depth−1) one-byte holes, each strictly 0 or 1

IDs are relative logical names with ASCII letters/digits, `_`, `-`, `/`, spaces
and dots. Empty, absolute, empty-segment, `.`/`..` segment IDs are rejected.
Trailing bytes, alternate hole encodings, unknown versions, truncation and
oversize files fail. The maximum file size is the exported `MAX_FILE_BYTES`
(149,810 bytes). Load checks limits and exact lengths before allocating arrays.

The stable asset ID is independent of content. Revision is SHA-256 of the whole
canonical asset, including ID. Revision is derived, never trusted from storage.
No clock, random seed, architecture-sized serialized integer or float is used.

## Editing

`TerrainDocument` accepts at most 4,096 vertex/hole operations per transaction.
It validates a private candidate, then replaces content atomically. Rejection
preserves bytes, revision, undo/redo and dirty region. Duplicate operations run
in list order; net no-ops consume no history and preserve redo/dirty state.

There are at most 32 snapshot history entries total across undo and redo,
roughly 8 MiB at maximum dimensions. Oldest undo entries are evicted on overflow.
Undo/redo restores exact revisions. Save/reopen loads canonical content, not
session history. New edits after undo discard redo.

Dirty regions are inclusive cell rectangles: a vertex change includes all
adjacent cells and holes dirty their cell. The last successful content change
replaces the region; callers may clear it. It is not an accumulated upload queue.
Undo/redo conservatively dirties the full grid. The view adapter rebuilds an
immutable mesh; regional uploads are not claimed.

## Verification

`cargo test -p orr_terrain --release --locked --offline -j2`

Tests cover an independent triangle-area interpolation oracle (including a
non-bilinear saddle), negative coordinates, grid/diagonal/outer boundaries,
holes, malformed and every truncated prefix, overflow/extreme values,
canonical hash/roundtrip, rejected transactions, bounded history,
undo/redo/save/reopen and dirty regions. See `orr_terrain_view` for the actual
package and required-GPU acceptance run.

This finite vertical slice does not implement scene depth composition, shadows,
streaming, LOD, collision, navigation, erosion, vegetation or editor painting.
