# Room level authoring

Build the optional `room-authoring` editor feature, or `windows-projects` for
the complete Windows Room project menu. Open **File > Room level authoring…**
in a local Room editor. This feature remains default-off.

The feature-gated Room authoring panel edits the `PLAYER`, `KEY` and `EXIT`
positions in X/Z coordinates. Use **Read Room layout** to load the current
values, then **Apply Room layout** to submit all three positions through one
editor operation and one undo entry. The host preserves each actor's Y
coordinate and all other actor fields.

The same panel can create a wall from a name, X/Z center and X/Z half-extents
with **Create Room wall**.
The new wall has a stable host-generated GUID and canonical static Body/Collider
values. Geometry bounds, player/wall overlap and the maximum wall count remain
the existing Room contract, not new or relaxed rules.
The host API remains responsible for authoritative game, mode, mutability,
geometry and document checks. The panel is disabled outside a mutable local Room
in Edit mode or while a preview or gesture is active.

Layout and wall operations acquire an owned ERP transaction, read current
authoritative actors, stage only the requested fields/entity, and run the full
existing Room scene admission on the staged document before commit. No file is
written during this validation. Invalid layouts/walls roll back as a whole;
successful edits use the normal Undo/Redo and File Save history. Read the layout
before editing; unapplied panel fields are drafts, not saved scene data.
Transport loss or malformed transaction acknowledgement blocks further edits
until reconnect and inspection; mutations are never automatically retried.

Position and extent fields use exact fixed-point decimal parsing, limited to 48
bytes per value. No floating-point conversion, simulation rule, scene schema,
HUD, or export behavior is changed by the panel. This document describes the
authoring interface and intended host contract; it does not assert that any
particular build or runtime validation has completed.
