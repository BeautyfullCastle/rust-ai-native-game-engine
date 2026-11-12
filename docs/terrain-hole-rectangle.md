# Terrain hole rectangles

Enable the optional editor `terrain` feature and open a local Yard3D authoring
scene. In **Terrain authoring**, create or open editable terrain, then expand
**Terrain hole rectangle**. Set an inclusive minimum/maximum X/Z rectangle in
**cell** coordinates. A 5×5 vertex heightfield has cells 0 through 3 on each axis.

Check **Carve holes (unchecked restores surface)** to remove the selected cells
from the terrain surface. Uncheck it to restore their original heights. Review
the changed-cell count and click **Apply hole rectangle**. Only changed cells
are included in one terrain Undo step. Repeating the same action does nothing
and preserves redo. Terrain Undo/Redo is separate from scene history.

Use **Save terrain** to persist the terrain asset; reopen reads that saved asset.
The existing terrain sampler and renderer consume its holes: carved cells have
no surface samples or triangles. Restoring cells recovers their existing heights,
not a flat replacement. An entirely carved terrain remains attached but has no
fake render geometry. Cell-edge sample ownership is unchanged.

Rectangles must be ordered and entirely in bounds. The complete footprint is
limited to 4096 cells even when some or all cells are already in the requested
state. Invalid rectangles are rejected, not clipped. Heights and cells outside
the rectangle are unchanged. Existing local Edit-mode, scene identity, immutable
package, render admission, save/source-pin and undo limits remain in force.
This does not add runtime terrain editing, a collision mode, an export format,
or an external package override. Linux headless/widget and saved-consumer checks
are not native-window, physical GPU, or Windows acceptance.
