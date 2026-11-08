# Expand a terrain grid

Open a scene-owned local terrain in Edit mode and expand **Expand terrain grid**. Choose target width and depth and a fill height, then apply. Expansion grows toward +X and +Z with the original origin and vertex spacing. Existing vertex heights and cell holes keep their coordinates; new vertices use the fill height and new cells are solid. The old outer border becomes interior, so exact border sampling follows the existing positive-axis cell ownership rule.

Each side remains between 2 and 129 vertices. Shrinking is rejected. Equal dimensions are a no-op and keep Undo/Redo state. Invalid heights, coordinate overflow, read-only package terrain, stale attachments and Play mode reject the operation without changing the admitted terrain.

Expansion is one terrain Undo step. Undo restores the old shape and clamps selection to it; Redo restores the expansion. Save is explicit. Rebuild navigation after saving to use the expanded area; an existing admitted graph is not silently replaced. The current navigation builder retains its 17 by 17 vertex limit; larger terrain can be authored and saved but cannot be built through that route. Large new height differences can create steep boundary triangles, so choose the fill height and navigation settings accordingly.

Focused CPU checks cover preserved data, history, controls, Save/reopen and explicit navigation consumption. Windows runtime, GPU rendering, physical play and video remain unverified.
