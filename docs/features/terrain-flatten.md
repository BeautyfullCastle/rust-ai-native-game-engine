# Terrain rectangle flattening

The optional `terrain` editor feature can set an inclusive rectangular range of
vertices to one fixed-point height, useful for a building pad or level platform.
It is not a brush, ramp, hole fill, collision installer, or slope-safety check.

In a coherent local Yard3D/navigation Edit scene, create/open terrain, expand
**Flatten terrain rectangle**, enter minimum/maximum X/Z vertex indices and the
**Flatten target height**. The panel shows valid indices, the 4096-vertex budget,
and the number of vertices that would change. Press **Apply flatten rectangle**.
The entire edit is one terrain Undo/Redo step; **Save terrain** remains explicit.
A rectangle with no changed heights adds no history entry.

Both endpoints are inclusive and must be ordered and in bounds. The full
rectangle, including unchanged vertices, must contain at most 4096 vertices.
Invalid decimals, reversed bounds and oversized/outside rectangles are rejected,
never clipped or split. The existing integer-only decimal parser is reused.
There is no edge falloff: adjacent cells may slope toward the new boundary.
Hole flags remain unchanged, even where their corner heights change. Identity,
origin, spacing and vertices outside the rectangle remain unchanged.

The existing scene association, Edit/Play, preview, coherent-row, package
read-only and render-admission guards apply. Installed package terrain must first
be explicitly copied to a scene file. Applying changes neither scene commands nor
files; Save uses the existing terrain document path rules. No retained preview
can authorize a later edit: Apply rechecks the current document and context.

Implementation is confined to the terrain panel and its helper; core format,
renderer and runtime are unchanged. Focused acceptance covers actual panel
controls, single-step Undo/Redo, Save/Reopen, preserved holes/outside values,
invalid and blocked-context atomicity, and no-op history. This does not certify
GPU rendering, physical traversal or gameplay suitability.
