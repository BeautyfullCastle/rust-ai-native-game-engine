# Apply RAW16 heights to existing Editor terrain

Enable the optional Editor feature `terrain-raw16` (default off). It includes
`terrain` plus the existing converter exposed through `orr_terrain_view/raw16`;
it does not activate a gameplay package or install content. The original
`orr_terrain_import` CLI and public API still create new `.orrt` assets unchanged.

Open writable scene terrain, expand **Import RAW16 heights**, and supply an
absolute input path, unsigned 16-bit byte order, optional Z-row reversal, signed
height offset and nonnegative full-range scale. **Preview RAW16 heights** reads
and validates the bounded source without changing terrain. Inspect its dimensions,
changed count and exact decimal range. **Apply RAW16 heights** rereads the source
and commits changed heights in one terrain Undo step. **Save terrain** is separate.
Installed terrain remains read-only until explicitly copied to the scene.

This replaces heights, not topology: current dimensions, X/Z origin, spacing,
identity and hole flags are preserved. Dimensions must match the raw samples;
there is no resampling, rectangle placement or inferred square size. The entire
current footprint, including no-ops, must be at most 4096 vertices. The offline
converter's larger 129×129 limit does not expand the Editor's transaction budget.

## Preserved conversion and input contract

The exact existing converter and input reader are reused in-process, not through
a subprocess or second decoder. Samples are unsigned `u16`, X-fastest, explicitly
LE or BE, with exactly `2*width*depth` bytes and no header/trailer. For each sample:
`height_raw = offset_raw + floor(sample*scale_raw/65535)`.
Offset/scale use the CLI's integer-only `FP::parse` decimal grammar and rounding;
Editor text fields are additionally bounded to 128 UTF-8 bytes each. Scale cannot
be negative. Overflow, invalid dimensions, inexact f32 display positions/heights,
and coordinates outside ±1,000,000 are rejected, never clamped or rounded for
rendering. The source path is additionally required to be absolute and at most
4096 UTF-8 bytes. Original input remains untouched.

The existing reader rejects symlink ancestry, traversal, nonregular files and
wrong lengths, and compares opened-file metadata while taking a bounded snapshot.
Input ancestry must remain trusted and stable; this is not a hostile concurrent
filesystem sandbox. The source cannot alias the current `.orrt` target. No output
publisher, permissions, file deletion or automatic Save is added by this UI.

## Preview validity and acceptance

A private preview binds the source path and bytes, all recipe controls, exact
current terrain bytes/source and scene epoch. Editing controls or a failed
Preview revokes the old preview. Every Apply attempt consumes it: after an error,
Preview explicitly again. Changed source/terrain, switched scene, Play/preview,
read-only or failed renderer admission leaves document/history/files unchanged.
Re-reading identical source bytes at a replaced inode is not an authenticity
claim. Dirty current terrain may be previewed; Undo returning to identical
content permits reuse because the revision is a content hash, not a monotonic
edit counter. Apply does not modify the live scene/Frame or certify navigation.

Focused acceptance exercises actual Preview/Apply widgets, immutable source and
pre-Save target bytes, holes/XZ/identity and model heights, single Undo/Redo and
Save/Reopen; stale input/context/recipe, budget and failed admission cases; and
original CLI conversion/publication plus feature isolation after API extraction.
CPU/egui/model data evidence is not physical GPU or native-window execution.
