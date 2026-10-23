# Draw a Room wall footprint

In a saved, clean local Room, open **Room level authoring**, enter a wall name,
and choose **Draw Room wall footprint**. Click two opposite corners in the 3D
viewport. Both points are projected onto the ground plane (Y=0), using the camera
that successfully rendered that viewport. This is not arbitrary mesh-surface
picking.

The two clicks only stage the existing wall X/Z center and half-extents fields.
The wall name and existing fixed vertical dimensions remain unchanged. Review
those values and press **Create Room wall** to submit the existing authoritative
wall transaction with a fresh GUID. Standard Undo, Redo and Save then apply.

Escape, lost focus, dialogs, playback, dirty/context changes and active content
jobs cancel an unfinished footprint. Pointer ownership is kept through the final
or cancelled frame, preventing the same click from selecting or moving a scene
entity. No scene, history, model or camera file changes until Create.

The software-Vulkan acceptance test uses the production editor with synthetic
pointer and accessibility events. It checks staging, selection-preserving Escape,
fresh wall creation, Undo/Redo/Save/reopen and player collision against the new
wall. It does not claim physical OS input or hardware-GPU coverage.

## Optional grid snapping

**Wall footprint grid** defaults to **Off**. Choose **0.25 units**, **0.5 units**,
**1 unit**, or **2 units** to snap each ground corner before calculating the wall
center and half-extents. Exact half-step ties round away from zero, including
negative coordinates. This affects only viewport drawing; it does not rewrite
existing walls or manually entered fields.

Changing the grid cancels an unfinished two-click gesture and preserves the
previous form fields. Draw again under the new setting. Corners that snap to the
same X or Z coordinate are refused rather than creating a zero-width wall.

Separate software UI cases retain the original Off lifecycle and exercise
negative/positive off-grid corners, exact snapped values, mid-gesture grid changes,
then the same Create/Undo/Redo/Save/reopen and gameplay collision checks.
