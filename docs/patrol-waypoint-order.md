# Reorder patrol waypoints

MeshNavigation, MeshHazards and Terrain navigation patrol drafts support inserting a waypoint before an existing row and moving a row up or down. Existing Add and Remove actions remain available.

Insert copies the selected row before it, preserving the exact coordinate text; edit the new copy or choose its position in the viewport. Move swaps adjacent rows without retyping or normalizing coordinates. Routes retain one to eight waypoints. Insertion is disabled at eight rows, and movement is disabled at either boundary.

These actions edit the draft only. The simulation Frame, scene file and undo history change only through the existing explicit Apply action and scene Save. Use Inspect to check the resulting complete route, Apply, then Save. Undo and Redo restore the applied route through the normal scene history.

Every accepted structural edit retires pending picks and the full-route preview, even when identical coordinates make the visible text unchanged or opposite moves return to the original order. Inspect again after reordering. Invalid indices and disabled boundary actions leave the draft unchanged. Controls respect editing/source/focus guards and remain unavailable while the picker owns the pointer, including its release fence. Cancel remains available.

Runtime movement uses the saved point array in its authored order. Initial approach, looping back to the first point and per-waypoint wait keep their existing meaning. Reordering can change route validity: Apply retains the existing complete-route validation and rejects an invalid candidate without partially changing the scene.

Focused automated checks cover raw coordinate preservation, bounds, equal-row identity changes, focus and picker fencing, UI edits followed by Apply/history/Save/reopening, and actual public simulation arrival order. These CPU checks do not establish Windows execution, GPU pixels or physical play coverage.