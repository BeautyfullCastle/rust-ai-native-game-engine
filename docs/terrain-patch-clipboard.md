# Terrain patch clipboard

Duplicate a small authored terrain feature, including its slope and holes, without
repainting individual vertices. The clipboard belongs to the current terrain
panel; it is not an operating-system clipboard or an asset export.

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

## Contract

- Source and destination use cell indices. The entire footprint must fit; no
  clipping, scaling, rotation, interpolation, falloff, or cross-terrain transfer.
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
no-op/failed-paste Redo preservation, failed-Copy invalidation, context changes,
and installed-package immutability.

These are headless authoring and consumer checks. They do not establish GPU,
window-system, mobile, browser, or live collision integration behavior.
