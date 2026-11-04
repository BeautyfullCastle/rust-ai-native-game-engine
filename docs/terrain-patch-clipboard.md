# Terrain patch clipboard

Duplicate a small authored terrain feature, including its slope and holes, without
repainting individual vertices. The clipboard belongs to the current terrain
panel; it is not an operating-system clipboard. Explicit JSON assets let you reuse
a captured patch in another session or terrain.

## Use

In local Edit mode, create or open writable terrain and expand **Terrain patch
clipboard** under **Terrain authoring**. Enter an inclusive source **cell**
rectangle and choose **Copy terrain patch**. Enter the destination's minimum cell
and a signed decimal height offset, then choose **Paste terrain patch**. Save
terrain separately when satisfied. One changing paste is one terrain Undo step.

A two-cell-wide, one-cell-deep source includes six surrounding heights and two
hole flags. Paste replaces the destination heights and flags, so copying a solid
cell onto a hole restores its surface. The height offset applies to copied
heights, not the destination's previous heights. Copy takes a snapshot: overlapping
pastes and later source edits do not change its contents. Repeated pastes reuse
that original sample until another Copy succeeds or the context becomes invalid.

## Reusable assets

After Copy, enter an absolute `.json` **Terrain patch file** path and choose
**Save terrain patch asset**. It creates a new file without overwriting anything;
terrain Save is separate. In a fresh writable terrain, enter that path and choose
**Load terrain patch asset**. Load replaces only the clipboard, explicitly binding
it to the current scene and terrain. Then choose a destination, offset and Paste.
The source and target spacing must match exactly; identity, origin and dimensions
may differ. The complete pasted footprint still has to fit the destination.

Version 1 JSON stores `format: "orr_terrain_patch"`, `version: 1`, `cells`,
`spacing_raw`, `heights_raw` and `holes`. Heights/spacing are exact signed Q16 raw
integers, arrays are row-major, and heights are absolute rather than offsets.
Files contain no source paths. Unknown or duplicate fields, unsupported versions,
invalid spacing, counts or dimensions are rejected. The 256 KiB file bound and
4,096 combined-value footprint bound apply before import. Failed Load preserves
the old clipboard. Save requires a valid clipboard context but ignores pending
paste coordinates and offset. Neither action changes terrain, history or scene.

File operations currently require Unix stable file identities and trusted, idle
directories. Symlink files/ancestors are rejected; ancestry checks are not a
hostile-filesystem sandbox. A publication warning means the output exists or may
exist: inspect it instead of retrying.

## Contract

- Source and destination use cell indices. The entire footprint must fit; no
  clipping, scaling, rotation, interpolation or falloff.
  Boundary heights are shared with adjacent cells, so changing them can reshape
  neighboring surfaces. Heights outside the vertex footprint and hole flags
  outside the cell footprint remain untouched.
- The budget counts every surrounding vertex plus every cell, including values
  already equal to their destination. At most 4,096 combined entries are allowed.
- Offsets use checked fixed-point arithmetic. Invalid decimal input, overflow,
  invalid bounds, and renderer rejection leave the document unchanged.
- Copy does not edit terrain or history. An invalid Copy clears the previous
  sample. A no-op Paste preserves Undo/Redo and dirty state.
- The snapshot is pinned to its scene, scene-load epoch, terrain source, asset identity, dimensions,
  origin, and spacing. Ordinary terrain edits do not invalidate it. Play mode,
  stale scenes and read-only installed packages cannot be used to paste.
- Paste is an atomic terrain document edit. It does not modify scene YAML,
  simulation history, collision installation, or the simulation checksum.

## Acceptance

Focused CPU tests drive the actual egui Copy/Paste, Undo/Redo and Save controls,
reopen the written `.orrt`, and query its terrain sampler and static-model
consumer. An asymmetric height fixture verifies coordinate order, a fractional
offset, copied holes and restored solid cells. Additional tests cover overlapping
snapshot reuse, raw fixed-point precision and overflow, footprint limits,
portable save/load into a different terrain, malformed/file rejection,
no-op/failed-paste Redo preservation, failed-Copy invalidation, context changes,
and installed-package immutability.

These are headless authoring and consumer checks. They do not establish GPU,
window-system, mobile, browser, or live collision integration behavior.
