# Tile pattern Copy and Stamp

In the Tile background panel, enable **Tile tool**, choose **Copy**, and drag
between any two corners of an inclusive rectangle. Release inside the map to
capture it. The panel reports the copied width and height. Choose **Stamp** and
click the destination's top-left cell. A yellow outline previews a destination
that fits; the entire pattern must fit, with no clipping. Each actual press
applies once; holding or dragging does not repeat stamping.

Copy is read-only: it does not dirty the map, write files, or change Undo/Redo.
Stamp replaces every destination cell, including empty cells (which erase).
Region zero remains a real tile. Overlapping stamps read the immutable copied
snapshot, not the partly changed destination. One changed stamp is one tile Undo
entry; unchanged stamps preserve Redo. Copying an all-empty pattern is supported.

Patterns contain at most 4096 cells and belong only to the current panel,
project, scene load, atlas and map geometry. Ordinary painting and tile Undo/Redo
retain them. Resize, a geometry-changing Undo/Redo, reopening or replacing the
project, scene reload, or atlas replacement expires them. Returning to old
geometry does not restore an expired pattern. Copy again when no pattern exists.

Copy uses the same guarded rectangle gesture as Rectangle: Escape, focus loss,
mode/tool changes, invalid release, skipped frames and changed source cancel it.
Stamp requires an unobstructed primary press in the matching editable viewport.
Play and stale project state cannot edit. Both operations check project sources and verify installed package bytes at
Copy completion or a Stamp attempt, not continuously;
stamping validates the whole candidate against the current atlas and combined
4096 tile-plus-actor limit before committing. Refused operations preserve history.

Acceptance coverage includes asymmetric row-major patterns, all four corner
orders, overlap and repeated reuse, empty versus zero, boundaries and invalid
coordinates, maximum size, actor budgets, no-op Redo preservation, lifecycle
invalidation and actual panel input. Platform/GPU claims require their separate
recorded verification. This is not an OS clipboard, package edit, actor operation,
transform, or cross-project paste. Save tilemap remains explicit.
