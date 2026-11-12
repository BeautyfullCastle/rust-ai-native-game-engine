# Surface-height prop grids

The optional Mesh Navigation prop editor can stamp an applied static prop across
an admitted walkable mesh. This is a presentation-authoring operation, not a
navigation, collision or simulation edit. It uses the existing `mesh-navigation`
feature and adds no runtime format or dependency.

## Author a grid

Select an applied prop, finish or discard pending prop/prefab/material edits, and
open **Prop grid**. Set columns, rows and positive X/Z spacing. The existing
**Stamp selected prop grid** still produces the original flat grid without change.

For **Stamp surface-height prop grid**, set a maximum vertical distance and an
explicit signed height offset. Cell `[0,0]` is the original prop and remains
unchanged. Every other cell queries the current admitted mesh at its grid X/Z,
using the same original prop-pivot Y as the height reference. The nearest surface
within the vertical distance wins; equal distances use the graph's lowest
triangle key. The signed height offset is added *after* surface selection.

This is not a connected-floor search, normal alignment, mesh-bottom snapping or
a collision-free placement guarantee. In overlapping floors, the closest height
may belong to another disconnected surface. Rotation, scale, art, material
settings and optional scalar shading are copied unchanged. Choose the height
offset appropriate for the model's pivot; it is not inferred from model bounds.

All new props are validated before a single presentation-history entry is
committed. One missing surface or invalid cell rejects the whole action. Undo and
Redo affect the complete stamp; **Save mesh props** is still explicit. A saved
sidecar remains self-contained for a fresh editor without the source package.

## Numeric and work limits

Only generated samples are quantized. Existing finite f32 source coordinates are
widened to f64, multiplied by 65,536 and rounded to nearest Q16, with exact ties
away from zero. The original prop is never rewritten to that rounded anchor.
Spacing, vertical distance and height offset use the existing fixed-point decimal
parser with a 48-byte input limit. Spacing is positive and at most 1,024; vertical
distance is 0–1,000,000; the signed offset is within ±1,000,000. Grid cell arithmetic
and height addition are checked before conversion or mutation.

The usual 1–16 rows/columns and 128-prop scene limits apply. Before projection,
`(cells - 1) × admitted triangle count` must be at most 1,048,576. This is a bounded
work rule, not a measured timing guarantee. Existing coordinate, art, appearance,
draw and sidecar budgets still apply. Final presentation coordinates use ordinary
FP-to-f32 conversion and may round; exact mathematical contact beyond f32
presentation precision is not promised. Values are rejected rather than clamped.

The operation rechecks current scene/epoch/graph and editing guards before its
one history commit. Play, stale context, active gestures and unapplied drafts
cannot silently change the result. There is no background projection or live
surface-following relationship after stamping.

[Unity's surface-snapping documentation](https://docs.unity.com/en-us/engine/6000.7/manual/working-with-scenes/scenes-manage-gameobjects/positioning-game-objects)
provides workflow context only; this tool is not a Unity format or behavior
compatibility layer. Offscreen editor acceptance does not establish native OS
window input, Windows or physical-GPU support.
