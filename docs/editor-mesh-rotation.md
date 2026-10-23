# Mesh model orientation authoring

With optional `mesh-navigation`, the actual editor's **Mesh agent art** and
**Mesh props** panels expose X, Y and Z rotation fields in degrees. Rotations
apply X, then Y, then Z (`qz * qy * qx`) around the model origin before the
existing offset translation. Imported node transforms and positive uniform
instance scale remain intact. These are cosmetic model transforms, not new
walkable surfaces, physics colliders or a gameplay controller.

Load or select a model, edit its rotation, and use **Apply agent art** or
**Update selected prop**. Private panel Undo/Redo restores the orientation.
Explicitly save the art/props sidecars before Play, reopening or exporting.
Moving a prop in the viewport changes translation only and preserves rotation.
The normal scene Undo/Redo still owns authoritative route placement separately.

Angles must be finite and bounded to ±360000 degrees. Saved orientations use
normalized, canonical XYZW quaternions; inspector Euler values may change to an
equivalent representation after applying or reloading. Near gimbal poles the
orientation remains authoritative, not the individual displayed angles.

Legacy identity-only private sidecars remain byte-compatible: omitted rotation
defaults to identity and identity is not serialized. A nonidentity orientation
uses an optional rotation field in the existing private sidecar; no public
simulation schema, package format or renderer changes are required. Export
requires a trusted local player carrying the dedicated `.orrrot` PE capability
`ORR_MESH_ROTATION_V1` for rotated art or props. Older players cannot silently
ignore the saved orientation. Explicit rotation fields also require that
capability when their value is identity, since older strict decoders reject
the field itself. Existing digest and exact inventory admission
remain mandatory.

Acceptance is Windows-local and feature-scoped. The combined production App
workflow covers viewport route placement, rotated agent art and props, private
and authoritative Undo/Redo, explicit saves, source-hidden fresh Physics-editor
Open, Play and source-independent export. Relocated native execution is a
separate check. Synthetic App events are not physical OS input; implementation,
local acceptance, development integration and engine completion are distinct.
