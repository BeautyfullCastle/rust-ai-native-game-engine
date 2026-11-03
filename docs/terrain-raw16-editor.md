# Import and export RAW16 heights in the Editor

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
filesystem sandbox. The source cannot alias the current `.orrt` target. Import does not publish files or automatically Save. Export uses a separate
explicit destination and the bounded no-overwrite publisher described below.

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

## Exact-height export

Within **Import RAW16 heights**, the offset, full-range scale, endian and flip-Z
controls are explicitly shared by import and export. Set a separate absolute
`.raw` output path and press **Export RAW16 heights**. The attached writable
terrain must already have a saved backing `.orrt` file; new unsaved terrain must
be saved first. Subsequent dirty in-memory height edits are exported without
requiring Save. The same 4096-vertex Editor limit applies; the converter's
129×129 capability does not enlarge that limit.

For each current height, export chooses the smallest unsigned 16-bit sample that
maps back to that exact fixed-point height under the displayed recipe. A height
with no matching sample is rejected before publication, never clamped or
quantized. For zero scale, every height must equal the offset and canonical
samples are zero. Negative scale is invalid. Endian encoding and optional Z-row
reversal use the same explicit convention in both directions. This is an exact
height inverse, not a promise to recover original RAW bytes: several samples may
map to one height. RAW carries neither holes nor identity, X/Z origin, spacing,
dimensions or other terrain metadata. Keep the recipe separately. Importing into
the same existing terrain preserves its non-height metadata; exporting is not a
whole-terrain serialization inverse.

Export pins in-memory cooked bytes, recipe, source, scene and scene-load epoch
separately from bounded saved scene (1 MiB maximum) and terrain file snapshots.
Saved bytes need not equal dirty memory. Both backing files are checked before
staging and again before final publication. Context, settings, memory, Play,
scene changes and changed backing bytes refuse publication. Trusted stable local
ancestry is required; symlink ancestry, nonregular or oversized backing files
are refused. These checks do not claim protection against hostile concurrent
filesystem replacement.

The destination must not exist. Publication never overwrites an existing file.
Success and failure leave document history, dirty state and any import preview
candidate unchanged. A post-publication durability or cleanup warning means the
file was published; do not retry that destination as though no output exists.
Export does not automatically Save or modify the scene or backing terrain.

The focused CPU/egui acceptance uses the actual Export button on dirty heights,
then existing Preview/Apply, Undo/Redo and Save/Reopen, checking literal heights,
model Y positions and retained hole/identity/XZ data. It also covers refused
representations, existing outputs, stale context and external backing changes.
This is not a physical GPU or native-window execution claim.

[Unity's heightmap workflow documentation](https://docs.unity.cn/Manual/terrain-Heightmaps.html)
is workflow context for explicit RAW parameters only, not a claim of Unity file
compatibility or scene interchange.
