# Move an existing Room wall in the viewport

Open **Room level authoring**, expand **Manage existing Room walls**, then **Read Room walls** and select the wall. Save pending scene changes before starting viewport placement. Apply or reread any pending name, height or size edits first; placement requires those fields to match the read wall so the commit changes only X/Z.

1. Click **Place selected wall in viewport**.
2. Click the ground plane where the wall's center should be. The selected wall's X/Z fields change; its name, height, width and depth remain unchanged. This first version uses unsnapped placement (grid Off).
3. Click **Apply Room wall** to commit the staged coordinates using the existing atomic Room admission. Overlaps or out-of-bounds geometry are rejected.
4. Use normal Undo/Redo and Save. Read the walls again before another edit.

Escape or **Cancel wall placement** cancels the pending click. Changing the selected wall, scene, source state or pinned draft invalidates placement; read again when prompted. Other viewport placement tools, Play, dialogs and busy content operations cannot share the gesture.

The click alone does not mutate the scene, history or files. Apply checks both the fresh host checksum and the originally read wall inside its transaction, including its name, so an unpumped edit cannot be overwritten by stale coordinates. The existing numeric **Apply Room wall** uses the same checked path. Collision rules, models and simulation schemas are unchanged.

## Verification scope

Focused core tests cover a selected-only X/Z move, unchanged GUID/name/Y/dimensions and other entities, atomic invalid-move rollback, and unpumped physics and name-only edits. Panel tests cover current-Read requirements, selected-only staging, stale pins and invalid rays.

The software-Vulkan integration test `room_wall_placement_ui` creates and saves a wall away from the route, cancels a placement, stages a ground-plane click without authoring changes, then applies it. It checks Undo/Redo, Save/fresh reopen and live player collision, with retained screenshots. Run status and platform limits belong in the accompanying verification record; the test uses synthetic egui input, not physical OS input.
