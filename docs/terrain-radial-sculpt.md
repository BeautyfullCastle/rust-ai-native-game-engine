# Radial terrain sculpt

Build the editor with `--features terrain`, open a saved Yard3D scene, and
create or open a local terrain. Under **Terrain authoring → Radial terrain
sculpt**, enter center vertex X/Z, radius in grid intervals, and a signed center
height delta. **Use selected vertex as sculpt center** copies the existing
vertex selection into the draft. **Apply radial sculpt** raises a hill or lowers
a depression in one terrain Undo step. Typing and preview do not edit terrain.
Repeated Apply clicks deliberately accumulate separate stamps. Save is explicit;
Undo/Redo and saved `.orrt` files retain the ordinary terrain contract.

The GPU-free `orr_terrain::sculpt::radial_edits` API also supports scripted game
world authoring. Pass an immutable terrain, center `[x, z]`, radius and fixed-point
delta; apply its returned batch through `TerrainDocument::apply`. The editor uses
this same function, then admits the complete mesh and query before committing.

The footprint is a disk centered on a terrain vertex. Radius is 1–128 grid
intervals; multiply by terrain spacing for world distance. The height delta is
scaled by `1 - squared_distance / squared_radius`, a parabolic falloff reaching
zero at the boundary. Each signed raw fixed-point delta is rounded once toward
zero. Positive and negative stamps are exact inverses unless a range limit
rejects the operation. Tiny deltas can quantize to zero away from the center.
The disk is clipped at terrain edges, with no wrap or vertex movement. Existing
height detail remains under the added shape. Holes and metadata stay unchanged.

A footprint can include at most 4096 vertices strictly inside the disk, counting
zero-delta vertices. Invalid center/radius, excess budget, any height overflow,
or editor render admission failure rejects the whole operation. No-op and
rejected edits preserve redo, saved state and revision. Play mode, stale scenes
and installed package terrain retain existing write restrictions.

Acceptance covers literal radial heights and edge clipping, signed rounding,
limits and overflow, actual editor Apply/Undo/Redo/Save controls, reload of saved
bytes through surface queries and the mesh bridge, read-only packages and stale
scene/Play rejection. This is an explicit numeric stamp, not a drag brush or
continuous stroke. It does not install collision or change the host scene. No
new renderer, dependencies, asset format or simulation fields are introduced.
