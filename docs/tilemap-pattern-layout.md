# Tilemap copied-pattern layout transforms

In an Arena tilemap project, enable **Tile tool**, choose **Copy**, and drag a
rectangle in the viewport. The copied snapshot includes empty cells, which erase
cells when stamped. Before stamping, use **Rotate layout clockwise**, **Mirror
layout horizontally**, or **Mirror layout vertically**. Clockwise turns swap the
rectangle's width and height. Four turns or two identical mirrors restore the
original layout.

These controls rearrange cells, not the pixels inside each tile. Tile artwork
stays upright. They do not rotate the map geometry, add collision, or modify the
simulation. For directional artwork, choose an appropriate atlas region instead.

Choose **Stamp** and click its top-left destination. The entire transformed
rectangle must fit inside the map; it is never clipped. Each successful changed
stamp is one **Undo tile** operation. **Redo tile** restores it. **Save tilemap**
persists ordinary tile cells, so fresh project opens, Play and existing tilemap
export/player rendering need no new format or runtime support.

Transforming alone does not change document history, saved files, or simulation
state. The copy remains a panel-local snapshot, not an operating-system clipboard.
Existing source, atlas, scene and geometry checks still apply. Reopen/reload or
geometry changes expire the copy; copy again. Finish or cancel an active painting
or rectangle gesture before transforming. No transform is available in Play.

Acceptance covers literal asymmetric rectangular/empty-cell permutations,
four-turn/double-mirror identities, transformed bounds, stale scene/atlas refusal,
and actual Editor controls through Stamp, Undo/Redo, Save, fresh reopen and Play.
Rendered acceptance uses Linux offscreen software GPU evidence; it does not imply
native-window or physical-input validation.
