# Repeating tilemap patterns

Use **Copy** to capture a motif or load a saved pattern. Optional clockwise and
mirror transforms rearrange its cells, not individual tile images. Select
**Repeat pattern**, drag an inclusive rectangle, and release inside the map.
The motif repeats from the rectangle's normalized top-left, regardless of drag
direction. Partial motif repetitions at the right and bottom edges are included.
Empty motif cells erase existing tiles; region zero is a tile, not empty.

The pattern is captured when the drag starts. Previewing does not change the map,
scene, history or files. Release applies one Undo/Redo operation. Escape, lost
focus, leaving the map on release, stale project inputs or an interrupted gesture
cancel without mutation. Save writes the ordinary tilemap sidecar; reopening and
existing player consumers need no new format or runtime feature.

Patterns retain their existing atlas, package, geometry and scene identity
checks. The complete candidate still shares the 4096 drawable-instance budget
with actors. Invalid, stale, over-budget and no-op operations preserve redo.
Installed package contents are not edited. Repeat uses the same Edit-only guards
as other brushes; Play, proposals and replay cannot author tiles.

Acceptance covers literal asymmetric patterns with empty cells, partial edges,
reverse drags, transformed/imported patterns, bounds/staleness and maximum-map
budgets; installed egui gestures exercise Save/reopen and literal runtime draw
consumption. These are Linux CPU tests, not native-window, physical input or GPU
rendering acceptance.
