# Pick Mesh patrol waypoints in the viewport

In MeshNavigation or MeshHazards patrol authoring, enable patrol and add the waypoint rows you need. Choose **Pick patrol waypoint N**, then click an admitted walkable mesh surface. The existing mesh hit tester projects the selected position into fixed-point XYZ. Only that row's draft changes; agent Start/Goal fields and the gameplay Frame remain untouched.

Use **Cancel patrol waypoint pick** or Escape to stop. A miss keeps the operation ready for another surface click. A source, Frame, form or row-generation change retires the pending pick. Removing and rebuilding an identical row cannot reuse the old ticket. Successful staging also retires an older whole-route inspection. Inspect the updated route, then use the existing Apply waypoint patrol and scene Save controls. Undo/Redo and saved-game runtime patrol behavior remain unchanged.

The new patrol path requires a completed current viewport frame. It defers the hit until drawing has succeeded; camera/render errors and unavailable frames cancel without changing the draft. Existing Start/Goal picking keeps its previous profile and behavior. Other authoring jobs or pointer tools block patrol arming. While a patrol operation owns input, orbit, pan, zoom, framing and entity selection are fenced.

Focus-loss events cancel even when focus is regained in the same input frame. A cancelled press stays owned through its physical release; forgetting egui pointer state after focus loss must not redirect that click into selection. After release and an idle frame, normal input resumes.

CPU acceptance uses sloped meshes in both supported profiles, actual widgets, the production picking adapter, unchanged Frame checksum/history/files and explicit Apply/save/reopen. It also checks row/source ABA, misses, fresh-frame rejection and batched focus/press/release. These tests supply an explicit completed-frame result to the adapter and do not prove GPU pixels, Windows execution, a physical game window or playback video.
