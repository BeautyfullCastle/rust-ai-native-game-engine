# One-pass terrain smoothing

With the optional `terrain` editor feature, open a scene-owned terrain and expand
**Terrain authoring → Smooth terrain rectangle**. Enter inclusive vertex X/Z
minimum and maximum indices, inspect the changed-vertex count, then choose
**Apply one smoothing pass**. This is a single terrain Undo step; **Save terrain**
is separate. Installed package terrain stays read-only until explicitly copied
to the scene through the existing command.

The operation averages each selected vertex with its existing 3×3 neighborhood
from the same original terrain. At the terrain boundary only existing neighbors
count. FP raw heights are summed in i128 and divided with rounding toward zero,
with no floating-point conversion and no in-place traversal feedback. Each click
is one pass; it does not iterate until convergence. The full selected footprint,
including unchanged vertices, must fit the existing 4096-edit budget. Reversed,
out-of-bounds or oversized selections are rejected, not clipped.

To avoid smoothing across gaps, any hole incident to a read-neighborhood vertex
rejects the entire operation. This conservative check reaches up to two cell
indices below the selection minimum and one above the maximum, clipped to actual
cells. Thus a nearby hole outside the selected rectangle can block the pass;
far-away holes are permitted and unchanged. Outside vertices, asset identity,
origin and spacing never change. There is no falloff or promise that the selected
boundary blends smoothly into unselected terrain.

Existing scene attachment/Edit/preview/coherence guards and terrain document
read-only checks apply. All edits go through the existing transaction and render
admission before replacing the document; failures keep its bytes, model and
history. No-op does not add history. Undo/Redo and Save/Reopen use the ordinary
terrain document. This authoring operation changes terrain content, not live
simulation/scene state; it does not certify navigation clearance or passability.

Focused acceptance covers independent integer averages and rounding, bounds and
hole halos, original-source order independence, no-op/history and failed render
admission, Play/stale/read-only refusal, and actual widgets through a single
Undo/Redo and separate Save/Reopen with decoded heights and rendered model data.
No new core format, dependency, renderer, GPU or window capability is introduced.
